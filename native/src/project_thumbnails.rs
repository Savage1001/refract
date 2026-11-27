use super::{Library, Palette, asset_image, pdf_view::raster_pdf_bytes};
use base64::{Engine, engine::general_purpose::STANDARD};
use gpui::{Context, ObjectFit, RenderImage, Task, Window, div, img, prelude::*, px, rgb};
use image::{ImageEncoder, ImageFormat, ImageReader, Limits, codecs::png::PngEncoder};
use refract_native::{Project, pdf_raster::PageRequest, tex::compiled_pdf_for_thumbnail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{self, Cursor, Read, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const MAX_ENTRIES: usize = 60;
const CACHE_BYTES: usize = 2_000_000;
const RASTER_LIMIT: u32 = 320;

#[derive(Deserialize, Serialize)]
struct ThumbnailRecord {
    id: String,
    digest: [u8; 32],
    png: String,
}

#[derive(Clone)]
struct Thumbnail {
    record: Arc<ThumbnailRecord>,
    image: Arc<RenderImage>,
}

#[derive(Default)]
pub(super) struct ProjectThumbnails {
    entries: HashMap<String, Thumbnail>,
    released: Vec<Arc<RenderImage>>,
    loaded: bool,
    scan: Option<Scan>,
}

struct Scan {
    cancel: Arc<AtomicBool>,
    _task: Task<()>,
}

impl Drop for Scan {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl ProjectThumbnails {
    pub(super) fn cancel(&mut self) {
        self.scan = None;
    }

    pub(super) fn release_replaced(&mut self, window: &mut Window) {
        for image in self.released.drain(..) {
            let _ = window.drop_image(image);
        }
    }

    fn set_entries(&mut self, entries: HashMap<String, Thumbnail>) {
        let previous = std::mem::replace(&mut self.entries, entries);
        for (id, thumbnail) in previous {
            if !self
                .entries
                .get(&id)
                .is_some_and(|current| Arc::ptr_eq(&current.image, &thumbnail.image))
            {
                self.released.push(thumbnail.image);
            }
        }
        self.loaded = true;
    }

    pub(super) fn image(&self, id: &str, grid: bool, palette: Palette) -> Option<impl IntoElement> {
        let thumbnail = self.entries.get(id)?;
        let (width, height) = if grid { (44., 60.) } else { (38., 48.) };
        let dimensions = thumbnail.image.size(0);
        let aspect = dimensions.height.0 as f32 / dimensions.width.0 as f32;
        let content_height = ((width - 2.) * aspect).min(height - 2.);
        Some(
            div()
                .flex_none()
                .w(px(width))
                .h(px(height))
                .rounded(px(3.))
                .border_1()
                .border_color(rgb(palette.border))
                .bg(rgb(0xffffff))
                .overflow_hidden()
                .child(
                    img(thumbnail.image.clone())
                        .w_full()
                        .h(px(content_height))
                        .object_fit(ObjectFit::Contain),
                ),
        )
    }
}

impl Library {
    pub(super) fn refresh_project_thumbnails(&mut self, cx: &mut Context<Self>) {
        self.thumbnails.cancel();
        if self.selection.is_some() {
            return;
        }
        let Some(Ok(projects)) = &self.projects else {
            return;
        };
        let projects = projects.clone();
        let root = self.projects_dir.clone();
        let path = root.with_file_name(".refract-native-thumbnails.json");
        let existing = self
            .thumbnails
            .loaded
            .then(|| self.thumbnails.entries.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        let executor = cx.background_executor().clone();
        let guard = cancel.clone();
        let task = cx.spawn(async move |library, cx| {
            let existing = match existing {
                Some(existing) => existing,
                None => {
                    let cache_path = path.clone();
                    let cached = executor.spawn(async move { load_cache(&cache_path) }).await;
                    if guard.load(Ordering::Relaxed) {
                        return;
                    }
                    let _ = library.update(cx, |library, cx| {
                        library.thumbnails.set_entries(cached.clone());
                        cx.notify();
                    });
                    cached
                }
            };
            let worker_cancel = guard.clone();
            let entries = executor
                .spawn(async move {
                    let entries =
                        load_project_thumbnails(&root, &projects, &existing, &worker_cancel);
                    if !worker_cancel.load(Ordering::Relaxed) && cache_changed(&existing, &entries)
                    {
                        let _ = save_cache(&path, &projects, &entries);
                    }
                    entries
                })
                .await;
            if !guard.load(Ordering::Relaxed) {
                let _ = library.update(cx, |library, cx| {
                    library.thumbnails.set_entries(entries);
                    library.thumbnails.scan = None;
                    cx.notify();
                });
            }
        });
        self.thumbnails.scan = Some(Scan {
            cancel,
            _task: task,
        });
    }
}

fn load_project_thumbnails(
    root: &Path,
    projects: &[Project],
    existing: &HashMap<String, Thumbnail>,
    cancelled: &AtomicBool,
) -> HashMap<String, Thumbnail> {
    let mut entries = HashMap::new();
    for project in projects {
        if entries.len() == MAX_ENTRIES || cancelled.load(Ordering::Relaxed) {
            break;
        }
        let cached = existing.get(&project.id);
        if let Some(thumbnail) =
            load_thumbnail(root, project, cached).unwrap_or_else(|_| cached.cloned())
        {
            entries.insert(project.id.clone(), thumbnail);
        }
    }
    entries
}

fn load_thumbnail(
    root: &Path,
    project: &Project,
    existing: Option<&Thumbnail>,
) -> io::Result<Option<Thumbnail>> {
    let Some(pdf) = compiled_pdf_for_thumbnail(root, project)? else {
        return Ok(None);
    };
    let digest: [u8; 32] = Sha256::digest(&pdf).into();
    if let Some(existing) = existing.filter(|entry| entry.record.digest == digest) {
        return Ok(Some(existing.clone()));
    }
    let page = raster_pdf_bytes(
        Arc::new(pdf),
        PageRequest {
            index: 0,
            width: Some(180.),
            height: Some(256.),
            pixel_budget: 80_000,
            display_scale: 1.,
        },
    )?;
    let image = page.image;
    let size = image.size(0);
    let mut rgba = image
        .as_bytes(0)
        .ok_or_else(|| io::Error::other("Empty PDF thumbnail"))?
        .to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            &rgba,
            size.width.into(),
            size.height.into(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(io::Error::other)?;
    Ok(Some(Thumbnail {
        record: Arc::new(ThumbnailRecord {
            id: project.id.clone(),
            digest,
            png: STANDARD.encode(&png),
        }),
        image,
    }))
}

fn load_cache(path: &Path) -> HashMap<String, Thumbnail> {
    let records: Vec<ThumbnailRecord> = File::open(path)
        .ok()
        .and_then(|file| serde_json::from_reader(file.take(CACHE_BYTES as u64)).ok())
        .unwrap_or_default();
    records
        .into_iter()
        .take(MAX_ENTRIES)
        .filter_map(|record| {
            let png = STANDARD.decode(&record.png).ok()?;
            let mut reader = ImageReader::with_format(Cursor::new(png), ImageFormat::Png);
            let mut limits = Limits::default();
            limits.max_image_width = Some(RASTER_LIMIT);
            limits.max_image_height = Some(RASTER_LIMIT);
            limits.max_alloc = Some(u64::from(RASTER_LIMIT) * u64::from(RASTER_LIMIT) * 4);
            reader.limits(limits);
            let image = asset_image::frame_image(reader.decode().ok()?.into_rgba8());
            Some((
                record.id.clone(),
                Thumbnail {
                    record: Arc::new(record),
                    image,
                },
            ))
        })
        .collect()
}

fn cache_changed(
    previous: &HashMap<String, Thumbnail>,
    current: &HashMap<String, Thumbnail>,
) -> bool {
    previous.len() != current.len()
        || current.iter().any(|(id, value)| {
            previous
                .get(id)
                .is_none_or(|previous| previous.record.digest != value.record.digest)
        })
}

fn save_cache(
    path: &Path,
    projects: &[Project],
    entries: &HashMap<String, Thumbnail>,
) -> io::Result<()> {
    let mut bytes = vec![b'['];
    for entry in projects
        .iter()
        .filter_map(|project| entries.get(&project.id))
    {
        let record = serde_json::to_vec(entry.record.as_ref())?;
        if bytes.len() + record.len() + 2 > CACHE_BYTES {
            break;
        }
        if bytes.len() > 1 {
            bytes.push(b',');
        }
        bytes.extend(record);
    }
    bytes.push(b']');
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut stage = tempfile::NamedTempFile::new_in(parent)?;
    stage.write_all(&bytes)?;
    stage.flush()?;
    stage.persist(path).map(|_| ()).map_err(|error| error.error)
}
