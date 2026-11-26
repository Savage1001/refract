use super::{
    Library, Palette, asset_image, pdf_view::render_asset_page, workspace_tools::tool_button,
};
use gpui::{Context, ObjectFit, RenderImage, Window, div, img, prelude::*, px, rgb};
use refract_native::{
    Project,
    file_preview::{ImageKind, Preview, inspect_file},
};
use std::{io, path::Path, sync::Arc};

pub(super) enum LoadedFile {
    Text(String),
    Asset(AssetPreview),
}

pub(super) enum AssetPreview {
    Image(Arc<RenderImage>),
    Pdf(ProjectPdf),
    Message(String),
}

pub(super) struct ProjectPdf {
    bytes: Arc<[u8]>,
    frame: PdfFrame,
    pending: Option<u32>,
    error: Option<String>,
}

struct PdfFrame {
    page: u32,
    count: u32,
    image: Arc<RenderImage>,
}

pub(super) fn load_file(
    projects_dir: &Path,
    project: &Project,
    file: &str,
) -> io::Result<LoadedFile> {
    let asset = match inspect_file(projects_dir, project, file)? {
        Preview::Text(text) => return Ok(LoadedFile::Text(text)),
        Preview::Unsupported(reason) => AssetPreview::Message(reason.into()),
        Preview::Image(kind, bytes) => match asset_image::decode(bytes, kind) {
            Ok(image) => AssetPreview::Image(image),
            Err(_) => AssetPreview::Message("Could not preview this image.".into()),
        },
        Preview::Pdf(bytes) => match decode_pdf(&bytes, 0) {
            Ok(frame) => AssetPreview::Pdf(ProjectPdf {
                bytes: bytes.into(),
                frame,
                pending: None,
                error: None,
            }),
            Err(error) => AssetPreview::Message(format!("Could not preview PDF: {error}")),
        },
    };
    Ok(LoadedFile::Asset(asset))
}

fn decode_pdf(bytes: &[u8], index: u32) -> io::Result<PdfFrame> {
    let page = render_asset_page(bytes, index)?;
    Ok(PdfFrame {
        page: page.current,
        count: page.count,
        image: asset_image::decode(page.png, ImageKind::Png)?,
    })
}

impl AssetPreview {
    pub(super) fn release(self, window: &mut Window) {
        match self {
            Self::Image(image) => {
                let _ = window.drop_image(image);
            }
            Self::Pdf(pdf) => {
                let _ = window.drop_image(pdf.frame.image);
            }
            Self::Message(_) => {}
        }
    }
}

impl Library {
    fn project_pdf_step(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(AssetPreview::Pdf(pdf)) = self
            .selection
            .as_mut()
            .and_then(|selection| selection.asset_preview.as_mut())
        else {
            return;
        };
        let target = pdf
            .frame
            .page
            .saturating_add_signed(delta)
            .min(pdf.frame.count - 1);
        if pdf.pending.is_some() || pdf.frame.page == target {
            return;
        }
        let bytes = pdf.bytes.clone();
        let request_bytes = bytes.clone();
        pdf.pending = Some(target);
        pdf.error = None;
        let load = cx
            .background_executor()
            .spawn(async move { decode_pdf(&request_bytes, target) });
        self.preview_task = Some(cx.spawn_in(window, async move |view, cx| {
            let page = load.await;
            let _ = view.update_in(cx, |view, window, cx| {
                if let Some(AssetPreview::Pdf(pdf)) = view
                    .selection
                    .as_mut()
                    .and_then(|selection| selection.asset_preview.as_mut())
                    && Arc::ptr_eq(&pdf.bytes, &bytes)
                {
                    pdf.pending = None;
                    match page {
                        Ok(frame) => {
                            let previous = std::mem::replace(&mut pdf.frame, frame);
                            let _ = window.drop_image(previous.image);
                        }
                        Err(error) => {
                            pdf.error = Some(format!("Could not render this page: {error}"))
                        }
                    }
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    pub(super) fn asset_preview(
        &self,
        asset: &AssetPreview,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let panel = div()
            .id("file-preview")
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .p_6()
            .overflow_y_scroll();
        match asset {
            AssetPreview::Message(message) => panel.child(message.clone()),
            AssetPreview::Image(image) => panel
                .debug_selector(|| "project-image".into())
                .flex()
                .items_center()
                .justify_center()
                .child(
                    img(image.clone())
                        .id("project-image")
                        .object_fit(ObjectFit::ScaleDown)
                        .max_w_full(),
                ),
            AssetPreview::Pdf(pdf) => {
                let visible_page = pdf.pending.unwrap_or(pdf.frame.page) + 1;
                let count = pdf.frame.count;
                let controls = div()
                    .flex()
                    .justify_center()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .child(page_button(
                        "project-pdf-previous",
                        "Previous",
                        pdf.pending.is_none() && visible_page > 1,
                        -1,
                        palette,
                        cx,
                    ))
                    .child(format!("Page {visible_page} of {count}"))
                    .child(page_button(
                        "project-pdf-next",
                        "Next",
                        pdf.pending.is_none() && visible_page < count,
                        1,
                        palette,
                        cx,
                    ));
                panel
                    .debug_selector(|| "project-pdf".into())
                    .flex()
                    .flex_col()
                    .child(controls)
                    .child(if pdf.pending.is_some() {
                        div()
                            .child(format!("Loading page {visible_page}…"))
                            .into_any_element()
                    } else if let Some(error) = &pdf.error {
                        div().child(error.clone()).into_any_element()
                    } else {
                        div()
                            .flex()
                            .justify_center()
                            .my_3()
                            .child(
                                img(pdf.frame.image.clone())
                                    .id("project-pdf-page")
                                    .max_w_full()
                                    .object_fit(ObjectFit::ScaleDown),
                            )
                            .into_any_element()
                    })
            }
        }
    }
}

fn page_button(
    id: &'static str,
    label: &'static str,
    enabled: bool,
    delta: i32,
    palette: Palette,
    cx: &Context<Library>,
) -> impl IntoElement {
    if enabled {
        tool_button(id, label, false, palette, cx, move |view, window, cx| {
            view.project_pdf_step(delta, window, cx)
        })
        .into_any_element()
    } else {
        div()
            .px_1()
            .py_1()
            .text_color(rgb(palette.muted))
            .child(label)
            .into_any_element()
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::{Root, initialize_app};
    use gpui::TestAppContext;
    use refract_native::create_project;
    use std::fs;

    #[gpui::test]
    fn previews_assets_in_file_tabs_and_restores_the_text_editor(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Assets").unwrap();
        let folder = projects.join(&project.id);
        fs::write(folder.join("main.tex"), "Text source\n").unwrap();
        fs::write(folder.join("diagram.svg"), br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="red"/></svg>"#).unwrap();
        fs::write(folder.join("broken.png"), "invalid image").unwrap();
        fs::write(folder.join("data.bin"), "printable data").unwrap();
        cx.update(initialize_app);
        let mut view = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let library = cx.new(|cx| Library::new(projects, window, cx));
            view = Some(library.clone());
            Root::new(library, window, cx)
        });
        let library = view.unwrap();
        cx.update(|window, app| {
            library.update(app, |view, cx| view.open_project(project, window, cx));
        });
        cx.run_until_parked();
        let initial = cx.update(|_, app| library.read(app).active_editor().unwrap());
        cx.update(|window, app| {
            library.update(app, |view, cx| {
                view.open_file("diagram.svg".into(), window, cx)
            });
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        assert!(cx.debug_bounds("project-image").is_some());
        cx.update(|_, app| {
            let view = library.read(app);
            assert!(view.active_editor().is_none());
            let Some(AssetPreview::Image(image)) = &view.selection.as_ref().unwrap().asset_preview
            else {
                panic!("image preview");
            };
            assert_eq!(image.as_bytes(0).unwrap()[..4], [0, 0, 255, 255]);
        });
        for (file, expected) in [
            ("broken.png", "Could not preview this image."),
            ("data.bin", "This file cannot be previewed as text."),
        ] {
            cx.update(|window, app| {
                library.update(app, |view, cx| view.open_file(file.into(), window, cx));
            });
            cx.run_until_parked();
            cx.update(|_, app| {
                let Some(AssetPreview::Message(message)) =
                    &library.read(app).selection.as_ref().unwrap().asset_preview
                else {
                    panic!("preview message");
                };
                assert_eq!(message, expected);
            });
        }
        cx.update(|window, app| {
            library.update(app, |view, cx| {
                view.open_file("main.tex".into(), window, cx)
            });
            assert_eq!(
                library.read(app).active_editor().unwrap().entity_id(),
                initial.entity_id()
            );
        });
    }
}
