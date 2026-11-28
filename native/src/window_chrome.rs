use gpui::{
    Bounds, ContentMask, Hsla, IntoElement, MouseButton, PathBuilder, Pixels, TitlebarOptions,
    Window, WindowAppearance, WindowControlArea, canvas, div, fill, point, prelude::*, px, rgb,
    size, transparent_black,
};
use gpui_component::tooltip::Tooltip;
#[cfg(not(test))]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use refract_native::workspace_layout::WORKSPACE_HEADER_HEIGHT;
use std::sync::OnceLock;
use windows::{
    Wdk::System::SystemServices::RtlGetVersion,
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        System::SystemInformation::OSVERSIONINFOW,
        UI::{
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            WindowsAndMessaging::{
                PostMessageW, SC_MAXIMIZE, SC_MINIMIZE, SC_RESTORE, SM_CYFRAME, WM_CLOSE,
                WM_SYSCOMMAND,
            },
        },
    },
};

// The dimensions of the existing Electron Window Controls Overlay on Windows.
const BUTTON_WIDTH: Pixels = px(45.);
const BUTTON_GAP: Pixels = px(1.);
const CONTROLS_WIDTH: Pixels = px(137.);
const SYMBOL_SIZE: Pixels = px(10.);

pub(super) fn titlebar_options() -> TitlebarOptions {
    TitlebarOptions {
        title: Some("Refract".into()),
        appears_transparent: true,
        ..Default::default()
    }
}

pub(super) fn controls_width(window: &Window) -> Pixels {
    if window.is_fullscreen() {
        px(0.)
    } else {
        CONTROLS_WIDTH
    }
}

pub(super) fn overlay(window: &Window, library: bool) -> Option<impl IntoElement> {
    if window.is_fullscreen() {
        return None;
    }
    let dark = matches!(
        window.appearance(),
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    );
    let symbol = rgb(if dark { 0xf1f3f7 } else { 0x27272a }).into();
    let highlight = rgb(if dark { 0xffffff } else { 0x000000 }).into();
    let toggle = if window.is_maximized() {
        CaptionButton::Restore
    } else {
        CaptionButton::Maximize
    };
    let top = if window.is_maximized() { 0. } else { 1. };
    Some(
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(WORKSPACE_HEADER_HEIGHT))
            .flex()
            .justify_end()
            .when(library, |row| {
                row.child(drag_space(
                    px(WORKSPACE_HEADER_HEIGHT),
                    resize_strip(window),
                ))
            })
            .child(
                div()
                    .flex_none()
                    .h_full()
                    .pt(px(top))
                    .flex()
                    .child(CaptionButton::Minimize.control(symbol, highlight))
                    .child(toggle.control(symbol, highlight))
                    .child(CaptionButton::Close.control(symbol, highlight)),
            ),
    )
}

pub(super) fn resize_strip(window: &Window) -> Pixels {
    if window.is_maximized() {
        return px(0.);
    }
    hwnd(window).map_or(px(5.), |handle| {
        // GPUI leaves the top resize frame to Windows when it has no control hitbox.
        let metrics = unsafe { GetSystemMetricsForDpi(SM_CYFRAME, GetDpiForWindow(handle)) };
        px((metrics + 1) as f32 / window.scale_factor())
    })
}

pub(super) fn drag_space(height: Pixels, inset: Pixels) -> impl IntoElement {
    div().flex_1().min_w(px(0.)).relative().h(height).child(
        div()
            .absolute()
            .left_0()
            .right_0()
            .top(inset)
            .bottom_0()
            // Leave the content's focus handler out of native caption events.
            .occlude()
            .window_control_area(WindowControlArea::Drag),
    )
}

#[derive(Clone, Copy)]
enum CaptionButton {
    Minimize,
    Maximize,
    Restore,
    Close,
}

impl CaptionButton {
    fn control(self, symbol: Hsla, highlight: Hsla) -> impl IntoElement {
        let (id, label, area) = match self {
            Self::Minimize => ("window-minimize", "Minimize", WindowControlArea::Min),
            Self::Maximize => ("window-maximize", "Maximize", WindowControlArea::Max),
            Self::Restore => ("window-restore", "Restore", WindowControlArea::Max),
            Self::Close => ("window-close", "Close", WindowControlArea::Close),
        };
        let spacing = if matches!(self, Self::Minimize) {
            px(0.)
        } else {
            BUTTON_GAP
        };
        let (hover, pressed, hover_symbol) = if matches!(self, Self::Close) {
            let close = Hsla::from(rgb(0xe81123));
            (
                close,
                close.opacity(0x98 as f32 / 255.),
                rgb(0xffffff).into(),
            )
        } else {
            (
                highlight.opacity(0x1a as f32 / 255.),
                highlight.opacity(0x33 as f32 / 255.),
                symbol,
            )
        };

        div()
            .id(id)
            .debug_selector(|| id.to_owned())
            .group(id)
            .occlude()
            .window_control_area(area)
            .h_full()
            .w(BUTTON_WIDTH + spacing)
            .flex_none()
            .flex()
            // Nonclient hit testing preserves the system maximize hover. Route
            // client and nonclient clicks through the same native commands.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                self.invoke(window);
            })
            .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
            .child(
                div()
                    .id("face")
                    .h_full()
                    .flex_1()
                    .pl(spacing / 2.)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(symbol)
                    .group_hover(id, move |style| style.bg(hover).text_color(hover_symbol))
                    .group_active(id, move |style| style.bg(pressed).text_color(hover_symbol))
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |bounds, (), window, _| self.paint_symbol(bounds, window),
                        )
                        .size(SYMBOL_SIZE),
                    ),
            )
    }

    fn invoke(self, window: &Window) {
        let Some(handle) = hwnd(window) else { return };
        let (message, command) = match self {
            Self::Minimize => (WM_SYSCOMMAND, SC_MINIMIZE),
            Self::Maximize => (WM_SYSCOMMAND, SC_MAXIMIZE),
            Self::Restore => (WM_SYSCOMMAND, SC_RESTORE),
            // WM_CLOSE goes through the existing on_window_should_close handler.
            Self::Close => (WM_CLOSE, 0),
        };
        let _ = unsafe { PostMessageW(Some(handle), message, WPARAM(command as usize), LPARAM(0)) };
    }

    fn paint_symbol(self, bounds: Bounds<Pixels>, window: &mut Window) {
        let scale = window.scale_factor();
        let snap = |value: Pixels| (value * scale).round() / scale;
        let origin = point(snap(bounds.left()), snap(bounds.top()));
        let extent = snap(SYMBOL_SIZE);
        let stroke = snap(px(1.));
        let color = window.text_style().color;
        let square = |origin, side| Bounds::new(origin, size(side, side));
        let outline = |bounds| {
            fill(bounds, transparent_black())
                .border_widths(stroke)
                .border_color(color)
                .corner_radii(gpui::Corners::all(caption_corner(scale)))
        };
        match self {
            Self::Minimize => window.paint_quad(fill(
                Bounds::new(
                    origin + point(px(0.), snap(extent / 2.)),
                    size(extent, stroke),
                ),
                color,
            )),
            Self::Maximize => window.paint_quad(outline(square(origin, extent))),
            Self::Restore => {
                let shift = snap(px(2.));
                let front = square(origin + point(px(0.), shift), extent - shift);
                let rear = outline(square(origin + point(shift, px(0.)), extent - shift));
                // Expose only the part of the back square outside the front one.
                for bounds in [
                    Bounds::new(origin, size(extent, shift)),
                    Bounds::new(
                        origin + point(extent - shift, shift),
                        size(shift, extent - shift),
                    ),
                ] {
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        window.paint_quad(rear.clone());
                    });
                }
                window.paint_quad(outline(front));
            }
            Self::Close => {
                let inset = stroke / 2.;
                let mut path = PathBuilder::stroke(stroke);
                path.move_to(origin + point(inset, inset));
                path.line_to(origin + point(extent - inset, extent - inset));
                path.move_to(origin + point(extent - inset, inset));
                path.line_to(origin + point(inset, extent - inset));
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            }
        }
    }
}

fn caption_corner(scale: f32) -> Pixels {
    static WINDOWS_11: OnceLock<bool> = OnceLock::new();
    let rounded = *WINDOWS_11.get_or_init(|| {
        let mut version = OSVERSIONINFOW {
            dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
            ..Default::default()
        };
        unsafe { RtlGetVersion(&mut version) }.is_ok() && version.dwBuildNumber >= 22_000
    });
    px(if rounded {
        (1.5 * scale).floor() / scale
    } else {
        0.
    })
}

#[cfg(not(test))]
fn hwnd(window: &Window) -> Option<HWND> {
    let RawWindowHandle::Win32(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    Some(HWND(handle.hwnd.get() as *mut _))
}

#[cfg(test)]
fn hwnd(_: &Window) -> Option<HWND> {
    // GPUI's synthetic test windows have no OS handle.
    None
}
