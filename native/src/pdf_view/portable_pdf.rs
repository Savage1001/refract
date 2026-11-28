use super::RasterizedPage;
use crate::asset_image;
use image::RgbaImage;
use refract_native::{
    pdf_raster::{PageRequest, portable},
    tex::RetainedPdf,
};
use std::{io, sync::Arc};

pub(super) use portable::read_retained_page_sizes as read_page_sizes;

pub(crate) fn render_asset_page(bytes: Arc<Vec<u8>>, index: u32) -> io::Result<RasterizedPage> {
    raster_pdf_bytes(bytes, PageRequest::project_file(index))
}

pub(crate) fn raster_pdf_bytes(
    bytes: Arc<Vec<u8>>,
    request: PageRequest,
) -> io::Result<RasterizedPage> {
    build_frame(portable::render_bytes(bytes, request)?)
}

pub(super) fn raster_retained_pdf(
    document: &RetainedPdf,
    request: PageRequest,
) -> io::Result<RasterizedPage> {
    build_frame(portable::render_retained(document, request)?)
}

fn build_frame(page: portable::Page) -> io::Result<RasterizedPage> {
    let layout = page.layout;
    let buffer = RgbaImage::from_raw(layout.pixel_width, layout.pixel_height, page.rgba)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "The PDF page bitmap is incomplete.",
            )
        })?;
    Ok(RasterizedPage {
        current: page.current,
        count: page.count,
        width: layout.width,
        height: layout.height,
        image: asset_image::frame_image(buffer),
    })
}
