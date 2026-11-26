use super::{PageRequest, RasterizedPage, io};
use crate::asset_image;
use refract_native::{
    file_preview::ImageKind,
    pdf_raster::{PageSize, page_layout},
};
use std::{path::Path, sync::Arc};
use windows::{
    Data::Pdf::{PdfDocument, PdfPageRenderOptions},
    Graphics::Imaging::BitmapEncoder,
    Storage::{
        StorageFile,
        Streams::{DataReader, DataWriter, InMemoryRandomAccessStream},
    },
    Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
    core::HSTRING,
};

pub(super) fn render_page(path: &Path, request: PageRequest) -> io::Result<RasterizedPage> {
    render(path, request)
        .map_err(|error| io::Error::other(format!("Couldn’t render PDF page. {error}")))
}

fn render(path: &Path, request: PageRequest) -> Result<RasterizedPage, Box<dyn std::error::Error>> {
    let _apartment = Apartment::initialize()?;
    render_document(&open_file(path)?, request)
}

fn open_file(path: &Path) -> windows::core::Result<PdfDocument> {
    let path = HSTRING::from(dunce::simplified(path).as_os_str());
    let file = StorageFile::GetFileFromPathAsync(&path)?.get()?;
    PdfDocument::LoadFromFileAsync(&file)?.get()
}

pub(super) fn read_page_sizes(path: &Path) -> io::Result<Vec<PageSize>> {
    let read = || -> windows::core::Result<Vec<PageSize>> {
        let _apartment = Apartment::initialize()?;
        let pdf = open_file(path)?;
        (0..pdf.PageCount()?)
            .map(|index| {
                let size = pdf.GetPage(index)?.Size()?;
                Ok(PageSize {
                    width: f64::from(size.Width) * 72. / 96.,
                    height: f64::from(size.Height) * 72. / 96.,
                })
            })
            .collect()
    };
    read().map_err(io::Error::other)
}

pub(crate) fn render_asset_page(bytes: Arc<Vec<u8>>, index: u32) -> io::Result<RasterizedPage> {
    render_asset(&bytes, index)
        .map_err(|error| io::Error::other(format!("Couldn’t render PDF page. {error}")))
}

fn render_asset(bytes: &[u8], index: u32) -> Result<RasterizedPage, Box<dyn std::error::Error>> {
    let _apartment = Apartment::initialize()?;
    let stream = InMemoryRandomAccessStream::new()?;
    let writer = DataWriter::CreateDataWriter(&stream.GetOutputStreamAt(0)?)?;
    writer.WriteBytes(bytes)?;
    if writer.StoreAsync()?.get()? as usize != bytes.len() {
        return Err("The input PDF stream is incomplete.".into());
    }
    writer.DetachStream()?;
    stream.Seek(0)?;
    let document = PdfDocument::LoadFromStreamAsync(&stream)?.get()?;
    render_document(&document, PageRequest::project_file(index))
}

fn render_document(
    document: &PdfDocument,
    request: PageRequest,
) -> Result<RasterizedPage, Box<dyn std::error::Error>> {
    let count = document.PageCount()?;
    if count == 0 {
        return Err("The PDF has no pages.".into());
    }
    let current = request.index.min(count - 1);
    let page = document.GetPage(current)?;
    let size = page.Size()?;
    let point_width = f64::from(size.Width) * 72. / 96.;
    let point_height = f64::from(size.Height) * 72. / 96.;
    let layout = page_layout(point_width, point_height, request)?;
    let pixels_wide = layout.pixel_width;
    let pixels_high = layout.pixel_height;
    let target = InMemoryRandomAccessStream::new()?;
    let settings = PdfPageRenderOptions::new()?;
    settings.SetDestinationWidth(pixels_wide)?;
    settings.SetDestinationHeight(pixels_high)?;
    settings.SetBitmapEncoderId(BitmapEncoder::PngEncoderId()?)?;
    page.RenderWithOptionsToStreamAsync(&target, &settings)?
        .get()?;

    let bytes = target.Size()?;
    if bytes > u64::from(pixels_wide) * u64::from(pixels_high) * 4 + 1024 * 1024 {
        return Err("The rendered PDF page is too large.".into());
    }
    let stream = target.GetInputStreamAt(0)?;
    let data = DataReader::CreateDataReader(&stream)?;
    if u64::from(data.LoadAsync(bytes as u32)?.get()?) != bytes {
        return Err("The PDF page image is incomplete.".into());
    }
    let mut png = vec![0; bytes as usize];
    data.ReadBytes(&mut png)?;
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("The PDF page did not render as a PNG.".into());
    }
    Ok(RasterizedPage {
        current,
        count,
        width: layout.width,
        height: layout.height,
        image: asset_image::decode(png, ImageKind::Png)?,
    })
}

struct Apartment;

impl Apartment {
    fn initialize() -> windows::core::Result<Self> {
        // Rendering runs on a background thread. Balance each successful initialization.
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }?;
        Ok(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // The apartment guard stays on the thread that initialized it.
        unsafe { RoUninitialize() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Write};

    #[test]
    fn renders_and_fits_a_page_without_opening_a_window() {
        let profile = tempfile::tempdir().unwrap();
        let path = profile.path().join("fixture.pdf");
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let stream = "0.2 0.4 0.7 rg\n10 30 170 100 re f\n";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << >> /Contents 5 0 R >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Resources << >> /Contents 5 0 R >>".into(),
            format!("<< /Length {} >>\nstream\n{stream}endstream", stream.len()),
        ];
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            writeln!(pdf, "{} 0 obj\n{object}\nendobj", index + 1).unwrap();
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
        fs::write(&path, &pdf).unwrap();
        let sizes = read_page_sizes(&path).unwrap();
        assert_eq!(sizes.len(), 2);
        for (size, (width, height)) in sizes.iter().zip([(300., 200.), (200., 300.)]) {
            assert!((size.width - width).abs() < 0.01);
            assert!((size.height - height).abs() < 0.01);
        }
        let frame = render_page(
            &path,
            PageRequest {
                index: 5,
                width: Some(320.),
                height: Some(300.),
                display_scale: 1.,
                pixel_budget: refract_native::pdf_raster::MAX_RASTER_PIXELS,
            },
        )
        .unwrap();
        assert_eq!(frame.current, 1);
        assert_eq!(frame.count, 2);
        assert!((frame.width - 200.).abs() < 0.01);
        assert!((frame.height - 300.).abs() < 0.01);
        assert_eq!(frame.image.as_bytes(0).unwrap().len(), 250 * 375 * 4);
        let asset = render_asset_page(Arc::new(pdf), 1).unwrap();
        assert_eq!((asset.current, asset.count), (1, 2));
        assert!((asset.width - 200.).abs() < 0.01);
        assert_eq!(asset.image.as_bytes(0).unwrap().len(), 300 * 450 * 4);
    }
}
