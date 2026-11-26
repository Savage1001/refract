use super::{Library, Palette};
use gpui::{
    Bounds, Context, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point,
    RenderImage, ScrollHandle, Size, Window, canvas, div, img, point, prelude::*, px, rgb, size,
};
use refract_native::{
    pdf_raster::{
        MAX_RASTER_PIXELS, PageRequest, PageSize,
        stack::{INSET, PageSlot, PageStack, per_page_budget},
    },
    tex::synctex::PdfPosition,
};
use std::{cell::Cell, collections::BTreeMap, io, ops::Range, path::Path, rc::Rc, sync::Arc};

mod controls;
use controls::PageEdit;
pub(super) use controls::init;

pub(super) const PDF_PANE_WIDTH: f32 = 352.;

#[derive(Clone, Copy, Default, PartialEq)]
enum Zoom {
    #[default]
    FitWidth,
    FitPage(u16),
    Percentage(u16),
}

impl Zoom {
    fn percentage(self) -> u16 {
        match self {
            Self::FitWidth => 100,
            Self::FitPage(zoom) | Self::Percentage(zoom) => zoom,
        }
    }
}

#[derive(Default)]
pub(super) struct PdfViewport {
    page: u32,
    zoom: Zoom,
    sizes: Vec<PageSize>,
    layout: PageStack,
    frames: BTreeMap<u32, RasterizedPage>,
    failed_pages: BTreeMap<u32, String>,
    visible: Range<usize>,
    viewport: Size<Pixels>,
    measurement: Rc<Cell<Option<(Point<Pixels>, Size<Pixels>)>>>,
    generation: u64,
    request_document: bool,
    loading_document: bool,
    error: Option<String>,
    last_sync: Option<PdfPosition>,
    center_sync: bool,
    scroll: ScrollHandle,
    page_focus: Option<FocusHandle>,
    page_edit: Option<PageEdit>,
}

pub(super) struct RasterizedPage {
    pub(super) current: u32,
    pub(super) count: u32,
    width: f32,
    height: f32,
    pub(super) image: Arc<RenderImage>,
}

enum PdfWork {
    ReadDocument,
    RenderPage(PageRequest),
}

enum PdfUpdate {
    Document(io::Result<Vec<PageSize>>),
    Page(PageRequest, io::Result<RasterizedPage>),
}

impl Library {
    pub(super) fn load_built_pdf(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(selection) = &mut self.selection {
            let available = selection.build.pdf().is_some();
            let preview = &mut selection.build.preview;
            preview.generation = preview.generation.wrapping_add(1);
            preview.request_document = available;
            preview.loading_document = available;
            preview.error = None;
            preview.page_edit = None;
            if available && preview.page_focus.is_none() {
                preview.page_focus = Some(cx.focus_handle().tab_index(5).tab_stop(true));
            }
        }
        self.start_pdf_frame(window, cx);
    }

    pub(super) fn request_pdf_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(selection) = &mut self.selection {
            selection.build.preview.apply_sync_jump(window);
        }
        self.start_pdf_frame(window, cx);
        cx.notify();
    }

    fn start_pdf_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pdf_task.is_some() || self.build_task.is_some() {
            return;
        }
        let Some(selection) = &mut self.selection else {
            return;
        };
        let Some(path) = selection.build.pdf().map(Path::to_owned) else {
            return;
        };
        let preview = &mut selection.build.preview;
        let work = if std::mem::take(&mut preview.request_document) {
            PdfWork::ReadDocument
        } else if !preview.loading_document
            && let Some(index) = preview.next_visible_page()
        {
            PdfWork::RenderPage(preview.request(index, window.scale_factor()))
        } else {
            return;
        };
        let sequence = self.project_open_sequence;
        let generation = preview.generation;
        let work = cx.background_executor().spawn(async move {
            match work {
                PdfWork::ReadDocument => PdfUpdate::Document(read_page_sizes(&path)),
                PdfWork::RenderPage(request) => {
                    PdfUpdate::Page(request, render_page(&path, request))
                }
            }
        });
        self.pdf_task = Some(cx.spawn_in(window, async move |library, cx| {
            let update = work.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.pdf_task = None;
                if let Some(selection) = &mut library.selection
                    && library.project_open_sequence == sequence
                    && selection.build.pdf().is_some()
                    && selection.build.preview.generation == generation
                {
                    let pane = &mut selection.build.preview;
                    match update {
                        PdfUpdate::Document(sizes) => pane.set_document(sizes, window),
                        PdfUpdate::Page(request, frame) => pane.accept_page(request, frame),
                    }
                }
                library.start_pdf_frame(window, cx);
                library.resume_navigation(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn pdf_scroll_changed(
        &mut self,
        measurement: &Rc<Cell<Option<(Point<Pixels>, Size<Pixels>)>>>,
        viewport: Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(selection) = &mut self.selection {
            let preview = &mut selection.build.preview;
            if !Rc::ptr_eq(measurement, &preview.measurement) {
                return;
            }
            if preview.viewport != viewport {
                preview.viewport = viewport;
                if !preview.loading_document {
                    preview.resize_pages(preview.zoom, window);
                }
            }
            preview.update_visible(window);
        }
        self.start_pdf_frame(window, cx);
        cx.notify();
    }
}

impl PdfViewport {
    pub(super) fn release(&mut self, window: &mut Window) {
        for frame in std::mem::take(&mut self.frames).into_values() {
            let _ = window.drop_image(frame.image);
        }
    }

    fn page_width(&self) -> f32 {
        (f32::from(self.viewport.width) - 2. * INSET).max(1.)
    }

    fn request(&self, index: u32, display_scale: f32) -> PageRequest {
        PageRequest {
            index,
            width: Some(self.page_width() * f32::from(self.zoom.percentage()) / 100.),
            height: None,
            display_scale,
            pixel_budget: self.pixel_budget(),
        }
    }

    fn pixel_budget(&self) -> u32 {
        per_page_budget(self.visible.len()).min(MAX_RASTER_PIXELS)
    }

    fn fits_viewport_budget(&self, page: &RasterizedPage) -> bool {
        page.image
            .as_bytes(0)
            .is_some_and(|bytes| bytes.len() / 4 <= self.pixel_budget() as usize)
    }

    fn accept_page(&mut self, request: PageRequest, frame: io::Result<RasterizedPage>) {
        if !self.visible.contains(&(request.index as usize)) {
            return;
        }
        let error = match frame {
            Ok(frame) if frame.current == request.index && self.fits_viewport_budget(&frame) => {
                self.frames.insert(request.index, frame);
                return;
            }
            // A second page became visible while the worker rendered this frame.
            Ok(frame)
                if frame.current == request.index && request.pixel_budget > self.pixel_budget() =>
            {
                return;
            }
            Ok(_) => "Couldn’t render the requested PDF page.".to_owned(),
            Err(error) => error.to_string(),
        };
        self.failed_pages.insert(request.index, error);
    }

    fn next_visible_page(&self) -> Option<u32> {
        let need =
            |index| !self.frames.contains_key(&index) && !self.failed_pages.contains_key(&index);
        if self.visible.contains(&(self.page as usize)) && need(self.page) {
            Some(self.page)
        } else {
            self.visible
                .clone()
                .map(|index| index as u32)
                .find(|&index| need(index))
        }
    }

    fn set_document(&mut self, result: io::Result<Vec<PageSize>>, window: &mut Window) {
        self.loading_document = false;
        self.release(window);
        self.failed_pages.clear();
        self.visible = 0..0;
        if self.viewport.height <= px(0.) || self.viewport.width <= px(0.) {
            self.viewport = size(
                px(PDF_PANE_WIDTH),
                (window.viewport_size().height - px(136.)).max(px(1.)),
            );
        }
        let document = result.and_then(|sizes| {
            if sizes.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "The PDF has no pages.",
                ));
            }
            let layout = self.make_layout(&sizes, window.scale_factor())?;
            Ok((sizes, layout))
        });
        match document {
            Ok((sizes, layout)) => {
                let old_offset = self.scroll.offset();
                self.sizes = sizes;
                self.layout = layout;
                self.scroll_to(-f32::from(old_offset.x), -f32::from(old_offset.y));
                self.apply_sync_jump(window);
                self.update_visible(window);
            }
            Err(error) => {
                self.sizes.clear();
                self.layout = PageStack::default();
                self.error = Some(error.to_string());
            }
        }
    }

    fn make_layout(&self, sizes: &[PageSize], display_scale: f32) -> io::Result<PageStack> {
        PageStack::new(
            sizes,
            f32::from(self.viewport.width),
            self.request(0, display_scale),
        )
    }

    fn scroll_to(&self, left: f32, top: f32) {
        let max_x = (self.layout.width - f32::from(self.viewport.width)).max(0.);
        let max_y = (self.layout.height - f32::from(self.viewport.height)).max(0.);
        self.scroll
            .set_offset(point(px(-left.clamp(0., max_x)), px(-top.clamp(0., max_y))));
    }

    fn go_to_page(&mut self, index: u32, window: &mut Window) {
        if let Some(slot) = self.layout.pages.get(index as usize) {
            self.scroll_to(-f32::from(self.scroll.offset().x), slot.top - INSET);
            self.update_visible(window);
            self.page = index;
        }
    }

    fn resize_pages(&mut self, zoom: Zoom, window: &mut Window) {
        if self.sizes.is_empty() {
            self.zoom = zoom;
            return;
        }
        let offset = self.scroll.offset();
        let half_width = f32::from(self.viewport.width) / 2.;
        let half_height = f32::from(self.viewport.height) / 2.;
        let center_x = half_width - f32::from(offset.x);
        let center_y = half_height - f32::from(offset.y);
        let anchor = self.layout.nearest(center_y).map(|index| {
            let slot = &self.layout.pages[index];
            let left = (self.layout.width - slot.width) / 2.;
            (
                index,
                (center_x - left) / slot.width,
                (center_y - slot.top) / slot.height,
            )
        });
        self.zoom = zoom;
        match self.make_layout(&self.sizes, window.scale_factor()) {
            Ok(layout) => {
                self.generation = self.generation.wrapping_add(1);
                self.layout = layout;
                self.release(window);
                self.failed_pages.clear();
                self.error = None;
                if let Some((index, x, y)) = anchor {
                    let slot = &self.layout.pages[index];
                    let left = (self.layout.width - slot.width) / 2.;
                    self.scroll_to(
                        left + x * slot.width - half_width,
                        slot.top + y * slot.height - half_height,
                    );
                }
                self.update_visible(window);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn update_visible(&mut self, window: &mut Window) {
        let top = -f32::from(self.scroll.offset().y);
        let height = f32::from(self.viewport.height);
        self.visible = self.layout.visible(top, height);
        if let Some(index) = self.layout.nearest(top + height / 2.) {
            self.page = index as u32;
        }
        let visible = &self.visible;
        let budget = self.pixel_budget() as usize;
        self.frames.retain(|&index, page| {
            let keep = visible.contains(&(index as usize))
                && page
                    .image
                    .as_bytes(0)
                    .is_some_and(|bytes| bytes.len() / 4 <= budget);
            if !keep {
                let _ = window.drop_image(page.image.clone());
            }
            keep
        });
        self.failed_pages
            .retain(|&index, _| visible.contains(&(index as usize)));
    }

    fn apply_sync_jump(&mut self, window: &mut Window) {
        if !self.center_sync || self.loading_document {
            return;
        }
        let Some(position) = self.last_sync else {
            return;
        };
        let Some(slot) = self
            .layout
            .pages
            .get(position.page.saturating_sub(1) as usize)
        else {
            return;
        };
        let y = (position.y / slot.size.height) as f32;
        let left = -f32::from(self.scroll.offset().x);
        self.scroll_to(
            left,
            slot.top + y * slot.height - f32::from(self.viewport.height) / 2.,
        );
        self.center_sync = false;
        self.update_visible(window);
    }

    pub(super) fn jump_to(&mut self, position: PdfPosition) {
        self.page = position.page.saturating_sub(1);
        self.last_sync = Some(position);
        self.center_sync = true;
    }

    pub(super) fn clear_sync(&mut self) {
        self.last_sync = None;
        self.center_sync = false;
    }

    pub(super) fn has_content(&self) -> bool {
        self.loading_document || !self.layout.pages.is_empty() || self.error.is_some()
    }

    pub(super) fn content(
        &self,
        status: Option<&'static str>,
        palette: Palette,
        ready: bool,
        cx: &Context<Library>,
    ) -> impl IntoElement {
        let scroll = self.scroll.clone();
        let measurement = self.measurement.clone();
        let view = cx.weak_entity();
        div()
            .flex()
            .flex_col()
            .size_full()
            .when_some(status, |pane, status| {
                pane.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_color(rgb(palette.muted))
                        .child(status),
                )
            })
            .children(
                self.error
                    .clone()
                    .map(|error| div().p_3().text_color(rgb(palette.warning)).child(error)),
            )
            .child(
                div()
                    .on_children_prepainted(move |_, window, app| {
                        let frame = (scroll.offset(), scroll.bounds().size);
                        if measurement.replace(Some(frame)) != Some(frame) {
                            let view = view.clone();
                            let measurement = measurement.clone();
                            window.defer(app, move |window, app| {
                                let _ = view.update(app, |view, cx| {
                                    view.pdf_scroll_changed(&measurement, frame.1, window, cx)
                                });
                            });
                        }
                    })
                    .id("pdf-area")
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .overflow_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .w(px(self.layout.width))
                            .h(px(self.layout.height))
                            .children(self.visible.clone().filter_map(|index| {
                                let slot = self.layout.pages.get(index)?;
                                let left = (self.layout.width - slot.width) / 2.;
                                Some(
                                    render_page_slot(
                                        index as u32,
                                        slot,
                                        left,
                                        self.frames.get(&(index as u32)),
                                        self.last_sync,
                                        ready && !self.loading_document,
                                        cx,
                                    )
                                    .children(
                                        self.failed_pages.get(&(index as u32)).map(|error| {
                                            div()
                                                .p_3()
                                                .text_color(rgb(palette.warning))
                                                .child(error.clone())
                                        }),
                                    ),
                                )
                            })),
                    ),
            )
    }
}

fn render_page_slot(
    index: u32,
    slot: &PageSlot,
    left: f32,
    frame: Option<&RasterizedPage>,
    last_sync: Option<PdfPosition>,
    ready: bool,
    cx: &Context<Library>,
) -> impl IntoElement + ParentElement {
    let page = index + 1;
    let point_width = slot.size.width;
    let point_height = slot.size.height;
    div()
        .id(("pdf-page", index as usize))
        .debug_selector(|| format!("pdf-page-{index}"))
        .absolute()
        .top(px(slot.top))
        .left(px(left))
        .w(px(slot.width))
        .h(px(slot.height))
        .bg(rgb(0xffffff))
        .shadow_sm()
        .children(frame.map(|frame| {
            img(frame.image.clone())
                .w(px(frame.width))
                .h(px(frame.height))
        }))
        .when(ready, |image| {
            let bounds = Rc::new(Cell::new(Bounds::<Pixels>::default()));
            let layout = bounds.clone();
            image
                .tab_index(5)
                .child(
                    canvas(move |bounds, _, _| layout.set(bounds), |_, (), _, _| {})
                        .absolute()
                        .inset_0(),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |library, event: &MouseDownEvent, window, cx| {
                        let bounds = bounds.get();
                        if event.click_count == 2
                            && bounds.size.width > px(0.)
                            && bounds.size.height > px(0.)
                        {
                            let x = f64::from(
                                f32::from(event.position.x - bounds.origin.x)
                                    / f32::from(bounds.size.width),
                            );
                            let y = f64::from(
                                f32::from(event.position.y - bounds.origin.y)
                                    / f32::from(bounds.size.height),
                            );
                            library.reverse_pdf_at(
                                PdfPosition {
                                    page,
                                    x: x.clamp(0., 1.) * point_width,
                                    y: y.clamp(0., 1.) * point_height,
                                },
                                window,
                                cx,
                            );
                        }
                    }),
                )
                .on_key_down(
                    cx.listener(move |library, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                            let position = last_sync
                                .filter(|position| position.page == page)
                                .unwrap_or(PdfPosition {
                                    page,
                                    x: point_width / 2.,
                                    y: point_height / 2.,
                                });
                            library.reverse_pdf_at(position, window, cx);
                            cx.stop_propagation();
                        }
                    }),
                )
        })
}

#[cfg(windows)]
mod windows_pdf;
#[cfg(windows)]
use windows_pdf::{read_page_sizes, render_page};

#[cfg(windows)]
pub(super) use windows_pdf::render_asset_page;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod portable_pdf;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) use portable_pdf::render_asset_page;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use portable_pdf::{read_page_sizes, render_page};

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub(super) fn render_asset_page(_: Arc<Vec<u8>>, _: u32) -> io::Result<RasterizedPage> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Inline project PDF preview is unavailable on this platform.",
    ))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn render_page(_: &Path, _: PageRequest) -> io::Result<RasterizedPage> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Inline PDF rendering is unavailable on this platform.",
    ))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn read_page_sizes(_: &Path) -> io::Result<Vec<PageSize>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Inline PDF rendering is unavailable on this platform.",
    ))
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::asset_image;
    use gpui::TestAppContext;

    #[gpui::test]
    fn page_navigation_centers_source_jumps_and_releases_offscreen_bitmaps(
        cx: &mut TestAppContext,
    ) {
        let cx = cx.add_empty_window();
        cx.update(|window, _| {
            let mut view = PdfViewport {
                viewport: size(px(352.), px(100.)),
                ..PdfViewport::default()
            };
            let size = PageSize {
                width: 320.,
                height: 480.,
            };
            view.set_document(Ok(vec![size; 20]), window);
            view.accept_page(
                view.request(0, 1.),
                Ok(RasterizedPage {
                    current: 0,
                    count: 20,
                    width: 320.,
                    height: 480.,
                    image: asset_image::frame_image(image::RgbaImage::new(32, 48)),
                }),
            );
            assert_eq!(view.visible, 0..1);
            assert!(view.frames.contains_key(&0));

            view.go_to_page(10, window);
            assert_eq!(view.visible, 10..11);
            assert_eq!(view.page, 10);
            assert!(view.frames.is_empty());
            assert_eq!(view.next_visible_page(), Some(10));

            view.jump_to(PdfPosition {
                page: 8,
                x: 120.,
                y: 240.,
            });
            view.apply_sync_jump(window);
            assert_eq!(view.visible, 7..8);
            view.resize_pages(Zoom::Percentage(150), window);
            let center_in_page = 50. - f32::from(view.scroll.offset().y) - view.layout.pages[7].top;
            assert!((center_in_page - 360.).abs() < 0.01);
            assert_eq!(view.visible, 7..8);
        });
    }
}
