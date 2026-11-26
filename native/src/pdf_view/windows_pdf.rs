use super::{PageRequest, RasterizedPage, io};
use std::path::Path;
use windows::{
    Data::Pdf::{PdfDocument, PdfPageRenderOptions},
    Graphics::Imaging::BitmapEncoder,
    Storage::{
        StorageFile,
        Streams::{DataReader, InMemoryRandomAccessStream},
    },
    Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
    core::HSTRING,
};

const MAX_PIXELS: f32 = 8_000_000.;

pub(super) fn render_page(path: &Path, request: PageRequest) -> io::Result<RasterizedPage> {
    render(path, request)
        .map_err(|error| io::Error::other(format!("Couldn’t render PDF page. {error}")))
}

fn render(path: &Path, request: PageRequest) -> Result<RasterizedPage, Box<dyn std::error::Error>> {
    let _apartment = Apartment::initialize()?;
    let path = HSTRING::from(dunce::simplified(path).as_os_str());
    let file = StorageFile::GetFileFromPathAsync(&path)?.get()?;
    let document = PdfDocument::LoadFromFileAsync(&file)?.get()?;
    let count = document.PageCount()?;
    if count == 0 {
        return Err("The PDF has no pages.".into());
    }
    let current = request.index.min(count - 1);
    let page = document.GetPage(current)?;
    let size = page.Size()?;
    if !size.Width.is_finite() || !size.Height.is_finite() || size.Width <= 0. || size.Height <= 0.
    {
        return Err("The PDF page dimensions are invalid.".into());
    }
    let aspect = size.Height / size.Width;
    let width = request
        .height
        .map_or(request.width, |height| request.width.min(height / aspect));
    let height = width * aspect;
    if !width.is_finite() || !height.is_finite() || width <= 0. || height <= 0. {
        return Err("The PDF page could not be fitted in the preview.".into());
    }
    let quality = request.display_scale.clamp(1., 4.) * 1.25;
    let scale = quality
        .min((MAX_PIXELS / (width * height)).sqrt())
        .min(4096. / width.max(height));
    let pixels_wide = (width * scale).round().max(1.) as u32;
    let pixels_high = (height * scale).round().max(1.) as u32;
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
        width,
        height,
        point_width: f64::from(size.Width) * 72. / 96.,
        point_height: f64::from(size.Height) * 72. / 96.,
        png,
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
        fs::write(&path, pdf).unwrap();
        let frame = render_page(
            &path,
            PageRequest {
                index: 5,
                width: 320.,
                height: Some(300.),
                display_scale: 1.,
            },
        )
        .unwrap();
        assert_eq!(frame.current, 1);
        assert_eq!(frame.count, 2);
        assert!((frame.width - 200.).abs() < 0.01);
        assert!((frame.height - 300.).abs() < 0.01);
        assert!(frame.png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}
