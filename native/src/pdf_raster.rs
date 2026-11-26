use std::io;

const MAX_RASTER_DIMENSION: f64 = 4096.;
const MAX_RASTER_PIXELS: f64 = 8_000_000.;

#[derive(Clone, Copy)]
pub struct PageRequest {
    pub index: u32,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub display_scale: f32,
}

impl PageRequest {
    pub fn project_file(index: u32) -> Self {
        Self {
            index,
            width: None,
            height: None,
            display_scale: 1.2,
        }
    }
}

pub struct PageLayout {
    pub width: f32,
    pub height: f32,
    pub pixel_width: u32,
    pub pixel_height: u32,
}

/// Fit the logical preview and cap the pixels allocated to a single page.
pub fn page_layout(
    point_width: f64,
    point_height: f64,
    request: PageRequest,
) -> io::Result<PageLayout> {
    let positive = |value: f64| value.is_finite() && value > 0.;
    if !positive(point_width) || !positive(point_height) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The PDF page dimensions are invalid.",
        ));
    }
    let aspect = point_height / point_width;
    let requested_width = request.width.map_or(point_width, f64::from);
    let width = request.height.map_or(requested_width, |height| {
        requested_width.min(f64::from(height) / aspect)
    });
    let height = width * aspect;
    if !positive(f64::from(width as f32))
        || !positive(f64::from(height as f32))
        || !request.display_scale.is_finite()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The PDF page could not be fitted in the preview.",
        ));
    }
    let quality = f64::from(request.display_scale.clamp(1., 4.)) * 1.25;
    let scale = quality
        .min((MAX_RASTER_PIXELS / (width * height)).sqrt())
        .min(MAX_RASTER_DIMENSION / width.max(height));
    Ok(PageLayout {
        width: width as f32,
        height: height as f32,
        pixel_width: (width * scale).floor().max(1.) as u32,
        pixel_height: (height * scale).floor().max(1.) as u32,
    })
}

#[cfg(all(
    feature = "portable-pdf",
    any(target_os = "linux", target_os = "macos")
))]
pub mod portable;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_a_portrait_page_inside_the_requested_viewport() {
        let request = PageRequest {
            index: 0,
            width: Some(320.),
            height: Some(300.),
            display_scale: 1.,
        };
        let layout = page_layout(200., 300., request).unwrap();
        assert_eq!((layout.width, layout.height), (200., 300.));
        assert_eq!((layout.pixel_width, layout.pixel_height), (250, 375));
    }

    #[test]
    fn limits_the_bitmap_even_when_the_pdf_or_display_is_large() {
        for (width, height) in [(5000., 3000.), (12_000., 200.), (50_000., 50_000.)] {
            let layout = page_layout(width, height, PageRequest::project_file(0)).unwrap();
            assert!(layout.pixel_width <= 4096 && layout.pixel_height <= 4096);
            assert!(layout.pixel_width * layout.pixel_height <= 8_000_000);
        }
        assert!(page_layout(0., 800., PageRequest::project_file(0)).is_err());
    }
}
