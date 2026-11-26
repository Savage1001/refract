use super::{Library, Palette};
use gpui::{Context, Image, ImageFormat, KeyDownEvent, Window, div, img, prelude::*, px, rgb};
use std::{io, path::Path, sync::Arc};

pub(super) const PDF_PANE_WIDTH: f32 = 352.;
const PADDING: f32 = 16.;

#[derive(Clone, Copy, Default, PartialEq)]
enum Zoom {
    #[default]
    FitWidth,
    FitPage,
    Percentage(u16),
}

impl Zoom {
    fn percentage(self) -> u16 {
        match self {
            Self::FitWidth | Self::FitPage => 100,
            Self::Percentage(zoom) => zoom,
        }
    }
}

#[derive(Default)]
pub(super) struct PdfViewport {
    page: u32,
    zoom: Zoom,
    frame: Option<PageImage>,
    pending: bool,
    error: Option<String>,
}

struct PageImage {
    current: u32,
    count: u32,
    width: f32,
    height: f32,
    image: Arc<Image>,
}

struct PageRequest {
    index: u32,
    width: f32,
    height: Option<f32>,
    display_scale: f32,
}

struct RasterizedPage {
    current: u32,
    count: u32,
    width: f32,
    height: f32,
    png: Vec<u8>,
}

impl Library {
    pub(super) fn request_pdf_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(selection) = &mut self.selection {
            selection.build.preview.pending = selection.build.pdf().is_some();
            selection.build.preview.error = None;
        }
        self.start_pdf_frame(window, cx);
    }

    fn start_pdf_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pdf_task.is_some() || self.build_task.is_some() {
            return;
        }
        let Some(selection) = &mut self.selection else {
            return;
        };
        if !selection.build.preview.pending {
            return;
        }
        let Some(path) = selection.build.pdf().map(Path::to_owned) else {
            return;
        };
        let preview = &mut selection.build.preview;
        preview.pending = false;
        let based_width = PDF_PANE_WIDTH - PADDING * 2.;
        let request = PageRequest {
            index: preview.page,
            width: based_width * f32::from(preview.zoom.percentage()) / 100.,
            height: (preview.zoom == Zoom::FitPage)
                .then(|| (f32::from(window.viewport_size().height) - 136.).max(64.)),
            display_scale: window.scale_factor(),
        };
        let sequence = self.project_open_sequence;
        let work = cx
            .background_executor()
            .spawn(async move { render_page(&path, request) });
        self.pdf_task = Some(cx.spawn_in(window, async move |library, cx| {
            let page = work.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.pdf_task = None;
                if let Some(selection) = &mut library.selection
                    && library.project_open_sequence == sequence
                    && selection.build.pdf().is_some()
                    && !selection.build.preview.pending
                {
                    let pane = &mut selection.build.preview;
                    match page {
                        Ok(page) => {
                            pane.page = page.current;
                            pane.frame = Some(PageImage {
                                current: page.current,
                                count: page.count,
                                width: page.width,
                                height: page.height,
                                image: Arc::new(Image::from_bytes(ImageFormat::Png, page.png)),
                            });
                        }
                        Err(error) => pane.error = Some(error.to_string()),
                    }
                }
                library.start_pdf_frame(window, cx);
                library.resume_navigation(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn change_pdf_page(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        let preview = &mut selection.build.preview;
        let Some(frame) = &preview.frame else {
            return;
        };
        preview.page = preview
            .page
            .saturating_add_signed(delta)
            .min(frame.count - 1);
        self.request_pdf_frame(window, cx);
    }

    fn change_pdf_zoom(&mut self, zoom: Zoom, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(selection) = &mut self.selection {
            selection.build.preview.zoom = zoom;
            self.request_pdf_frame(window, cx);
        }
    }

    fn step_pdf_zoom(&mut self, delta: i16, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(selection) = &self.selection {
            let zoom = selection
                .build
                .preview
                .zoom
                .percentage()
                .saturating_add_signed(delta)
                .clamp(25, 250);
            self.change_pdf_zoom(Zoom::Percentage(zoom), window, cx);
        }
    }

    pub(super) fn pdf_controls(
        &self,
        view: &PdfViewport,
        ready: bool,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let zoom = view.zoom.percentage();
        let label = if view.zoom == Zoom::FitWidth {
            "Fit".to_owned()
        } else {
            format!("{zoom}%")
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .px_2()
            .children(view.frame.as_ref().map(|frame| {
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(pdf_button(
                        "pdf-prev",
                        "‹",
                        ready && view.page > 0,
                        palette,
                        cx,
                        |library, window, cx| library.change_pdf_page(-1, window, cx),
                    ))
                    .child(format!("{} / {}", frame.current + 1, frame.count))
                    .child(pdf_button(
                        "pdf-next",
                        "›",
                        ready && view.page < frame.count - 1,
                        palette,
                        cx,
                        |library, window, cx| library.change_pdf_page(1, window, cx),
                    ))
                    .child(div().flex_1())
                    .child(pdf_button(
                        "pdf-zoom-out",
                        "−",
                        ready && zoom > 25,
                        palette,
                        cx,
                        |library, window, cx| library.step_pdf_zoom(-25, window, cx),
                    ))
                    .child(label)
                    .child(pdf_button(
                        "pdf-zoom-in",
                        "+",
                        ready && zoom < 250,
                        palette,
                        cx,
                        |library, window, cx| library.step_pdf_zoom(25, window, cx),
                    ))
                    .child(pdf_button(
                        "pdf-fit-width",
                        "Fit width",
                        ready,
                        palette,
                        cx,
                        |library, window, cx| library.change_pdf_zoom(Zoom::FitWidth, window, cx),
                    ))
                    .child(pdf_button(
                        "pdf-fit-page",
                        "Fit page",
                        ready,
                        palette,
                        cx,
                        |library, window, cx| library.change_pdf_zoom(Zoom::FitPage, window, cx),
                    ))
            }))
    }
}

impl PdfViewport {
    pub(super) fn has_content(&self) -> bool {
        self.frame.is_some() || self.error.is_some()
    }

    pub(super) fn content(
        &self,
        status: Option<&'static str>,
        palette: Palette,
    ) -> impl IntoElement {
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
                    .id("pdf-area")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_scroll()
                    .p(px(PADDING))
                    .children(self.frame.as_ref().map(|frame| {
                        div()
                            .w(px(frame.width))
                            .min_w(px(frame.width))
                            .mx_auto()
                            .bg(rgb(0xffffff))
                            .shadow_sm()
                            .child(
                                img(frame.image.clone())
                                    .w(px(frame.width))
                                    .h(px(frame.height)),
                            )
                    })),
            )
    }
}

fn pdf_button(
    id: &'static str,
    label: &'static str,
    enabled: bool,
    palette: Palette,
    cx: &Context<Library>,
    activate: fn(&mut Library, &mut Window, &mut Context<Library>),
) -> impl IntoElement {
    div()
        .id(id)
        .debug_selector(|| id.to_owned())
        .px_1()
        .py_1()
        .rounded_md()
        .child(label)
        .when(!enabled, |button| button.text_color(rgb(palette.muted)))
        .when(enabled, |button| {
            button
                .tab_index(5)
                .cursor_pointer()
                .hover(move |style| style.bg(rgb(palette.selection)))
                .focus(move |style| style.bg(rgb(palette.selection)))
                .on_click(cx.listener(move |library, _, window, cx| activate(library, window, cx)))
                .on_key_down(
                    cx.listener(move |library, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                            activate(library, window, cx);
                        }
                    }),
                )
        })
}

#[cfg(windows)]
mod windows_pdf;
#[cfg(windows)]
use windows_pdf::render_page;

#[cfg(not(windows))]
fn render_page(_: &Path, _: PageRequest) -> io::Result<RasterizedPage> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Inline PDF rendering is unavailable on this platform.",
    ))
}
