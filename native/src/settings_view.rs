use super::{Library, Navigation, Palette, deleted_projects_view::recovery_button};
use crate::account_settings_view::{AccountControls, AccountSection};
use gpui::{
    AnyElement, Context, Entity, FocusHandle, FontWeight, KeyDownEvent, Pixels, SharedString,
    Subscription, Task, WeakEntity, Window, div, prelude::*, px, rgb, svg,
};
use gpui_component::{
    WindowExt,
    select::{Select, SelectDelegate, SelectEvent, SelectItem, SelectState},
    tooltip::Tooltip,
};
use refract_native::updates::{
    CHECK_INTERVAL, PreparedUpdate, UpdateTrack, WindowsUpdater, installed_version,
};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const DISABLED: &str = "Automatic updates are available in the installed Windows app.";
const TRACK_DESCRIPTION: &str = "Stable receives stable releases. Pre-release receives the newest pre-release or stable version. Updates are checked every 10 minutes and download automatically. Install them when you choose Restart to update.";
const DIALOG_PADDING: Pixels = px(28.);
const TITLE_GAP: Pixels = px(24.);

#[derive(Clone)]
struct TrackChoice(UpdateTrack);

impl SelectItem for TrackChoice {
    type Value = UpdateTrack;

    fn title(&self) -> SharedString {
        match self.0 {
            UpdateTrack::Stable => "Stable",
            UpdateTrack::Beta => "Pre-release",
        }
        .into()
    }

    fn value(&self) -> &UpdateTrack {
        &self.0
    }
}

pub(super) struct SettingsView {
    profile: PathBuf,
    pub(super) account: AccountControls,
    track: UpdateTrack,
    track_select: Entity<SelectState<Vec<TrackChoice>>>,
    _track_subscription: Subscription,
    open_focus: FocusHandle,
    close_focus: FocusHandle,
    check_focus: FocusHandle,
    restart_focus: FocusHandle,
    install_version: String,
    message: String,
    error: Option<String>,
    downloading: Option<u8>,
    updater: Option<WindowsUpdater>,
    prepared: Option<PreparedUpdate>,
    work: Option<UpdateWork>,
    save: Option<Task<()>>,
    dialog: Option<Rc<RefCell<SettingsDialog>>>,
}

struct SettingsDialog {
    account: AccountSection,
    track_select: Entity<SelectState<Vec<TrackChoice>>>,
    close_focus: FocusHandle,
    check_focus: FocusHandle,
    restart_focus: FocusHandle,
    status: String,
    error: Option<String>,
    downloading: Option<u8>,
    track_saving: bool,
    check_available: bool,
    restart_visible: bool,
    restart_available: bool,
}

struct UpdateWork {
    cancel: Arc<AtomicBool>,
    _task: Task<()>,
}

impl Drop for UpdateWork {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl SettingsView {
    pub(super) fn new(projects_dir: &Path, window: &mut Window, cx: &mut Context<Library>) -> Self {
        let profile = projects_dir.parent().unwrap_or(projects_dir).to_path_buf();
        let track = UpdateTrack::load(&profile);
        let selector = cx.new(|cx| {
            let options = vec![
                TrackChoice(UpdateTrack::Stable),
                TrackChoice(UpdateTrack::Beta),
            ];
            let mut state = SelectState::new(options, None, window, cx);
            state.set_selected_value(&track, window, cx);
            state
        });
        let subscription = cx.subscribe_in(&selector, window, |library, _, event, window, cx| {
            if let SelectEvent::Confirm(Some(track)) = event {
                library.change_update_track(*track, window, cx);
            }
        });
        let updater = WindowsUpdater::installed(&profile);
        let account = AccountControls::new(&profile, window, cx);
        let message = if updater.is_some() {
            "Updates are checked every 10 minutes."
        } else {
            DISABLED
        };
        Self {
            profile,
            account,
            track,
            track_select: selector,
            _track_subscription: subscription,
            open_focus: cx.focus_handle().tab_stop(true).tab_index(1),
            close_focus: cx.focus_handle().tab_stop(true),
            check_focus: cx.focus_handle().tab_stop(true),
            restart_focus: cx.focus_handle().tab_stop(true),
            install_version: installed_version().to_string(),
            message: message.to_owned(),
            error: None,
            downloading: None,
            updater,
            prepared: None,
            work: None,
            save: None,
            dialog: None,
        }
    }

    fn dialog_contents(&self) -> SettingsDialog {
        let busy = self.save.is_some() || self.work.is_some();
        SettingsDialog {
            account: self.account.section(),
            track_select: self.track_select.clone(),
            close_focus: self.close_focus.clone(),
            check_focus: self.check_focus.clone(),
            restart_focus: self.restart_focus.clone(),
            status: format!("Installed: {}. {}", self.install_version, self.message),
            error: self.error.clone(),
            downloading: self.downloading,
            track_saving: self.save.is_some(),
            check_available: !busy,
            restart_visible: self.prepared.is_some(),
            restart_available: self.prepared.is_some() && !busy,
        }
    }

    pub(super) fn refresh_dialog(&self) {
        if let Some(dialog) = &self.dialog {
            *dialog.borrow_mut() = self.dialog_contents();
        }
    }

    pub(super) fn dialog_open(&self) -> bool {
        self.dialog.is_some()
    }

    pub(super) fn prepare_exit(&self) {
        self.account.cancel_setup();
        if let Some(work) = &self.work {
            work.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub(super) fn ready_to_exit(&self) -> bool {
        self.work.is_none() && self.save.is_none() && !self.account.busy()
    }
}

impl SettingsDialog {
    fn contents(&self, library: WeakEntity<Library>, palette: Palette) -> AnyElement {
        let check = library.clone();
        let restart = library.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .debug_selector(|| "settings-dialog".into())
            .child(self.account.contents(library.clone(), palette))
            .child(
                div()
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .font_weight(FontWeight::MEDIUM)
                    .child("Update track"),
            )
            .child(settings_select(
                &self.track_select,
                palette,
                self.track_saving,
            ))
            .child(
                div()
                    .mt(px(4.))
                    .text_size(px(13.))
                    .line_height(px(20.8))
                    .text_color(rgb(palette.muted))
                    .child(TRACK_DESCRIPTION),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .line_height(px(20.8))
                    .text_color(rgb(palette.muted))
                    .child(self.status.clone()),
            )
            .when_some(self.error.clone(), |view, error| {
                view.child(div().text_color(rgb(palette.warning)).child(error))
            })
            .when_some(self.downloading, |view, percent| {
                view.child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .h(px(6.))
                                .rounded_md()
                                .bg(rgb(palette.selection))
                                .child(
                                    div()
                                        .h_full()
                                        .w(gpui::relative(f32::from(percent) / 100.))
                                        .rounded_md()
                                        .bg(rgb(palette.focus)),
                                ),
                        )
                        .child(div().text_size(px(13.)).child(format!("{percent}%"))),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap(px(6.))
                    .mt(px(16.))
                    .pt(px(16.))
                    .border_t_1()
                    .border_color(rgb(palette.border))
                    .when(self.restart_visible, |actions| {
                        actions.child(settings_action(
                            "restart-update",
                            "Restart to update",
                            &self.restart_focus,
                            palette,
                            self.restart_available,
                            move |window, app| {
                                let _ = restart.update(app, |library, cx| {
                                    library.restart_for_update(window, cx)
                                });
                            },
                        ))
                    })
                    .child(settings_action(
                        "check-updates",
                        "Check for updates",
                        &self.check_focus,
                        palette,
                        self.check_available,
                        move |window, app| {
                            let _ =
                                check.update(app, |library, cx| library.check_updates(window, cx));
                        },
                    ))
                    .child(settings_action(
                        "close-settings",
                        "Close",
                        &self.close_focus,
                        palette,
                        true,
                        move |window, app| {
                            let _ = library.update(app, |library, cx| {
                                window.close_dialog(cx);
                                library.settings_closed(window, cx);
                            });
                        },
                    )),
            )
            .into_any_element()
    }
}

pub(super) fn settings_select<D: SelectDelegate + 'static>(
    state: &Entity<SelectState<D>>,
    palette: Palette,
    disabled: bool,
) -> impl IntoElement {
    div().w_full().h(px(36.)).flex_none().child(
        Select::new(state)
            .w_full()
            .h_full()
            .text_size(px(13.))
            .text_color(rgb(palette.text))
            .bg(rgb(palette.surface))
            .border_color(rgb(palette.input_border))
            .disabled(disabled),
    )
}

fn settings_action(
    id: &'static str,
    label: &'static str,
    focus: &FocusHandle,
    palette: Palette,
    enabled: bool,
    activate: impl Fn(&mut Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    recovery_button(id, label, focus, palette, enabled, activate)
        .flex()
        .items_center()
        .justify_center()
        .min_w(px(76.))
        .h(px(36.))
        .text_size(px(12.))
        .line_height(px(18.))
        .border_color(rgb(palette.input_border))
}

impl Library {
    pub(super) fn settings_button(&self, palette: Palette, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id("settings")
            .debug_selector(|| "settings".into())
            .track_focus(&self.settings.open_focus)
            .size(px(36.))
            .rounded_md()
            .flex()
            .items_center()
            .justify_center()
            .text_color(rgb(palette.text))
            .cursor_pointer()
            .hover(move |style| style.bg(rgb(palette.selection)))
            .focus(move |style| style.bg(rgb(palette.selection)))
            .tooltip(|window, cx| Tooltip::new("Settings").build(window, cx))
            .child(
                svg()
                    .path("refract/settings.svg")
                    .size(px(20.))
                    .text_color(rgb(palette.text)),
            )
            .on_click(cx.listener(|library, _, window, cx| library.open_settings(window, cx)))
            .on_key_down(cx.listener(|library, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    library.open_settings(window, cx);
                }
            }))
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_some() || window.has_active_dialog(cx) {
            return;
        }
        self.dismiss_project_menu(cx);
        self.refresh_account_settings(window, cx);
        let owner = cx.weak_entity();
        let contents = Rc::new(RefCell::new(self.settings.dialog_contents()));
        self.settings.dialog = Some(contents.clone());
        let measured_height = Rc::new(Cell::new(None::<Pixels>));
        window.open_dialog(cx, move |dialog, window, _| {
            let close = owner.clone();
            let viewport = window.viewport_size();
            let max_height = viewport.height * 0.85;
            let dialog_chrome = DIALOG_PADDING * 2. + px(27.) + TITLE_GAP + px(2.);
            let height = measured_height
                .get()
                .map_or(px(480.), |content_height| content_height + dialog_chrome)
                .min(max_height);
            let width = (viewport.width - px(48.)).max(px(0.)).min(px(440.));
            let measurement = measured_height.clone();
            let measure_owner = owner.clone();
            let mut palette = Palette::for_window(window);
            if matches!(
                window.appearance(),
                gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
            ) {
                palette.muted = 0x818181;
            }
            dialog
                .title(crate::dialog_title("Settings"))
                .w(width)
                .max_h(max_height)
                .margin_top((viewport.height - height) / 2.)
                .p(DIALOG_PADDING)
                .gap(TITLE_GAP)
                .rounded(px(12.))
                .bg(rgb(palette.surface))
                .border_color(rgb(palette.input_border))
                .close_button(false)
                .overlay(false)
                .overlay_closable(false)
                .child(
                    div()
                        .flex_none()
                        .on_children_prepainted(move |bounds, window, app| {
                            if let Some(content) = bounds.first()
                                && measurement.replace(Some(content.size.height))
                                    != Some(content.size.height)
                            {
                                let measure_owner = measure_owner.clone();
                                window.defer(app, move |window, app| {
                                    let _ = measure_owner.update(app, |_, cx| cx.notify());
                                    window.refresh();
                                });
                            }
                        })
                        .child(contents.borrow().contents(owner.clone(), palette)),
                )
                .on_close(move |_, window, app| {
                    let _ = close.update(app, |library, cx| library.settings_closed(window, cx));
                })
        });
        window.focus(&self.settings.account.focus_handle(cx));
    }

    fn settings_closed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.dialog = None;
        window.focus(&self.settings.open_focus);
        cx.notify();
    }

    fn change_update_track(&mut self, track: UpdateTrack, window: &Window, cx: &mut Context<Self>) {
        if self.preparing_exit() || track == self.settings.track || self.settings.save.is_some() {
            return;
        }
        let profile = self.settings.profile.clone();
        let operation = cx
            .background_executor()
            .spawn(async move { track.save(&profile) });
        self.settings.error = None;
        self.settings.save = Some(cx.spawn_in(window, async move |library, cx| {
            let result = operation.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.settings.save = None;
                match result {
                    Ok(()) => {
                        library.settings.track = track;
                        library.settings.prepared = None;
                        library.settings.work = None;
                        library.check_updates(window, cx);
                    }
                    Err(error) => {
                        library.settings.error =
                            Some(format!("Couldn’t save update track. {error}"))
                    }
                }
                let selected = library.settings.track;
                library.settings.track_select.update(cx, |state, cx| {
                    state.set_selected_value(&selected, window, cx);
                });
                library.resume_navigation(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn start_update_checks(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.settings.updater.is_none() {
            return;
        }
        self.check_updates(window, cx);
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |library, cx| {
            loop {
                executor.timer(CHECK_INTERVAL).await;
                if library
                    .update_in(cx, |library, window, cx| library.check_updates(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn check_updates(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.preparing_exit() || self.settings.work.is_some() || self.settings.save.is_some() {
            return;
        }
        let Some(updater) = self.settings.updater.clone() else {
            return;
        };
        self.settings.message = "Checking GitHub releases…".into();
        self.settings.downloading = None;
        let track = self.settings.track;
        let prepared = self.settings.prepared.take();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (progress, updates) = async_channel::bounded(102);
        let operation = cx.background_executor().spawn(async move {
            let Some(candidate) = updater.check(track, &worker_cancel)? else {
                return Ok(None);
            };
            if prepared
                .as_ref()
                .is_some_and(|prepared| prepared.candidate() == &candidate)
            {
                return Ok(prepared);
            }
            let name = candidate.version().to_string();
            let size = candidate.size() as f64;
            let size = if size >= 1024. * 1024. {
                format!("{:.1} MiB", size / (1024. * 1024.))
            } else {
                format!("{:.1} KiB", size / 1024.)
            };
            let _ = progress.try_send((Some(format!("Downloading {name} ({size})…")), 0));
            updater
                .download(candidate, &worker_cancel, |percent| {
                    let _ = progress.try_send((None, percent));
                })
                .map(Some)
        });
        self.settings.work = Some(UpdateWork {
            cancel,
            _task: cx.spawn_in(window, async move |library, cx| {
                while let Ok((label, percent)) = updates.recv().await {
                    let _ = library.update_in(cx, |library, _, cx| {
                        if let Some(label) = label {
                            library.settings.message = label;
                        }
                        library.settings.downloading = Some(percent);
                        cx.notify();
                    });
                }
                let result = operation.await;
                let _ = library.update_in(cx, |library, window, cx| {
                    let settings = &mut library.settings;
                    settings.work = None;
                    settings.downloading = None;
                    settings.message = match result {
                        Ok(Some(update)) => {
                            let version = update.candidate().version().to_string();
                            settings.prepared = Some(update);
                            format!("{version} is ready. Click Restart to update to install it.")
                        }
                        Ok(None) => "You are up to date. Checking every 10 minutes.".into(),
                        Err(error) => format!("{error}. Updates will retry in 10 minutes."),
                    };
                    library.resume_navigation(window, cx);
                    cx.notify();
                });
            }),
        });
        cx.notify();
    }

    fn restart_for_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.prepared.is_some() {
            self.request_navigation(Navigation::RestartForUpdate, window, cx);
        }
    }

    pub(super) fn launch_prepared_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(installer) = self.settings.prepared.take() else {
            return;
        };
        match installer.launch() {
            Ok(()) => window.remove_window(),
            Err(error) => {
                self.settings.message =
                    format!("Update did not finish: {error}. Check for updates to retry.");
                cx.notify();
            }
        }
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext};
    use gpui_component::Root;
    use refract_native::codex_connection::CodexMode;

    #[gpui::test]
    fn settings_read_and_save_the_existing_update_track(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        UpdateTrack::Beta.save(profile.path()).unwrap();
        cx.update(crate::initialize_app);
        let mut library = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Library::new(profile.path().join("projects"), window, cx));
            library = Some(view.clone());
            Root::new(view, window, cx)
        });
        let library = library.unwrap();
        cx.refresh().unwrap();
        let trigger = cx.debug_bounds("settings").unwrap();
        cx.simulate_click(trigger.center(), Modifiers::none());
        cx.refresh().unwrap();
        assert!(cx.debug_bounds("settings-dialog").is_some());
        cx.update(|window, app| {
            let settings = &library.read(app).settings;
            let selector = settings.track_select.read(app);
            assert_eq!(selector.selected_value(), Some(&UpdateTrack::Beta));
            assert!(settings.account.focus_handle(app).is_focused(window));
            assert!(settings.updater.is_none());
            library.update(app, |library, cx| {
                library.change_update_track(UpdateTrack::Stable, window, cx);
                library.account_action(
                    crate::account_settings_view::AccountAction::Select(CodexMode::Cli),
                    window,
                    cx,
                );
            });
        });
        cx.run_until_parked();
        assert_eq!(UpdateTrack::load(profile.path()), UpdateTrack::Stable);
        assert_eq!(CodexMode::load(profile.path()), CodexMode::Cli);
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        cx.update(|window, app| {
            let settings = &library.read(app).settings;
            assert!(settings.dialog.is_none());
            assert!(settings.open_focus.is_focused(window));
        });
    }
}
