use super::{Library, Palette};
use gpui::{
    Bounds, Context, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point,
    RenderImage, ScrollHandle, Size, Window, canvas, div, img, point, prelude::*, px, rgb, size,
};
use gpui_component::tooltip::Tooltip;
use refract_native::{
    pdf_raster::{
        MAX_RASTER_PIXELS, PageRequest, PageSize,
        links::{LinkTarget, PdfLink, retained_document_links},
        stack::{INSET, PageSlot, PageStack, per_page_budget},
        text::{TextPage, retained_page_text},
    },
    tex::{RetainedPdf, synctex::PdfPosition},
};
use std::{cell::Cell, collections::BTreeMap, io, ops::Range, rc::Rc, sync::Arc};

mod controls;
mod selection;
use controls::PageEdit;
pub(super) use controls::init;
use selection::TextSelection;

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
    links: Vec<Vec<PdfLink>>,
    text_pages: BTreeMap<u32, TextPage>,
    text_selection: Option<TextSelection>,
    layout: PageStack,
    frames: BTreeMap<u32, RasterizedPage>,
    failed_pages: BTreeMap<u32, String>,
    visible: Range<usize>,
    viewport: Size<Pixels>,
    measurement: Rc<Cell<Option<(Point<Pixels>, Size<Pixels>)>>>,
    generation: u64,
    document: Option<Arc<RetainedPdf>>,
    pending_document: Option<Arc<RetainedPdf>>,
    document_generation: u64,
    committed_document_generation: u64,
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
    ReadDocument(Arc<RetainedPdf>),
    RenderPage(Arc<RetainedPdf>, PageRequest, bool),
}

enum PdfUpdate {
    Document(
        Arc<RetainedPdf>,
        io::Result<Vec<PageSize>>,
        Vec<Vec<PdfLink>>,
    ),
    Page(PageRequest, io::Result<RasterizedPage>, Option<TextPage>),
}

impl Library {
    pub(super) fn load_built_pdf(
        &mut self,
        result: io::Result<RetainedPdf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(selection) = &mut self.selection {
            let preview = &mut selection.build.preview;
            preview.document_generation = preview.document_generation.wrapping_add(1);
            match result {
                Ok(document) => {
                    preview.pending_document = Some(Arc::new(document));
                    preview.loading_document = true;
                    preview.error = None;
                    if preview.page_focus.is_none() {
                        preview.page_focus = Some(cx.focus_handle().tab_index(5).tab_stop(true));
                    }
                }
                Err(error) => {
                    preview.pending_document = None;
                    preview.loading_document = false;
                    preview.error = Some(error.to_string());
                    preview.clear_sync();
                }
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
        if self.pdf_task.is_some() {
            return;
        }
        let visible = self.pdf_is_visible(window);
        let Some(selection) = &mut self.selection else {
            return;
        };
        let preview = &mut selection.build.preview;
        let work = if let Some(document) = preview.pending_document.take() {
            PdfWork::ReadDocument(document)
        } else if visible
            && let Some(document) = preview.document.as_ref()
            && let Some(index) = preview.next_visible_page()
        {
            let needs_text = !preview.text_pages.contains_key(&index);
            PdfWork::RenderPage(
                document.clone(),
                preview.request(index, window.scale_factor()),
                needs_text,
            )
        } else {
            return;
        };
        let sequence = self.project_open_sequence;
        let generation = preview.generation;
        let document_generation = preview.document_generation;
        let work = cx.background_executor().spawn(async move {
            match work {
                PdfWork::ReadDocument(document) => {
                    let sizes = read_page_sizes(&document);
                    let links = if sizes.is_ok() {
                        retained_document_links(&document).unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    PdfUpdate::Document(document, sizes, links)
                }
                PdfWork::RenderPage(document, request, needs_text) => {
                    let image = raster_retained_pdf(&document, request);
                    let text = (needs_text && image.is_ok())
                        .then(|| retained_page_text(&document, request.index).unwrap_or_default());
                    PdfUpdate::Page(request, image, text)
                }
            }
        });
        self.pdf_task = Some(cx.spawn_in(window, async move |library, cx| {
            let update = work.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.pdf_task = None;
                let visible = library.pdf_is_visible(window);
                if let Some(selection) = &mut library.selection
                    && library.project_open_sequence == sequence
                {
                    let pane = &mut selection.build.preview;
                    match update {
                        PdfUpdate::Document(document, sizes, links)
                            if pane.document_generation == document_generation =>
                        {
                            if pane.set_document(sizes, window) {
                                pane.document = Some(document);
                                pane.committed_document_generation = document_generation;
                                pane.links = links;
                            }
                        }
                        PdfUpdate::Page(request, frame, text)
                            if visible && pane.generation == generation =>
                        {
                            if let Some(text) = text
                                && pane.visible.contains(&(request.index as usize))
                            {
                                pane.text_pages.insert(request.index, text);
                            }
                            pane.accept_page(request, frame);
                        }
                        _ => {}
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
                let zoom = match preview.zoom {
                    Zoom::FitPage(_) => preview.fit_page_zoom().unwrap_or(preview.zoom),
                    zoom => zoom,
                };
                preview.resize_pages(zoom, window);
            }
            preview.update_visible(window);
        }
        self.start_pdf_frame(window, cx);
        cx.notify();
    }
}

impl PdfViewport {
    pub(super) fn hide(&mut self, window: &mut Window) {
        self.release(window);
        self.measurement.set(None);
        self.page_edit = None;
        self.text_pages.clear();
        self.text_selection = None;
    }

    pub(super) fn release(&mut self, window: &mut Window) {
        for frame in std::mem::take(&mut self.frames).into_values() {
            let _ = window.drop_image(frame.image);
        }
    }

    fn page_width(&self) -> f32 {
        (f32::from(self.viewport.width) - 2. * INSET).max(1.)
    }

    fn fit_page_zoom(&self) -> Option<Zoom> {
        let first = self.sizes.first()?;
        let first_height = f64::from(self.page_width()) * first.height / first.width;
        let available = f64::from((f32::from(self.viewport.height) - 36.).max(1.));
        let zoom = (100. * available / first_height).floor().clamp(25., 100.) as u16;
        Some(Zoom::FitPage(zoom))
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

    fn set_document(&mut self, result: io::Result<Vec<PageSize>>, window: &mut Window) -> bool {
        self.loading_document = false;
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
                self.generation = self.generation.wrapping_add(1);
                self.text_pages.clear();
                self.text_selection = None;
                self.release(window);
                self.failed_pages.clear();
                self.error = None;
                let old_offset = self.scroll.offset();
                self.sizes = sizes;
                self.layout = layout;
                self.scroll_to(-f32::from(old_offset.x), -f32::from(old_offset.y));
                self.apply_sync_jump(window);
                self.update_visible(window);
                true
            }
            Err(error) => {
                self.clear_sync();
                self.error = Some(error.to_string());
                false
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
        let selection = self.text_selection;
        self.text_pages.retain(|&index, _| {
            visible.contains(&(index as usize))
                || selection.is_some_and(|selection| selection.contains_page(index))
        });
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
        sync_ready: bool,
        cx: &Context<Library>,
    ) -> impl IntoElement {
        let scroll = self.scroll.clone();
        let measurement = self.measurement.clone();
        let view = cx.weak_entity();
        let sync_ready = sync_ready
            && !self.loading_document
            && self.document_generation == self.committed_document_generation;
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
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|library, _, _, cx| library.finish_pdf_selection(cx)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|library, _, _, cx| library.finish_pdf_selection(cx)),
                    )
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
                                        self.links
                                            .get(index)
                                            .map(Vec::as_slice)
                                            .unwrap_or_default(),
                                        self,
                                        self.last_sync,
                                        sync_ready,
                                        palette,
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
    links: &[PdfLink],
    preview: &PdfViewport,
    last_sync: Option<PdfPosition>,
    sync_ready: bool,
    palette: Palette,
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
        .children(selection::highlights(preview, index, slot))
        .when(preview.document.is_some(), |image| {
            let bounds = Rc::new(Cell::new(Bounds::<Pixels>::default()));
            let layout = bounds.clone();
            let selection_bounds = bounds.clone();
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
                        if sync_ready
                            && event.click_count == 2
                            && bounds.size.width > px(0.)
                            && bounds.size.height > px(0.)
                        {
                            library.clear_pdf_selection(cx);
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
                        } else {
                            library.begin_pdf_selection(index, bounds, event, cx);
                        }
                    }),
                )
                .on_mouse_move(
                    cx.listener(move |library, event: &gpui::MouseMoveEvent, _, cx| {
                        library.move_pdf_selection(index, selection_bounds.get(), event, cx);
                    }),
                )
                .on_key_down(
                    cx.listener(move |library, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "c" && event.keystroke.modifiers.secondary() {
                            if library.copy_pdf_selection(cx) {
                                cx.stop_propagation();
                            }
                        } else if event.keystroke.key == "escape" {
                            library.clear_pdf_selection(cx);
                        } else if sync_ready
                            && (event.keystroke.key == "enter" || event.keystroke.key == "space")
                        {
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
                .children(
                    links
                        .iter()
                        .enumerate()
                        .map(|(index, link)| render_pdf_link(index, slot, link, palette, cx)),
                )
        })
}

fn render_pdf_link(
    index: usize,
    slot: &PageSlot,
    link: &PdfLink,
    palette: Palette,
    cx: &Context<Library>,
) -> impl IntoElement {
    let label = match &link.target {
        LinkTarget::Page(_) => "Go to linked PDF location",
        LinkTarget::External(_) => "Open PDF link",
    };
    let target = link.target.clone();
    let key_target = target.clone();
    div()
        .id(("pdf-link", index))
        .debug_selector(|| format!("pdf-link-{index}"))
        .absolute()
        .left(px(link.left * slot.width))
        .top(px(link.top * slot.height))
        .w(px((link.right - link.left) * slot.width))
        .h(px((link.bottom - link.top) * slot.height))
        .tab_index(5)
        .cursor_pointer()
        .focus(move |style| style.border_1().border_color(rgb(palette.focus)))
        .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |library, _, window, cx| {
            library.follow_pdf_link(&target, window, cx);
            cx.stop_propagation();
        }))
        .on_key_down(
            cx.listener(move |library, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                    library.follow_pdf_link(&key_target, window, cx);
                    cx.stop_propagation();
                }
            }),
        )
}

impl Library {
    fn follow_pdf_link(&mut self, link: &LinkTarget, window: &mut Window, cx: &mut Context<Self>) {
        match link {
            LinkTarget::Page(index) => {
                if let Some(selection) = &mut self.selection {
                    selection.build.preview.go_to_page(*index, window);
                    self.start_pdf_frame(window, cx);
                    cx.notify();
                }
            }
            LinkTarget::External(url) => cx.open_url(url),
        }
    }
}

#[cfg(windows)]
mod windows_pdf;
#[cfg(windows)]
use windows_pdf::{raster_retained_pdf, read_page_sizes};

#[cfg(windows)]
pub(super) use windows_pdf::{raster_pdf_bytes, render_asset_page};

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod portable_pdf;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) use portable_pdf::{raster_pdf_bytes, render_asset_page};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use portable_pdf::{raster_retained_pdf, read_page_sizes};

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub(super) fn render_asset_page(_: Arc<Vec<u8>>, _: u32) -> io::Result<RasterizedPage> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Inline project PDF preview is unavailable on this platform.",
    ))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub(super) fn raster_pdf_bytes(_: Arc<Vec<u8>>, _: PageRequest) -> io::Result<RasterizedPage> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "PDF thumbnails are unavailable on this platform.",
    ))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn read_page_sizes(_: &RetainedPdf) -> io::Result<Vec<PageSize>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Inline PDF rendering is unavailable on this platform.",
    ))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn raster_retained_pdf(_: &RetainedPdf, _: PageRequest) -> io::Result<RasterizedPage> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Inline PDF rendering is unavailable on this platform.",
    ))
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::{asset_image, initialize_app};
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use gpui_component::Root;
    use refract_native::{
        Project, create_project,
        tex::{BuildResult, disk_pdf_fixture},
    };
    use std::{fs, io::Write, path::Path, sync::atomic::AtomicBool};

    fn two_page_pdf() -> Vec<u8> {
        let text = "BT /F1 12 Tf 30 390 Td (Retained text) Tj ET\n";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 450] /Resources << /Font << /F1 6 0 R >> >> /Contents 5 0 R /Annots [7 0 R] >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 450] /Resources << /Font << /F1 6 0 R >> >> /Contents 5 0 R >>".into(),
            format!("<< /Length {} >>\nstream\n{text}endstream", text.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
            "<< /Type /Annot /Subtype /Link /Rect [100 315 200 375] /Dest [4 0 R /Fit] >>".into(),
        ];
        let mut bytes = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
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
        bytes
    }

    fn compile_then_fail(
        projects: &Path,
        project: &Project,
        _: &AtomicBool,
        _: bool,
    ) -> io::Result<BuildResult> {
        let folder = projects.join(&project.id);
        let pdf = folder.join(".refract-build/main.pdf");
        let broken = fs::read_to_string(folder.join("main.tex"))?.contains("Broken build");
        if broken {
            fs::remove_file(&pdf)?;
        } else {
            fs::create_dir_all(pdf.parent().unwrap())?;
            fs::write(&pdf, two_page_pdf())?;
        }
        Ok(BuildResult {
            cached: false,
            arguments: Vec::new(),
            exit_code: Some(if broken { 1 } else { 0 }),
            log: if broken {
                "./sections/abstract.tex:2: Undefined control sequence.\nl.2"
            } else {
                "Fixture build output"
            }
            .into(),
            pdf: (!broken).then_some(pdf),
        })
    }

    fn click(cx: &mut VisualTestContext, id: &'static str) {
        cx.refresh().unwrap();
        let bounds = cx.debug_bounds(id).expect(id);
        cx.simulate_click(bounds.center(), Modifiers::none());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn failed_recompile_keeps_the_previous_pdf_interactive(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Retained preview").unwrap();
        let folder = projects.join(&project.id);
        let output = folder.join(".refract-build/main.pdf");
        fs::create_dir(folder.join("sections")).unwrap();
        fs::write(
            folder.join("sections/abstract.tex"),
            "Introduction\nBroken line\n",
        )
        .unwrap();
        cx.update(initialize_app);
        let mut library = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Library::new(projects, window, cx));
            library = Some(view.clone());
            Root::new(view, window, cx)
        });
        let library = library.unwrap();
        cx.update(|window, app| {
            library.update(app, |view, cx| {
                view.test_compiler = Some(compile_then_fail);
                view.open_project(project, window, cx);
            });
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            library.update(app, |view, cx| {
                assert!(view.selection.as_ref().unwrap().build.pdf().is_some());
                // Exercise the disk-backed path with a fixture small enough for UI tests.
                view.load_built_pdf(disk_pdf_fixture(&two_page_pdf()), window, cx);
            });
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            let view = library.read(app);
            assert!(matches!(
                view.selection
                    .as_ref()
                    .unwrap()
                    .build
                    .preview
                    .document
                    .as_deref(),
                Some(RetainedPdf::File(_))
            ));
            view.active_editor()
                .unwrap()
                .update(app, |input, cx| input.focus(window, cx));
        });
        cx.simulate_input("Broken build");
        cx.run_until_parked();
        assert!(!output.exists());
        cx.update(|_, app| {
            let build = &library.read(app).selection.as_ref().unwrap().build;
            assert!(build.pdf().is_none());
            assert_eq!(build.preview.layout.pages.len(), 2);
            assert_eq!(build.preview.links[0][0].target, LinkTarget::Page(1));
        });

        click(cx, "pdf-next");
        let (first, last) = cx.update(|_, app| {
            let pane = &library.read(app).selection.as_ref().unwrap().build.preview;
            assert_eq!(pane.page, 1);
            assert!(pane.frames.contains_key(&1));
            let text = pane.text_pages.get(&1).unwrap();
            assert_eq!(text.text(0..text.len()), "Retained text");
            let first = text.bounds(0..1).next().unwrap();
            let last = text.bounds(text.len() - 1..text.len()).next().unwrap();
            (first, last)
        });
        let page = cx.debug_bounds("pdf-page-1").unwrap();
        let glyph = |x: f32, y: f32| page.origin + point(page.size.width * x, page.size.height * y);
        let start = glyph(
            first.left * 0.75 + first.right * 0.25,
            (first.top + first.bottom) * 0.5,
        );
        let end = glyph(
            last.left * 0.25 + last.right * 0.75,
            (last.top + last.bottom) * 0.5,
        );
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
        cx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-c"
        } else {
            "ctrl-c"
        });
        cx.update(|_, app| {
            assert_eq!(
                app.read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref(),
                Some("Retained text")
            );
        });
        click(cx, "pdf-zoom-in");
        cx.update(|_, app| {
            let pane = &library.read(app).selection.as_ref().unwrap().build.preview;
            assert_eq!(pane.zoom.percentage(), 125);
            assert!(!pane.frames.is_empty());
        });
        click(cx, "pdf-prev");
        click(cx, "pdf-link-0");
        cx.update(|_, app| {
            let pane = &library.read(app).selection.as_ref().unwrap().build.preview;
            assert_eq!(pane.page, 1);
        });

        click(cx, "preview-error-log");
        cx.refresh().unwrap();
        assert!(cx.debug_bounds("preview-error-output").is_some());
        click(cx, "preview-error-log");
        click(cx, "preview-error-source");
        cx.update(|window, app| {
            let view = library.read(app);
            let selection = view.selection.as_ref().unwrap();
            assert_eq!(selection.file.as_deref(), Some("sections/abstract.tex"));
            let editor = view.active_editor().unwrap();
            assert_eq!(
                crate::editor_find_view::selected_bytes(&editor, window, app),
                "Introduction\n".len().."Introduction\nBroken line".len()
            );
        });
    }

    #[gpui::test]
    fn page_navigation_centers_source_jumps_and_releases_offscreen_bitmaps(
        cx: &mut TestAppContext,
    ) {
        let cx = cx.add_empty_window();
        cx.update(|window, _| {
            let mut view = PdfViewport {
                viewport: size(px(320. + 2. * INSET), px(100.)),
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
            assert_eq!(view.visible, 9..11);
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
