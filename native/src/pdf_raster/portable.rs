use super::{PageLayout, PageRequest, PageSize, page_layout};
use hayro::{
    RenderCache, RenderSettings,
    hayro_interpret::{InterpreterSettings, util::TransformExt},
    hayro_syntax::Pdf,
    kurbo::Affine,
    render_into,
    vello_cpu::{
        Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit,
        color::palette::css::WHITE, peniko::ImageAlphaType,
    },
};
use std::{fs, io, path::Path, sync::Arc};

pub struct Page {
    pub current: u32,
    pub count: u32,
    pub point_width: f64,
    pub point_height: f64,
    pub layout: PageLayout,
    pub rgba: Vec<u8>,
}

pub fn render_file(path: &Path, request: PageRequest) -> io::Result<Page> {
    render_bytes(Arc::new(fs::read(path)?), request)
}

pub fn read_page_sizes(path: &Path) -> io::Result<Vec<PageSize>> {
    read_page_sizes_from_bytes(Arc::new(fs::read(path)?))
}

pub fn read_page_sizes_from_bytes(bytes: Arc<Vec<u8>>) -> io::Result<Vec<PageSize>> {
    std::panic::catch_unwind(move || read_sizes(bytes)).unwrap_or_else(|_| {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Couldn’t open PDF.",
        ))
    })
}

fn read_sizes(bytes: Arc<Vec<u8>>) -> io::Result<Vec<PageSize>> {
    let pdf = open_pdf(bytes)?;
    Ok(pdf
        .pages()
        .iter()
        .map(|page| {
            let (width, height) = page.render_dimensions();
            PageSize {
                width: f64::from(width),
                height: f64::from(height),
            }
        })
        .collect())
}

pub fn render_bytes(bytes: Arc<Vec<u8>>, request: PageRequest) -> io::Result<Page> {
    // A malformed document should fail its preview on the worker instead of interrupting the app.
    std::panic::catch_unwind(move || render(bytes, request)).unwrap_or_else(|_| {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Couldn’t render this PDF page.",
        ))
    })
}

fn open_pdf(bytes: Arc<Vec<u8>>) -> io::Result<Pdf> {
    Pdf::new(bytes).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Couldn’t open PDF. {error:?}"),
        )
    })
}

fn render(bytes: Arc<Vec<u8>>, request: PageRequest) -> io::Result<Page> {
    let pdf = open_pdf(bytes)?;
    let pages = pdf.pages();
    let count = u32::try_from(pages.len()).map_err(io::Error::other)?;
    if count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The PDF has no pages.",
        ));
    }
    let current = request.index.min(count - 1);
    let page = &pages[current as usize];
    let (point_width, point_height) = page.render_dimensions();
    let (point_width, point_height) = (f64::from(point_width), f64::from(point_height));
    let layout = page_layout(point_width, point_height, request)?;
    let width = layout.pixel_width as u16;
    let height = layout.pixel_height as u16;
    let transform = Affine::scale_non_uniform(
        f64::from(width) / point_width,
        f64::from(height) / point_height,
    ) * page.initial_transform(true).to_kurbo();
    let mut context = RenderContext::new(width, height);
    render_into(
        page,
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &mut context,
        transform,
    );
    context.flush();
    let mut pixmap = Pixmap::new(width, height);
    context.render_with(
        &mut pixmap,
        &mut Resources::default(),
        RasterizerSettings {
            target_init: TargetInit::Clear(WHITE),
            ..Default::default()
        },
    );
    Ok(Page {
        current,
        count,
        point_width,
        point_height,
        layout,
        rgba: pixmap.take_rgba8(ImageAlphaType::Alpha),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture() -> Arc<Vec<u8>> {
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let content = "1 0 0 rg\n10 20 100 190 re f\n";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] /Resources << >> /Contents 5 0 R >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 300] /CropBox [10 20 110 210] /Rotate 90 /Resources << >> /Contents 5 0 R >>".into(),
            format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
        ];
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            writeln!(pdf, "{} 0 obj\n{object}\nendobj", i + 1).unwrap();
        }
        let xref = pdf.len();
        write!(pdf, "xref\n0 6\n0000000000 65535 f \n").unwrap();
        for offset in offsets {
            writeln!(pdf, "{offset:010} 00000 n ").unwrap();
        }
        writeln!(
            pdf,
            "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF"
        )
        .unwrap();
        Arc::new(pdf)
    }

    #[test]
    fn renders_the_requested_page_with_crop_and_rotation() {
        let pdf = fixture();
        let file = tempfile::NamedTempFile::new().unwrap();
        fs::write(file.path(), pdf.as_slice()).unwrap();
        let sizes = read_page_sizes(file.path()).unwrap();
        assert_eq!(sizes.len(), 2);
        assert_eq!((sizes[0].width, sizes[0].height), (400., 200.));
        assert_eq!((sizes[1].width, sizes[1].height), (190., 100.));
        let first = render_bytes(pdf.clone(), PageRequest::project_file(0)).unwrap();
        assert_eq!((first.current, first.count), (0, 2));
        assert_eq!((first.point_width, first.point_height), (400., 200.));
        assert_eq!(&first.rgba[..4], &[255, 255, 255, 255]);
        let last = render_bytes(pdf, PageRequest::project_file(99)).unwrap();
        assert_eq!((last.current, last.count), (1, 2));
        assert_eq!((last.point_width, last.point_height), (190., 100.));
        let (width, height) = (last.layout.pixel_width, last.layout.pixel_height);
        let center = ((height / 2 * width + width / 2) * 4) as usize;
        assert_eq!(&last.rgba[center..center + 4], &[255, 0, 0, 255]);
        assert!(
            render_bytes(
                Arc::new(b"not a PDF".to_vec()),
                PageRequest::project_file(0)
            )
            .is_err()
        );
    }
}
