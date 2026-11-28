use super::{Library, Palette, PdfViewport, Zoom};
use gpui::{
    App, Context, Entity, Focusable, KeyBinding, KeyDownEvent, Subscription, Window, actions, div,
    prelude::*, px, rgb, svg,
};
use gpui_component::{
    input::{Input, InputEvent, InputState, SelectAll},
    tooltip::Tooltip,
};

actions!(refract_pdf, [SubmitPage, CancelPage]);

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", SubmitPage, Some("RefractPdfPage > Input")),
        KeyBinding::new("escape", CancelPage, Some("RefractPdfPage > Input")),
    ]);
}

pub(super) struct PageEdit {
    input: Entity<InputState>,
    invalid: bool,
    _events: Subscription,
}

impl Library {
    fn change_pdf_page(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        let preview = &mut selection.build.preview;
        if preview.layout.pages.is_empty() {
            return;
        }
        let index = preview
            .page
            .saturating_add_signed(delta)
            .min(preview.layout.pages.len().saturating_sub(1) as u32);
        preview.go_to_page(index, window);
        self.start_pdf_frame(window, cx);
        cx.notify();
    }

    fn change_pdf_zoom(&mut self, zoom: Zoom, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(selection) = &mut self.selection {
            selection.build.preview.resize_pages(zoom, window);
            self.start_pdf_frame(window, cx);
            cx.notify();
        }
    }

    fn fit_pdf_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(preview) = self
            .selection
            .as_mut()
            .map(|selection| &mut selection.build.preview)
        else {
            return;
        };
        if let Some(zoom) = preview.fit_page_zoom() {
            let page = preview.page;
            preview.resize_pages(zoom, window);
            preview.go_to_page(page, window);
            self.start_pdf_frame(window, cx);
            cx.notify();
        }
    }

    fn edit_pdf_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(preview) = self
            .selection
            .as_mut()
            .map(|selection| &mut selection.build.preview)
        else {
            return;
        };
        let value = (preview.page + 1).to_string();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
        let events = cx.subscribe_in(&input, window, |library, input, event, _, cx| {
            let Some(preview) = library
                .selection
                .as_mut()
                .map(|selection| &mut selection.build.preview)
            else {
                return;
            };
            let Some(edit) = preview
                .page_edit
                .as_mut()
                .filter(|edit| &edit.input == input)
            else {
                return;
            };
            match event {
                InputEvent::Blur => preview.page_edit = None,
                InputEvent::Change => edit.invalid = false,
                _ => return,
            }
            cx.notify();
        });
        preview.page_edit = Some(PageEdit {
            input: input.clone(),
            invalid: false,
            _events: events,
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let focus = input.read(cx).focus_handle(cx);
        window.defer(cx, move |window, app| {
            if focus.is_focused(window) {
                window.draw(app).clear();
                focus.dispatch_action(&SelectAll, window, app);
            }
        });
        cx.notify();
    }

    fn submit_pdf_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(preview) = self
            .selection
            .as_mut()
            .map(|selection| &mut selection.build.preview)
        else {
            return;
        };
        let Some(edit) = &mut preview.page_edit else {
            return;
        };
        let value = edit.input.read(cx).value();
        let current = value.trim();
        let index = current
            .bytes()
            .all(|byte| byte.is_ascii_digit())
            .then_some(current)
            .and_then(|value| value.parse::<u32>().ok())
            .and_then(|page| page.checked_sub(1))
            .filter(|&index| (index as usize) < preview.layout.pages.len());
        if let Some(index) = index {
            preview.go_to_page(index, window);
            self.close_pdf_page_edit(window, cx);
            self.start_pdf_frame(window, cx);
        } else {
            edit.invalid = true;
            edit.input
                .read(cx)
                .focus_handle(cx)
                .dispatch_action(&SelectAll, window, cx);
            cx.notify();
        }
    }

    fn close_pdf_page_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(selection) = &mut self.selection {
            let preview = &mut selection.build.preview;
            preview.page_edit = None;
            if let Some(focus) = &preview.page_focus {
                window.focus(focus);
            }
            cx.notify();
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

    pub(crate) fn pdf_controls(
        &self,
        view: &PdfViewport,
        pane_width: f32,
        palette: Palette,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        let count = view.layout.pages.len() as u32;
        if count == 0 {
            return None;
        }
        let narrow = pane_width <= 650.;
        let show_labels = pane_width > 575.;
        let group_margin = if narrow { 6. } else { 12. };
        let zoom = view.zoom.percentage();
        let label = if view.zoom == Zoom::FitWidth {
            "Fit".to_owned()
        } else {
            format!("{zoom}%")
        };
        let zoom_readout_width = if view.zoom == Zoom::FitWidth {
            22.
        } else {
            34.
        };
        let zoom_group_width =
            28. * 2. + 6. * 2. + zoom_readout_width + if show_labels { 36. } else { 0. };
        Some(
            div()
                .flex()
                .flex_none()
                .flex_wrap()
                .items_center()
                .max_w_full()
                .gap(px(6.))
                .mx(px(if narrow { 2. } else { 8. }))
                .text_size(px(12.))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(6.))
                        .when(show_labels, |group| group.child(pdf_label("Page", palette)))
                        .child(
                            pdf_button(
                                "pdf-prev",
                                "Previous page",
                                view.page > 0,
                                palette,
                                cx,
                                |library, window, cx| library.change_pdf_page(-1, window, cx),
                            )
                            .child(pdf_icon("refract/chevron-left.svg", palette)),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .min_w(px(40.))
                                .child(self.pdf_page_number(view, palette, cx))
                                .child(format!(" / {count}")),
                        )
                        .child(
                            pdf_button(
                                "pdf-next",
                                "Next page",
                                view.page < count - 1,
                                palette,
                                cx,
                                |library, window, cx| library.change_pdf_page(1, window, cx),
                            )
                            .child(pdf_icon("refract/chevron-right.svg", palette)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .w(px(zoom_group_width))
                        .items_center()
                        .gap(px(6.))
                        .ml(px(group_margin))
                        .when(show_labels, |group| group.child(pdf_label("Zoom", palette)))
                        .child(
                            pdf_button(
                                "pdf-zoom-out",
                                "Zoom out",
                                zoom > 25,
                                palette,
                                cx,
                                |library, window, cx| library.step_pdf_zoom(-25, window, cx),
                            )
                            .child(pdf_icon("refract/minus.svg", palette)),
                        )
                        .child(
                            div()
                                .flex_none()
                                .w(px(zoom_readout_width))
                                .text_center()
                                .child(label),
                        )
                        .child(
                            pdf_button(
                                "pdf-zoom-in",
                                "Zoom in",
                                zoom < 250,
                                palette,
                                cx,
                                |library, window, cx| library.step_pdf_zoom(25, window, cx),
                            )
                            .child(pdf_icon("refract/plus.svg", palette)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(6.))
                        .ml(px(group_margin))
                        .child(fit_button(
                            pdf_button(
                                "pdf-fit-width",
                                "Fit PDF to width",
                                true,
                                palette,
                                cx,
                                |library, window, cx| {
                                    library.change_pdf_zoom(Zoom::FitWidth, window, cx)
                                },
                            ),
                            "Fit width",
                            view.zoom == Zoom::FitWidth,
                            palette,
                        ))
                        .child(fit_button(
                            pdf_button(
                                "pdf-fit-page",
                                "Fit PDF page to view",
                                true,
                                palette,
                                cx,
                                Self::fit_pdf_page,
                            ),
                            "Fit page",
                            matches!(view.zoom, Zoom::FitPage(_)),
                            palette,
                        )),
                ),
        )
    }

    fn pdf_page_number(
        &self,
        view: &PdfViewport,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        if let Some(edit) = &view.page_edit {
            return div()
                .key_context("RefractPdfPage")
                .on_action(cx.listener(|view, _: &SubmitPage, window, cx| {
                    view.submit_pdf_page(window, cx);
                }))
                .on_action(cx.listener(|view, _: &CancelPage, window, cx| {
                    view.close_pdf_page_edit(window, cx);
                }))
                .rounded_sm()
                .border_1()
                .border_color(rgb(if edit.invalid {
                    palette.warning
                } else {
                    palette.focus
                }))
                .child(
                    Input::new(&edit.input)
                        .appearance(false)
                        .tab_index(5)
                        .h(px(24.))
                        .w(px(36.)),
                )
                .into_any_element();
        }
        pdf_button(
            "pdf-current-page",
            "Go to page",
            true,
            palette,
            cx,
            Self::edit_pdf_page,
        )
        .min_w(px(28.))
        .text_center()
        .text_size(px(12.))
        .child((view.page + 1).to_string())
        .when_some(view.page_focus.as_ref(), |button, focus| {
            button.track_focus(focus)
        })
        .into_any_element()
    }
}

fn pdf_button(
    id: &'static str,
    title: &'static str,
    enabled: bool,
    palette: Palette,
    cx: &Context<Library>,
    activate: fn(&mut Library, &mut Window, &mut Context<Library>),
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(|| id.to_owned())
        .min_w(px(28.))
        .min_h(px(28.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .px(px(5.))
        .py(px(3.))
        .rounded(px(6.))
        .tooltip(move |window, cx| Tooltip::new(title).build(window, cx))
        .when(!enabled, |button| button.opacity(0.45))
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

fn pdf_label(label: &'static str, palette: Palette) -> impl IntoElement {
    div()
        .text_size(px(11.))
        .text_color(rgb(palette.muted))
        .child(label)
}

fn pdf_icon(path: &'static str, palette: Palette) -> impl IntoElement {
    svg()
        .path(path)
        .size(px(18.))
        .text_color(rgb(palette.muted))
}

fn fit_button(
    button: gpui::Stateful<gpui::Div>,
    label: &'static str,
    active: bool,
    palette: Palette,
) -> impl IntoElement {
    button
        .px(px(6.))
        .text_size(px(11.))
        .border_1()
        .border_color(if active {
            rgb(palette.input_border)
        } else {
            gpui::rgba(0)
        })
        .text_color(rgb(if active { palette.text } else { palette.muted }))
        .when(active, |button| button.bg(rgb(palette.selection)))
        .child(label)
}

#[cfg(all(test, feature = "ui-tests", windows))]
mod tests {
    use super::*;
    use crate::initialize_app;
    use gpui::{Modifiers, TestAppContext};
    use gpui_component::Root;
    use refract_native::{Project, create_project, tex::BuildResult};
    use std::{fs, io, io::Write, path::Path, sync::atomic::AtomicBool};

    fn fixture_compile(
        projects: &Path,
        project: &Project,
        _: &AtomicBool,
        _: bool,
    ) -> io::Result<BuildResult> {
        let path = projects.join(&project.id).join(".refract-build/main.pdf");
        let mut bytes = b"%PDF-1.4\n".to_vec();
        let page = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 900] /Resources << >> /Contents 6 0 R >>";
        let stream = "0.2 0.4 0.7 rg\n10 30 170 100 re f\n";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>".into(),
            page.into(),
            page.into(),
            page.into(),
            format!("<< /Length {} >>\nstream\n{stream}endstream", stream.len()),
        ];
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            writeln!(bytes, "{} 0 obj\n{object}\nendobj", index + 1)?;
        }
        let xref = bytes.len();
        write!(bytes, "xref\n0 7\n0000000000 65535 f \n")?;
        for offset in offsets {
            writeln!(bytes, "{offset:010} 00000 n ")?;
        }
        write!(
            bytes,
            "trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
        )?;
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, bytes)?;
        Ok(BuildResult {
            cached: false,
            arguments: Vec::new(),
            exit_code: Some(0),
            log: String::new(),
            pdf: Some(path),
        })
    }

    #[gpui::test]
    fn page_entry_validates_and_restores_keyboard_focus(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
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
                view.test_compiler = Some(fixture_compile);
                view.open_project(project, window, cx);
            });
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        let zoom = cx.debug_bounds("pdf-zoom-in").unwrap();
        let fit = cx.debug_bounds("pdf-fit-width").unwrap();
        let same_row = zoom.origin.y < fit.origin.y + fit.size.height
            && fit.origin.y < zoom.origin.y + zoom.size.height;
        assert!(!same_row || zoom.origin.x + zoom.size.width <= fit.origin.x);
        cx.update(|_, app| {
            let build = &library.read(app).selection.as_ref().unwrap().build;
            assert_eq!(
                build.preview.layout.pages.len(),
                3,
                "PDF error: {:?}, build output: {:?}",
                build.preview.error,
                build.log()
            );
        });
        let page_button = cx.debug_bounds("pdf-current-page").unwrap().center();
        cx.simulate_click(page_button, Modifiers::none());
        cx.simulate_input("0");
        cx.simulate_keystrokes("enter");
        cx.update(|_, app| {
            let preview = &library.read(app).selection.as_ref().unwrap().build.preview;
            assert_eq!(preview.page, 0);
            let edit = preview.page_edit.as_ref().unwrap();
            assert!(edit.invalid);
            assert_eq!(edit.input.read(app).value().as_ref(), "0");
        });
        cx.simulate_input("3");
        cx.simulate_keystrokes("enter");
        cx.update(|window, app| {
            let preview = &library.read(app).selection.as_ref().unwrap().build.preview;
            assert_eq!(preview.page, 2);
            assert!(preview.page_edit.is_none());
            assert!(preview.page_focus.as_ref().unwrap().is_focused(window));
        });
        cx.simulate_keystrokes("enter");
        cx.update(|_, app| {
            assert!(
                library
                    .read(app)
                    .selection
                    .as_ref()
                    .unwrap()
                    .build
                    .preview
                    .page_edit
                    .is_some()
            );
        });
        cx.simulate_keystrokes("escape");
        cx.update(|window, app| {
            let preview = &library.read(app).selection.as_ref().unwrap().build.preview;
            assert_eq!(preview.page, 2);
            assert!(preview.page_edit.is_none());
            assert!(preview.page_focus.as_ref().unwrap().is_focused(window));
        });
    }
}
