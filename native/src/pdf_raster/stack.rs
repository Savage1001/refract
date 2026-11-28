use super::{PageRequest, PageSize, page_layout};
use std::{io, ops::Range};

pub const INSET: f32 = 24.;
const PAGE_GAP: f32 = 18.;
pub const VIEWPORT_PIXEL_BUDGET: u32 = 16_000_000;

#[derive(Default)]
pub struct PageStack {
    pub pages: Vec<PageSlot>,
    pub width: f32,
    pub height: f32,
}

pub struct PageSlot {
    pub size: PageSize,
    pub width: f32,
    pub height: f32,
    pub top: f32,
}

impl PageSlot {
    fn bottom(&self) -> f32 {
        self.top + self.height
    }
}

impl PageStack {
    pub fn new(sizes: &[PageSize], width: f32, request: PageRequest) -> io::Result<Self> {
        let mut top = INSET;
        let mut longest = width;
        let mut pages = Vec::with_capacity(sizes.len());
        for &size in sizes {
            let layout = page_layout(size.width, size.height, request)?;
            longest = longest.max(layout.width + 2. * INSET);
            pages.push(PageSlot {
                size,
                width: layout.width,
                height: layout.height,
                top,
            });
            top += layout.height + PAGE_GAP;
        }
        Ok(Self {
            pages,
            width: longest,
            height: top - PAGE_GAP + INSET,
        })
    }

    pub fn visible(&self, scroll_top: f32, height: f32) -> Range<usize> {
        if height <= 0. {
            return 0..0;
        }
        let first = self
            .pages
            .partition_point(|page| page.bottom() <= scroll_top);
        let end = self
            .pages
            .partition_point(|page| page.top < scroll_top + height);
        first..end.max(first)
    }

    pub fn nearest(&self, y: f32) -> Option<usize> {
        let next = self.pages.partition_point(|page| page.top <= y);
        match (next.checked_sub(1), self.pages.get(next)) {
            (Some(previous), Some(after))
                if (y - self.pages[previous].bottom()).max(0.) > after.top - y =>
            {
                Some(next)
            }
            (Some(previous), _) => Some(previous),
            (_, Some(_)) => Some(next),
            _ => None,
        }
    }
}

pub fn per_page_budget(count: usize) -> u32 {
    (VIEWPORT_PIXEL_BUDGET / u32::try_from(count).unwrap_or(u32::MAX).max(1)).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf_raster::MAX_RASTER_PIXELS;

    #[test]
    fn finds_visible_pages_without_materializing_earlier_pages() {
        let request = PageRequest {
            index: 0,
            width: Some(200.),
            height: None,
            display_scale: 1.,
            pixel_budget: MAX_RASTER_PIXELS,
        };
        let stack = PageStack::new(
            &[PageSize {
                width: 200.,
                height: 300.,
            }; 5000],
            248.,
            request,
        )
        .unwrap();
        let top = stack.pages[2500].top;
        assert_eq!(stack.visible(top - 4., 400.), 2500..2502);
        assert_eq!(stack.nearest(top + 150.), Some(2500));
        assert_eq!(stack.nearest(top + 311.), Some(2501));
        assert_eq!(stack.visible(top, 0.), 0..0);
        assert_eq!(stack.width, 248.);
    }

    #[test]
    fn divides_the_bitmap_budget_across_visible_pages() {
        for count in [2, 3, 10, 100] {
            let request = PageRequest {
                pixel_budget: per_page_budget(count),
                ..PageRequest::project_file(0)
            };
            let page = page_layout(10_000., 0.1, request).unwrap();
            assert!(page.pixel_width * page.pixel_height * count as u32 <= VIEWPORT_PIXEL_BUDGET);
        }
    }
}
