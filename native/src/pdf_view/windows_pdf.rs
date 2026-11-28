use super::{PageRequest, RasterizedPage, io};
use crate::asset_image;
use refract_native::{
    file_preview::ImageKind,
    pdf_raster::{PageSize, page_layout},
};
use std::{sync::Arc, thread};
use windows::{
    Data::Pdf::{PdfDocument, PdfPageRenderOptions},
    Graphics::Imaging::BitmapEncoder,
    Storage::Streams::{DataReader, DataWriter, InMemoryRandomAccessStream},
    Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
    core::HRESULT,
};

const RPC_E_CHANGED_MODE: HRESULT = HRESULT(0x8001_0106u32 as i32);

pub(super) fn read_page_sizes(bytes: Arc<Vec<u8>>) -> io::Result<Vec<PageSize>> {
    with_mta(|| {
        let read = || -> windows::core::Result<Vec<PageSize>> {
            let (_stream, pdf) = open_stream(&bytes)?;
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
    })
}

pub(crate) fn render_asset_page(bytes: Arc<Vec<u8>>, index: u32) -> io::Result<RasterizedPage> {
    raster_pdf_bytes(bytes, PageRequest::project_file(index))
}

pub(crate) fn raster_pdf_bytes(
    bytes: Arc<Vec<u8>>,
    request: PageRequest,
) -> io::Result<RasterizedPage> {
    with_mta(|| {
        let (_stream, document) = open_stream(&bytes).map_err(io::Error::other)?;
        render_document(&document, request)
            .map_err(|error| io::Error::other(format!("Couldn’t render PDF page. {error}")))
    })
}

fn open_stream(bytes: &[u8]) -> windows::core::Result<(InMemoryRandomAccessStream, PdfDocument)> {
    let stream = InMemoryRandomAccessStream::new()?;
    let writer = DataWriter::CreateDataWriter(&stream.GetOutputStreamAt(0)?)?;
    writer.WriteBytes(bytes)?;
    if writer.StoreAsync()?.get()? as usize != bytes.len() {
        return Err(windows::core::Error::new(
            HRESULT(0x80070026_u32 as i32),
            "The input PDF stream is incomplete.",
        ));
    }
    writer.DetachStream()?;
    stream.Seek(0)?;
    let document = PdfDocument::LoadFromStreamAsync(&stream)?.get()?;
    Ok((stream, document))
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

fn with_mta<T: Send>(operation: impl FnOnce() -> io::Result<T> + Send) -> io::Result<T> {
    match Apartment::initialize() {
        Ok(_apartment) => operation(),
        // WinRT's synchronous waits require an MTA. Leave an existing apartment intact.
        Err(error) if error.code() == RPC_E_CHANGED_MODE => thread::scope(|scope| {
            thread::Builder::new()
                .name("refract-pdf".into())
                .spawn_scoped(scope, move || {
                    let _apartment = Apartment::initialize().map_err(io::Error::other)?;
                    operation()
                })?
                .join()
                .map_err(|_| io::Error::other("The Windows PDF worker stopped unexpectedly."))?
        }),
        Err(error) => Err(io::Error::other(error)),
    }
}

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
    use std::io::Write;

    #[test]
    fn renders_and_fits_a_page_without_opening_a_window() {
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
        let pdf = Arc::new(pdf);
        let sizes = read_page_sizes(pdf.clone()).unwrap();
        assert_eq!(sizes.len(), 2);
        for (size, (width, height)) in sizes.iter().zip([(300., 200.), (200., 300.)]) {
            assert!((size.width - width).abs() < 0.01);
            assert!((size.height - height).abs() < 0.01);
        }
        let frame = raster_pdf_bytes(
            pdf.clone(),
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
        let asset = render_asset_page(pdf, 1).unwrap();
        assert_eq!((asset.current, asset.count), (1, 2));
        assert!((asset.width - 200.).abs() < 0.01);
        assert_eq!(asset.image.as_bytes(0).unwrap().len(), 300 * 450 * 4);
    }
}
