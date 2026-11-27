use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, RenderContent, SoftMask, TransformExt,
    font::{Glyph, GlyphRun},
    hayro_cmap::BfString,
    hayro_syntax::Pdf,
    interpret_page,
    kurbo::{BezPath, Point, Rect, Vec2},
};
use std::{
    fs::File,
    io::{self, Read},
    ops::Range,
    path::Path,
};

const FILE_LIMIT: u64 = 128 * 1024 * 1024;
const GLYPH_LIMIT: usize = 30_000;

#[derive(Clone, Copy, Debug)]
pub struct TextBounds {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

struct GlyphText {
    bounds: TextBounds,
    text: String,
    separator: Option<char>,
    vertical: bool,
    forward: bool,
}

#[derive(Default)]
pub struct TextPage {
    glyphs: Vec<GlyphText>,
}

impl TextPage {
    pub fn len(&self) -> usize {
        self.glyphs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    pub fn caret(&self, x: f32, y: f32, allow_outside: bool) -> Option<usize> {
        let point = |glyph: &GlyphText| {
            let bounds = glyph.bounds;
            let dx = (bounds.left - x).max(x - bounds.right).max(0.);
            let dy = (bounds.top - y).max(y - bounds.bottom).max(0.);
            dx * dx + dy * dy
        };
        let (index, nearest) = self
            .glyphs
            .iter()
            .enumerate()
            .filter(|(_, glyph)| allow_outside || point(glyph) == 0.)
            .min_by(|(_, left), (_, right)| point(left).total_cmp(&point(right)))?;
        let bounds = nearest.bounds;
        let past_midpoint = if nearest.vertical {
            y >= (bounds.top + bounds.bottom) / 2.
        } else {
            x >= (bounds.left + bounds.right) / 2.
        };
        Some(index + usize::from(past_midpoint == nearest.forward))
    }

    pub fn bounds(&self, range: Range<usize>) -> impl Iterator<Item = TextBounds> + '_ {
        self.glyphs[bounded_range(range, self.len())]
            .iter()
            .map(|glyph| glyph.bounds)
    }

    pub fn text(&self, range: Range<usize>) -> String {
        let mut text = String::new();
        for glyph in &self.glyphs[bounded_range(range, self.len())] {
            if let Some(separator) = glyph.separator
                && !text.is_empty()
                && (separator == '\n' || !text.ends_with(char::is_whitespace))
                && !glyph.text.starts_with(char::is_whitespace)
            {
                text.push(separator);
            }
            text.push_str(&glyph.text);
        }
        text
    }
}

fn bounded_range(range: Range<usize>, limit: usize) -> Range<usize> {
    let end = range.end.min(limit);
    range.start.min(end)..end
}

pub fn read_page_text(path: &Path, index: u32) -> io::Result<TextPage> {
    let file = File::open(path)?;
    if file.metadata()?.len() > FILE_LIMIT {
        return Err(invalid_pdf());
    }
    let mut bytes = Vec::new();
    file.take(FILE_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err(invalid_pdf());
    }
    std::panic::catch_unwind(move || extract(bytes, index)).unwrap_or_else(|_| Err(invalid_pdf()))
}

fn extract(bytes: Vec<u8>, index: u32) -> io::Result<TextPage> {
    let pdf = Pdf::new(bytes).map_err(|_| invalid_pdf())?;
    let page = pdf.pages().get(index as usize).ok_or_else(invalid_pdf)?;
    let (width, height) = page.render_dimensions();
    if width <= 0. || height <= 0. || !width.is_finite() || !height.is_finite() {
        return Err(invalid_pdf());
    }
    let (width, height) = (f64::from(width), f64::from(height));
    let cache = InterpreterCache::new();
    let settings = InterpreterSettings {
        render_content: RenderContent::PAGE,
        ..InterpreterSettings::default()
    };
    let mut context = Context::new(
        page.initial_transform(true).to_kurbo(),
        Rect::new(0., 0., width, height),
        &cache,
        pdf.xref(),
        settings,
    );
    let mut extractor = Extractor {
        page_width: width,
        page_height: height,
        result: TextPage::default(),
        previous: None,
    };
    interpret_page(page, &mut context, &mut extractor);
    Ok(extractor.result)
}

fn invalid_pdf() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Could not read text from the PDF",
    )
}

struct GlyphPosition {
    origin: Point,
    advance: Vec2,
    em: f64,
}

struct Extractor {
    page_width: f64,
    page_height: f64,
    result: TextPage,
    previous: Option<GlyphPosition>,
}

impl Device<'_> for Extractor {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'_>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    fn draw_image(&mut self, _: Image<'_, '_>, _: ImageDrawProps<'_>) {}

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, '_>, props: DrawProps<'_>, _: &DrawMode) {
        for glyph in run.glyphs() {
            if self.result.len() >= GLYPH_LIMIT {
                break;
            }
            let mut text = match glyph.as_unicode() {
                Some(BfString::Char(character)) => character.to_string(),
                Some(BfString::String(text)) => text,
                None => "�".into(),
            };
            if let Some((end, _)) = text.char_indices().nth(32) {
                text.truncate(end);
            }
            if text.is_empty() {
                continue;
            }
            let advance = match &**glyph {
                Glyph::Outline(glyph) => glyph.advance_width(),
                Glyph::Type3(glyph) => glyph.advance_width(),
            };
            let advance = f64::from(advance.unwrap_or(500.)).clamp(100., 4000.);
            let transform = props.transform * glyph.transform();
            let origin = transform * Point::ZERO;
            let direction = transform * Point::new(advance, 0.) - origin;
            let em = (transform * Point::new(0., 1000.) - origin).hypot();
            if !em.is_finite() || em == 0. || !direction.hypot().is_finite() {
                continue;
            }
            let bounds = transform.transform_rect_bbox(Rect::new(0., -200., advance, 850.));
            let bounds = TextBounds {
                left: normalized(bounds.x0 / self.page_width),
                right: normalized(bounds.x1 / self.page_width),
                top: normalized(bounds.y0 / self.page_height),
                bottom: normalized(bounds.y1 / self.page_height),
            };
            if !(bounds.left < bounds.right && bounds.top < bounds.bottom) {
                continue;
            }
            let separator = self.previous.as_ref().and_then(|previous| {
                let offset = origin - previous.origin;
                let length = previous.advance.hypot();
                if length == 0. {
                    return None;
                }
                let direction = previous.advance / length;
                let along_line = offset.dot(direction);
                let across_line = offset.cross(direction).abs();
                let em = em.max(previous.em);
                if across_line > em * 0.6 || along_line < -em * 0.5 {
                    Some('\n')
                } else if along_line - length > em * 0.16 {
                    Some(' ')
                } else {
                    None
                }
            });
            let vertical = direction.y.abs() > direction.x.abs();
            let forward = if vertical {
                direction.y >= 0.
            } else {
                direction.x >= 0.
            };
            self.result.glyphs.push(GlyphText {
                bounds,
                text,
                separator,
                vertical,
                forward,
            });
            self.previous = Some(GlyphPosition {
                origin,
                advance: direction,
                em,
            });
        }
    }
}

fn normalized(value: f64) -> f32 {
    value.clamp(0., 1.) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn extracts_text_and_geometry_from_a_compiled_page() {
        let resources = "<< /Font << /F1 5 0 R >> >>";
        let stream = "BT /F1 12 Tf 20 150 Td (Text selection) Tj 0 -20 Td (Second line) Tj ET\n";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources {resources} /Contents 4 0 R >>"
            ),
            format!("<< /Length {} >>\nstream\n{stream}endstream", stream.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        ];
        let mut bytes = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.into_iter().enumerate() {
            offsets.push(bytes.len());
            writeln!(bytes, "{} 0 obj\n{object}\nendobj", index + 1).unwrap();
        }
        let xref = bytes.len();
        writeln!(bytes, "xref\n0 6\n0000000000 65535 f ").unwrap();
        for offset in offsets {
            writeln!(bytes, "{offset:010} 00000 n ").unwrap();
        }
        writeln!(
            bytes,
            "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF"
        )
        .unwrap();

        let page = extract(bytes, 0).unwrap();
        assert_eq!(page.text(0..page.len()), "Text selection\nSecond line");
        let first = page.bounds(0..1).next().unwrap();
        assert!(first.left >= 0.09 && first.left <= 0.11);
        assert!(first.top >= 0.15 && first.bottom <= 0.30);
        let y = (first.top + first.bottom) / 2.;
        assert_eq!(page.caret(first.left + 0.0001, y, false), Some(0));
        assert_eq!(page.caret(0.99, 0.99, false), None);
    }
}
