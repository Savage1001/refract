use crate::tex::RetainedPdf;
use hayro_syntax::{
    Pdf, PdfData,
    object::{Array, Dict, Name, Object, Rect, String as PdfString},
    page::Page,
};
use std::{
    fs::File,
    io::{self, Read},
    panic::AssertUnwindSafe,
    path::Path,
    sync::Arc,
};
use url::Url;

const MAX_PDF_BYTES: u64 = 128 * 1024 * 1024;
const MAX_PAGE_LINKS: usize = 256;
const MAX_DOCUMENT_LINKS: usize = 16_384;
const MAX_DESTINATION_NODES: usize = 4096;
const MAX_URL_BYTES: usize = 8192;

#[derive(Clone, Debug, PartialEq)]
pub struct PdfLink {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub target: LinkTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LinkTarget {
    Page(u32),
    External(String),
}

pub fn read_document_links(path: &Path) -> io::Result<Vec<Vec<PdfLink>>> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_PDF_BYTES {
        return Err(invalid_pdf());
    }
    let mut bytes = Vec::new();
    file.take(MAX_PDF_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PDF_BYTES {
        return Err(invalid_pdf());
    }
    document_links_from_bytes(Arc::new(bytes))
}

pub fn document_links_from_bytes(bytes: Arc<Vec<u8>>) -> io::Result<Vec<Vec<PdfLink>>> {
    if bytes.len() as u64 > MAX_PDF_BYTES {
        return Err(invalid_pdf());
    }
    document_links_from_data(bytes.into())
}

pub fn retained_document_links(document: &RetainedPdf) -> io::Result<Vec<Vec<PdfLink>>> {
    document_links_from_data(document.pdf_data()?)
}

fn document_links_from_data(data: PdfData) -> io::Result<Vec<Vec<PdfLink>>> {
    // The backing data is immutable and each call owns its parser state.
    std::panic::catch_unwind(AssertUnwindSafe(move || parse_document_links(data)))
        .unwrap_or_else(|_| Err(invalid_pdf()))
}

fn invalid_pdf() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Could not read links from the PDF",
    )
}

fn parse_document_links(data: PdfData) -> io::Result<Vec<Vec<PdfLink>>> {
    let pdf = Pdf::new(data).map_err(|_| invalid_pdf())?;
    let catalog = pdf.xref().get::<Dict<'_>>(pdf.xref().root_id());
    let base = catalog.as_ref().and_then(|dict| {
        let uri: Dict<'_> = dict.get("URI")?;
        let base: PdfString<'_> = uri.get("Base")?;
        external_url(base.as_bytes(), None)
    });
    let mut remaining = MAX_DOCUMENT_LINKS;
    Ok(pdf
        .pages()
        .iter()
        .map(|page| {
            let count = remaining.min(MAX_PAGE_LINKS);
            if count == 0 {
                return Vec::new();
            }
            let Some(annotations) = page.raw().get::<Array<'_>>("Annots") else {
                return Vec::new();
            };
            let links: Vec<_> = annotations
                .iter::<Dict<'_>>()
                .take(MAX_PAGE_LINKS)
                .filter_map(|annotation| {
                    read_link(&pdf, catalog.as_ref(), base.as_ref(), page, annotation)
                })
                .take(count)
                .collect();
            remaining -= links.len();
            links
        })
        .collect())
}

fn read_link(
    pdf: &Pdf,
    catalog: Option<&Dict<'_>>,
    base: Option<&Url>,
    page: &Page<'_>,
    annotation: Dict<'_>,
) -> Option<PdfLink> {
    if annotation.get::<Name<'_>>("Subtype")?.as_str() != "Link" {
        return None;
    }
    if annotation
        .get::<u32>("F")
        .is_some_and(|flags| flags & 0b100010 != 0)
    {
        return None;
    }
    let rect: Rect = annotation.get("Rect")?;
    let [left, top, right, bottom] = viewport_rect(page, rect)?;
    let target = if let Some(action) = annotation.get::<Dict<'_>>("A") {
        match action.get::<Name<'_>>("S")?.as_str() {
            "URI" => LinkTarget::External(
                external_url(action.get::<PdfString<'_>>("URI")?.as_bytes(), base)?.into(),
            ),
            "GoTo" => LinkTarget::Page(destination_page(pdf, catalog, action.get("D")?)?),
            _ => return None,
        }
    } else {
        LinkTarget::Page(destination_page(pdf, catalog, annotation.get("Dest")?)?)
    };
    Some(PdfLink {
        left,
        top,
        right,
        bottom,
        target,
    })
}

fn viewport_rect(page: &Page<'_>, rect: Rect) -> Option<[f32; 4]> {
    let (width, height) = page.render_dimensions();
    if width <= 0. || height <= 0. || !width.is_finite() || !height.is_finite() {
        return None;
    }
    let [a, b, c, d, e, f] = page.initial_transform(true).as_coeffs();
    let mut left = f64::INFINITY;
    let mut top = f64::INFINITY;
    let mut right = f64::NEG_INFINITY;
    let mut bottom = f64::NEG_INFINITY;
    for (x, y) in [
        (rect.x0, rect.y0),
        (rect.x0, rect.y1),
        (rect.x1, rect.y0),
        (rect.x1, rect.y1),
    ] {
        let mapped_x = (a * x + c * y + e) / f64::from(width);
        let mapped_y = (b * x + d * y + f) / f64::from(height);
        if !mapped_x.is_finite() || !mapped_y.is_finite() {
            return None;
        }
        left = left.min(mapped_x);
        top = top.min(mapped_y);
        right = right.max(mapped_x);
        bottom = bottom.max(mapped_y);
    }
    let [left, top, right, bottom] =
        [left, top, right, bottom].map(|value| value.clamp(0., 1.) as f32);
    (left < right && top < bottom).then_some([left, top, right, bottom])
}

fn external_url(raw_url: &[u8], base: Option<&Url>) -> Option<Url> {
    if raw_url.is_empty()
        || raw_url.len() > MAX_URL_BYTES
        || raw_url.iter().any(u8::is_ascii_control)
    {
        return None;
    }
    let text = std::str::from_utf8(raw_url).ok()?;
    let url = Url::parse(text).ok().or_else(|| base?.join(text).ok())?;
    (matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.as_str().len() <= MAX_URL_BYTES
        && url.username().is_empty()
        && url.password().is_none())
    .then_some(url)
}

fn destination_page(pdf: &Pdf, catalog: Option<&Dict<'_>>, destination: Object<'_>) -> Option<u32> {
    let values = match destination {
        Object::Array(values) => values,
        Object::Name(name) => lookup_destination(catalog?, name.as_str().as_bytes())?,
        Object::String(name) => lookup_destination(catalog?, name.as_bytes())?,
        _ => return None,
    };
    let page = match values.flex_iter().next::<Object<'_>>()? {
        Object::Dict(page) => {
            let id = page.obj_id()?;
            pdf.pages()
                .iter()
                .position(|page| page.raw().obj_id() == Some(id))?
        }
        Object::Number(number) => {
            let index = number.as_f64();
            if !index.is_finite() || index < 0. || index.fract() != 0. {
                return None;
            }
            index as usize
        }
        _ => return None,
    };
    (page < pdf.pages().len())
        .then(|| u32::try_from(page).ok())
        .flatten()
}

fn lookup_destination<'a>(catalog: &Dict<'a>, name: &[u8]) -> Option<Array<'a>> {
    if name.is_empty() || name.len() > MAX_URL_BYTES {
        return None;
    }
    if let Some(destinations) = catalog.get::<Dict<'_>>("Dests")
        && let Some(value) = destinations.get::<Object<'_>>(name)
    {
        return destination_array(value);
    }
    let names = catalog.get::<Dict<'_>>("Names")?.get::<Dict<'_>>("Dests")?;
    let mut budget = MAX_DESTINATION_NODES;
    search_destinations(names, name, 0, &mut budget)
}

fn destination_array(value: Object<'_>) -> Option<Array<'_>> {
    match value {
        Object::Array(values) => Some(values),
        Object::Dict(value) => value.get("D"),
        _ => None,
    }
}

fn search_destinations<'a>(
    node: Dict<'a>,
    name: &[u8],
    depth: u8,
    budget: &mut usize,
) -> Option<Array<'a>> {
    if depth == 16 || *budget == 0 {
        return None;
    }
    *budget -= 1;
    if let Some(names) = node.get::<Array<'_>>("Names") {
        let mut entries = names.flex_iter();
        while *budget > 0 {
            *budget -= 1;
            let key: PdfString<'_> = entries.next()?;
            let value: Object<'_> = entries.next()?;
            if key.as_bytes() == name {
                return destination_array(value);
            }
        }
    }
    for child in node.get::<Array<'_>>("Kids")?.iter::<Dict<'_>>() {
        if *budget == 0 {
            break;
        }
        if let Some(values) = search_destinations(child, name, depth + 1, budget) {
            return Some(values);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tex::disk_pdf_fixture;
    use std::io::Write;

    #[test]
    fn parses_safe_pdf_links_and_maps_page_rectangles() {
        let mut bytes = b"%PDF-1.4\n".to_vec();
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R /Names << /Dests << /Names [(chapter) [4 0 R /Fit]] >> >> >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 200] /Annots [5 0 R 6 0 R 7 0 R] >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 200] >>",
            "<< /Type /Annot /Subtype /Link /Rect [10 40 20 60] /A << /S /URI /URI (https://example.org/paper) >> >>",
            "<< /Type /Annot /Subtype /Link /Rect [20 70 40 90] /Dest (chapter) >>",
            "<< /Type /Annot /Subtype /Link /Rect [5 5 10 10] /A << /S /URI /URI (file:///tmp/private) >> >>",
        ];
        let mut offsets = Vec::new();
        for (index, object) in objects.into_iter().enumerate() {
            offsets.push(bytes.len());
            writeln!(bytes, "{} 0 obj\n{object}\nendobj", index + 1).unwrap();
        }
        let xref = bytes.len();
        writeln!(bytes, "xref\n0 8\n0000000000 65535 f ").unwrap();
        for offset in offsets {
            writeln!(bytes, "{offset:010} 00000 n ").unwrap();
        }
        writeln!(
            bytes,
            "trailer\n<< /Size 8 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF"
        )
        .unwrap();
        let snapshot = disk_pdf_fixture(&bytes).unwrap();
        let links = document_links_from_bytes(Arc::new(bytes)).unwrap();
        assert_eq!(retained_document_links(&snapshot).unwrap(), links);
        assert_eq!(links.iter().map(Vec::len).collect::<Vec<_>>(), [2, 0]);
        assert_eq!(
            links[0][0],
            PdfLink {
                left: 0.1,
                top: 0.7,
                right: 0.2,
                bottom: 0.8,
                target: LinkTarget::External("https://example.org/paper".into()),
            }
        );
        assert_eq!(links[0][1].target, LinkTarget::Page(1));
        assert!(external_url(b"https://user:fixture@localhost/", None).is_none());
        assert!(external_url(b"https://example.org/\x0A", None).is_none());
    }
}
