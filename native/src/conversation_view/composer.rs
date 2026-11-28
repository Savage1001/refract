use super::{ConversationPane, Library, Palette, Provider, attachment_preview, chat_button};
use gpui::{
    App, ClipboardEntry, Context, Entity, EntityInputHandler, ImageFormat, KeyBinding,
    PathPromptOptions, SharedString, Subscription, Task, Window, div, prelude::*, px, rgb, svg,
};
use gpui_component::{
    input::{Enter, Input, InputEvent, InputState, Paste},
    tooltip::Tooltip,
};
use refract_native::conversation::{
    Attachment, ConversationDraft, MAX_ATTACHMENTS, attach_conversation_files,
    attach_conversation_image, save_conversation_draft,
};
use std::time::Duration;

pub(super) struct Composer {
    pub(super) input: Entity<InputState>,
    pub(super) attachments: Vec<Attachment>,
    pub(super) dirty: bool,
    save_timer: Option<Task<()>>,
    _events: Subscription,
}

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "shift-enter",
        Enter { secondary: true },
        Some("RefractChat > Input"),
    )]);
}

impl Composer {
    pub(super) fn new(
        draft: ConversationDraft,
        window: &mut Window,
        cx: &mut Context<Library>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(1, 6)
                .placeholder("Ask anything")
                .default_value(draft.text)
        });
        let events = cx.subscribe(&input, |library, input, event: &InputEvent, cx| {
            let is_current = library
                .selection
                .as_ref()
                .and_then(|selection| selection.conversation.composer.as_ref())
                .is_some_and(|composer| composer.input == input);
            if is_current && matches!(event, InputEvent::Change) {
                library.schedule_chat_draft_save(cx);
                cx.notify();
            }
        });
        Self {
            input,
            attachments: draft.attachments,
            dirty: false,
            save_timer: None,
            _events: events,
        }
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &mut Context<Library>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    pub(super) fn draft(&self, provider: Option<Provider>, cx: &App) -> ConversationDraft {
        ConversationDraft {
            text: self.input.read(cx).value().to_string(),
            attachments: self.attachments.clone(),
            provider: provider.map(|provider| provider.as_str().into()),
        }
    }
}

impl Library {
    pub(super) fn schedule_chat_draft_save(&mut self, cx: &mut Context<Self>) {
        let Some(composer) = self
            .selection
            .as_mut()
            .and_then(|selection| selection.conversation.composer.as_mut())
        else {
            return;
        };
        composer.dirty = true;
        let delay = cx.background_executor().timer(Duration::from_millis(350));
        composer.save_timer = Some(cx.spawn(async move |library, cx| {
            delay.await;
            let _ = library.update(cx, |library, cx| library.flush_chat_draft(cx));
        }));
    }

    pub(crate) fn flush_chat_draft(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(selection) = &mut self.selection else {
            return true;
        };
        let pane = &mut selection.conversation;
        let Some(composer) = &mut pane.composer else {
            return true;
        };
        composer.save_timer = None;
        if !composer.dirty {
            return true;
        }
        let mut draft = composer.draft(pane.provider, cx);
        if let Some(pending) = pane.running.as_ref().and_then(|turn| turn.pending.clone()) {
            pending.prepend_to(&mut draft);
        }
        match save_conversation_draft(&self.projects_dir, &selection.project, &pane.active, &draft)
        {
            Ok(()) => {
                composer.dirty = false;
                true
            }
            Err(error) => {
                pane.error = Some(format!("Couldn’t save chat draft. {error}"));
                cx.notify();
                false
            }
        }
    }

    fn attach_to_conversation(
        &mut self,
        screenshot: Option<Vec<u8>>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if !self.project_writes_ready() || self.navigation_after_save.is_some() {
            return;
        }
        let Some(selection) = &mut self.selection else {
            return;
        };
        let pane = &mut selection.conversation;
        let Some(composer) = &pane.composer else {
            return;
        };
        let remaining = MAX_ATTACHMENTS.saturating_sub(composer.attachments.len());
        if remaining == 0 || pane.load_task.is_some() {
            return;
        }
        pane.error = None;
        let sequence = self.project_open_sequence;
        let id = pane.active.clone();
        let project = selection.project.clone();
        let path = self.projects_dir.clone();
        let executor = cx.background_executor().clone();
        let picker = screenshot.is_none().then(|| {
            cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: true,
                prompt: Some("Upload files & photos".into()),
            })
        });
        self.entry_write_task = Some(cx.spawn_in(window, async move |library, cx| {
            let result = if let Some(image) = screenshot {
                executor
                    .spawn(async move {
                        attach_conversation_image(&path, &project, &image).map(|image| vec![image])
                    })
                    .await
                    .map_err(|error| error.to_string())
            } else {
                match picker
                    .expect("an attachment picker when no image was pasted")
                    .await
                {
                    Ok(Ok(Some(files))) => executor
                        .spawn(async move {
                            attach_conversation_files(&path, &project, &files, remaining)
                        })
                        .await
                        .map_err(|error| error.to_string()),
                    Ok(Err(error)) => Err(error.to_string()),
                    _ => Ok(Vec::new()),
                }
            };
            let _ = library.update_in(cx, |library, window, cx| {
                library.entry_write_task = None;
                if library.project_open_sequence == sequence
                    && let Some(selection) = &mut library.selection
                    && selection.conversation.active == id
                {
                    let pane = &mut selection.conversation;
                    match result {
                        Ok(files) if !files.is_empty() => {
                            if let Some(composer) = &mut pane.composer {
                                composer.attachments.extend(files);
                                composer.dirty = true;
                                library.flush_chat_draft(cx);
                            }
                        }
                        Ok(_) => {}
                        Err(error) => pane.error = Some(format!("Couldn’t attach files. {error}")),
                    }
                }
                library.start_next_save(window, cx);
                if library.snapshot_timer.is_none() {
                    library.start_snapshot(window, cx);
                }
                library.resume_navigation(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn remove_draft_attachment(
        &mut self,
        token: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(composer) = self
            .selection
            .as_mut()
            .and_then(|selection| selection.conversation.composer.as_mut())
        else {
            return;
        };
        composer.attachments.retain(|item| item.token != token);
        composer.dirty = true;
        composer.focus(window, cx);
        self.flush_chat_draft(cx);
        cx.notify();
    }

    pub(super) fn conversation_composer(
        &self,
        pane: &ConversationPane,
        composer: &Composer,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let remaining = composer.attachments.len() < MAX_ATTACHMENTS;
        let disabled = pane.load_task.is_some() || self.entry_write_task.is_some() || !remaining;
        let can_send = !composer.input.read(cx).value().trim().is_empty();
        let tooltip = if remaining {
            "Upload files & photos"
        } else {
            "Maximum of 10 attachments"
        };
        div()
            .id("chat-composer")
            .flex()
            .flex_col()
            .flex_none()
            .p_2()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(palette.input_border))
            .when(!composer.attachments.is_empty(), |form| {
                form.child(
                    div().flex().flex_wrap().gap_2().children(composer.attachments.iter().map(
                        |item| {
                            let token = item.token.clone();
                            let tooltip = format!("Remove {}", item.name);
                            div()
                                .id(SharedString::from(item.token.clone()))
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(attachment_preview(item, palette, cx))
                                .child(
                                    chat_button("remove-attachment", palette, cx, move |library, window, cx| {
                                        library.remove_draft_attachment(&token, window, cx)
                                    })
                                    .p_1()
                                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                                    .child(svg().path("refract/close.svg").size(px(14.)).text_color(rgb(palette.muted))),
                                )
                        },
                    )),
                )
            })
            .child(
                div()
                    .id("chat-input")
                    .debug_selector(|| "chat-input".into())
                    .key_context("RefractChat")
                    .capture_action({
                        let input = composer.input.clone();
                        cx.listener(move |library, action: &Enter, window, cx| {
                            if input.update(cx, |input, cx| {
                                input.marked_text_range(window, cx).is_some()
                            }) {
                                cx.stop_propagation();
                                return;
                            }
                            if !action.secondary {
                                cx.stop_propagation();
                                library.send_assistant_turn(window, cx);
                            }
                        })
                    })
                    .capture_action(cx.listener(|library, _: &Paste, window, cx| {
                        if let Some(item) = cx.read_from_clipboard()
                            && let Some(ClipboardEntry::Image(image)) = item.entries().iter().find(
                                |entry| matches!(entry, ClipboardEntry::Image(image) if image.format == ImageFormat::Png)
                            )
                        {
                            cx.stop_propagation();
                            library.attach_to_conversation(Some(image.bytes.clone()), window, cx);
                        }
                    }))
                    .child(
                        Input::new(&composer.input)
                            .appearance(false)
                            .bordered(false)
                            .min_h(px(36.))
                            .max_h(px(120.))
                            .px(px(4.))
                            .py(px(4.))
                            .text_size(px(13.))
                            .line_height(gpui::relative(1.5)),
                    ),
            )
            .child(
                div().flex().min_h(px(36.)).items_center().child(
                    chat_button("attach", palette, cx, move |library, window, cx| {
                        if !disabled {
                            library.attach_to_conversation(None, window, cx);
                        }
                    })
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .tab_stop(!disabled)
                    .when(disabled, |button| button.cursor_default().opacity(0.5))
                    .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
                    .child(svg().path("refract/attach.svg").size(px(18.)).text_color(rgb(palette.muted))),
                )
                .child(div().flex_1())
                .child(self.assistant_controls(pane, can_send, palette, cx)),
            )
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::{Root, initialize_app};
    use gpui::{ClipboardItem, Image, Modifiers, TestAppContext};
    use refract_native::{
        conversation::{ConversationId, load_conversation_draft},
        create_project,
    };
    use std::io::Cursor;

    #[gpui::test]
    fn pastes_and_removes_a_screenshot_without_losing_draft_text(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
        cx.update(initialize_app);
        let mut library = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Library::new(projects.clone(), window, cx));
            library = Some(view.clone());
            Root::new(view, window, cx)
        });
        let library = library.unwrap();
        cx.update(|window, app| {
            library.update(app, |view, cx| {
                view.open_project(project.clone(), window, cx);
            });
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        let input = cx.debug_bounds("chat-input").unwrap().center();
        cx.simulate_click(input, Modifiers::none());
        cx.simulate_input("Continue the draft");
        let mut bytes = Cursor::new(Vec::new());
        image::RgbaImage::new(2, 2)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let image = Image::from_bytes(ImageFormat::Png, bytes.into_inner());
        cx.write_to_clipboard(ClipboardItem::new_image(&image));
        cx.update(|window, app| window.dispatch_action(Box::new(Paste), app));
        cx.run_until_parked();
        cx.refresh().unwrap();
        let id = ConversationId::default();
        let draft = load_conversation_draft(&projects, &project, &id).unwrap();
        assert_eq!(draft.text, "Continue the draft");
        assert_eq!(draft.attachments[0].name, "Pasted screenshot.png");

        let remove = cx.debug_bounds("remove-attachment").unwrap().center();
        cx.simulate_click(remove, Modifiers::none());
        let draft = load_conversation_draft(&projects, &project, &id).unwrap();
        assert_eq!(draft.text, "Continue the draft");
        assert!(draft.attachments.is_empty());
    }
}
