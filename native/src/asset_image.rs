use gpui::RenderImage;
use image::{
    AnimationDecoder, Frame, Frames, ImageDecoder, ImageFormat, ImageReader, Limits, RgbaImage,
    codecs::{gif::GifDecoder, png::PngDecoder, webp::WebPDecoder},
};
use refract_native::file_preview::ImageKind;
use std::{
    io::{self, Cursor},
    sync::{Arc, LazyLock},
};

const MAX_IMAGE_SIDE: u32 = 8192;
const MAX_PIXEL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ANIMATION_FRAMES: usize = 2000;

pub(super) fn decode(bytes: Vec<u8>, kind: ImageKind) -> io::Result<Arc<RenderImage>> {
    let cursor = Cursor::new(bytes.as_slice());
    let format = match kind {
        ImageKind::Svg => return render_svg(&bytes),
        ImageKind::Gif => {
            let mut gif = GifDecoder::new(cursor).map_err(io::Error::other)?;
            check_dimensions(gif.dimensions())?;
            gif.set_limits(limits()).map_err(io::Error::other)?;
            return animation(gif.into_frames());
        }
        ImageKind::WebP => {
            let mut webp = WebPDecoder::new(cursor).map_err(io::Error::other)?;
            check_dimensions(webp.dimensions())?;
            webp.set_limits(limits()).map_err(io::Error::other)?;
            if webp.has_animation() {
                return animation(webp.into_frames());
            }
            return decode_still(webp);
        }
        ImageKind::Png => {
            let mut png = PngDecoder::new(cursor).map_err(io::Error::other)?;
            check_dimensions(png.dimensions())?;
            png.set_limits(limits()).map_err(io::Error::other)?;
            if png.is_apng().map_err(io::Error::other)? {
                let png = png.apng().map_err(io::Error::other)?;
                return animation(png.into_frames());
            }
            return decode_still(png);
        }
        ImageKind::Bmp => ImageFormat::Bmp,
        ImageKind::Jpeg => ImageFormat::Jpeg,
    };
    let mut reader = ImageReader::with_format(cursor, format);
    reader.limits(limits());
    decode_still(reader.into_decoder().map_err(io::Error::other)?)
}

fn decode_still(mut decoder: impl ImageDecoder) -> io::Result<Arc<RenderImage>> {
    check_dimensions(decoder.dimensions())?;
    let orientation = decoder.orientation().map_err(io::Error::other)?;
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(io::Error::other)?;
    image.apply_orientation(orientation);
    Ok(frame_image(image.into_rgba8()))
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(MAX_PIXEL_BYTES);
    limits
}

fn check_dimensions((width, height): (u32, u32)) -> io::Result<u64> {
    let bytes = u64::from(width.min(MAX_IMAGE_SIDE)) * u64::from(height.min(MAX_IMAGE_SIDE)) * 4;
    if width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE || bytes == 0 || bytes > MAX_PIXEL_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Image dimensions exceed the preview limit",
        ));
    }
    Ok(bytes)
}

fn animation(frames: Frames<'_>) -> io::Result<Arc<RenderImage>> {
    let mut decoded = Vec::new();
    let mut total = 0;
    for frame in frames {
        let mut frame = frame.map_err(io::Error::other)?;
        total += check_dimensions(frame.buffer().dimensions())?;
        if total > MAX_PIXEL_BYTES || decoded.len() == MAX_ANIMATION_FRAMES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Image animation exceeds the preview limit",
            ));
        }
        to_render_order(frame.buffer_mut());
        decoded.push(frame);
    }
    if decoded.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Image contains no frames",
        ));
    }
    Ok(Arc::new(RenderImage::new(decoded)))
}

pub(super) fn frame_image(mut buffer: RgbaImage) -> Arc<RenderImage> {
    to_render_order(&mut buffer);
    Arc::new(RenderImage::new([Frame::new(buffer)]))
}

fn to_render_order(buffer: &mut RgbaImage) {
    for pixel in buffer.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
}

fn render_svg(bytes: &[u8]) -> io::Result<Arc<RenderImage>> {
    use resvg::usvg;

    static FONTS: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
        let mut fonts = usvg::fontdb::Database::new();
        fonts.load_system_fonts();
        Arc::new(fonts)
    });
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_string: Box::new(|_, _| None),
            ..Default::default()
        },
        fontdb: FONTS.clone(),
        ..Default::default()
    };
    let tree = usvg::Tree::from_data(bytes, &options).map_err(io::Error::other)?;
    render_svg_tree(
        &tree,
        1.0_f32.min(4096. / tree.size().width().max(tree.size().height())),
    )
}

pub(super) fn render_svg_icon(bytes: &[u8], pixels: f32) -> io::Result<Arc<RenderImage>> {
    let tree =
        resvg::usvg::Tree::from_data(bytes, &Default::default()).map_err(io::Error::other)?;
    render_svg_tree(
        &tree,
        pixels / tree.size().width().max(tree.size().height()),
    )
}

fn render_svg_tree(tree: &resvg::usvg::Tree, scale: f32) -> io::Result<Arc<RenderImage>> {
    use resvg::tiny_skia;

    let width = tree.size().width();
    let height = tree.size().height();
    let mut pixmap = tiny_skia::Pixmap::new(
        (width * scale).ceil() as u32,
        (height * scale).ceil() as u32,
    )
    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid SVG dimensions"))?;
    resvg::render(
        tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let rgba: Vec<_> = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let pixel = pixel.demultiply();
            [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]
        })
        .collect();
    let buffer = RgbaImage::from_raw(pixmap.width(), pixmap.height(), rgba)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid SVG bitmap"))?;
    Ok(frame_image(buffer))
}
