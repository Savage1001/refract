use super::{Library, Palette};
use crate::settings_view::settings_select;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, FontWeight, SharedString,
    Subscription, Task, WeakEntity, Window, div, prelude::*, px, rgb,
};
use gpui_component::{
    Sizable, Size,
    button::{Button, ButtonVariants},
    select::{SelectEvent, SelectItem, SelectState},
};
#[cfg(windows)]
use refract_native::codex_account::CodexAccount;
#[cfg(not(windows))]
use refract_native::managed_codex::UNSUPPORTED_PLATFORM;
use refract_native::{codex_connection::CodexMode, managed_codex::RuntimeProgress};
#[cfg(not(windows))]
use std::path::PathBuf;
use std::{
    io,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone)]
struct ModeChoice(CodexMode);

impl SelectItem for ModeChoice {
    type Value = CodexMode;

    fn title(&self) -> SharedString {
        match self.0 {
            CodexMode::Account => "ChatGPT account",
            CodexMode::Cli => "Codex CLI",
        }
        .into()
    }

    fn value(&self) -> &CodexMode {
        &self.0
    }
}

#[derive(Clone)]
struct AccountDisplay {
    mode: CodexMode,
    installed: bool,
    signed_in: bool,
    email: Option<String>,
    has_registration: bool,
    remembered: Option<String>,
}

impl AccountDisplay {
    fn empty(mode: CodexMode) -> Self {
        Self {
            mode,
            installed: false,
            signed_in: false,
            email: None,
            has_registration: false,
            remembered: None,
        }
    }

    fn status(&self) -> String {
        if !self.installed {
            return "Download the Codex runtime to continue.".into();
        }
        if self.signed_in {
            return self.email.as_ref().map_or_else(
                || "Signed in to ChatGPT.".into(),
                |email| format!("Signed in as {email}"),
            );
        }
        "Connect your ChatGPT account to use Codex.".into()
    }
}

#[derive(Clone, Copy)]
pub(super) enum AccountAction {
    Select(CodexMode),
    Download,
    SignIn {
        new_registration: bool,
        install_first: bool,
    },
    SignOut,
}

enum AccountEvent {
    Status(&'static str, Option<u8>),
    #[cfg(windows)]
    OpenBrowser(String),
}

struct AccountWork {
    cancel: Arc<AtomicBool>,
    _task: Task<()>,
}

impl Drop for AccountWork {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

#[derive(Clone)]
struct AccountProfile {
    #[cfg(not(windows))]
    path: PathBuf,
    #[cfg(windows)]
    service: Arc<CodexAccount>,
}

impl AccountProfile {
    fn status(&self) -> io::Result<AccountDisplay> {
        #[cfg(windows)]
        {
            let status = self.service.status()?;
            Ok(AccountDisplay {
                mode: status.mode,
                installed: status.installed,
                signed_in: status.signed_in,
                email: status.email,
                has_registration: status.has_registration,
                remembered: status.remembered,
            })
        }
        #[cfg(not(windows))]
        Ok(AccountDisplay::empty(CodexMode::load(&self.path)))
    }

    fn run(
        &self,
        action: AccountAction,
        cancelled: &AtomicBool,
        updates: &async_channel::Sender<AccountEvent>,
    ) -> io::Result<Option<&'static str>> {
        let progress = |progress| {
            let event = match progress {
                RuntimeProgress::Downloading(percent) => {
                    AccountEvent::Status("Downloading the Codex runtime…", Some(percent))
                }
                RuntimeProgress::Installing => {
                    AccountEvent::Status("Installing the Codex runtime…", None)
                }
            };
            let _ = updates.try_send(event);
        };
        match action {
            AccountAction::Select(mode) => {
                #[cfg(windows)]
                self.service.select(mode, cancelled, progress)?;
                #[cfg(not(windows))]
                {
                    mode.save(&self.path)?;
                    if mode == CodexMode::Account {
                        return Err(io::Error::new(
                            io::ErrorKind::Unsupported,
                            UNSUPPORTED_PLATFORM,
                        ));
                    }
                }
            }
            AccountAction::Download => {
                #[cfg(windows)]
                self.service.install(cancelled, progress)?;
                #[cfg(not(windows))]
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    UNSUPPORTED_PLATFORM,
                ));
            }
            AccountAction::SignIn {
                new_registration,
                install_first,
            } => {
                #[cfg(windows)]
                {
                    if install_first {
                        self.service.install(cancelled, progress)?;
                    }
                    let _ = updates.try_send(AccountEvent::Status(
                        "Finish signing in in your browser.",
                        None,
                    ));
                    self.service.sign_in(
                        new_registration,
                        |url| {
                            updates
                                .try_send(AccountEvent::OpenBrowser(url.to_owned()))
                                .map_err(|_| io::Error::other("Could not open ChatGPT sign-in."))
                        },
                        cancelled,
                    )?;
                }
                #[cfg(not(windows))]
                {
                    let _ = (new_registration, install_first);
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        UNSUPPORTED_PLATFORM,
                    ));
                }
            }
            AccountAction::SignOut =>
            {
                #[cfg(windows)]
                if !self.service.sign_out(cancelled)? {
                    return Ok(Some(
                        "Signed out on this device. You can also disconnect Refract in ChatGPT Settings.",
                    ));
                }
            }
        }
        #[cfg(not(windows))]
        let _ = (cancelled, progress);
        Ok(None)
    }
}

pub(super) struct AccountControls {
    profile: AccountProfile,
    selector: Entity<SelectState<Vec<ModeChoice>>>,
    _select: Subscription,
    display: AccountDisplay,
    refresh: Option<Task<()>>,
    work: Option<AccountWork>,
    message: Option<String>,
    percent: Option<u8>,
    can_cancel: bool,
}

pub(super) struct AccountSection {
    selector: Entity<SelectState<Vec<ModeChoice>>>,
    display: AccountDisplay,
    message: Option<String>,
    percent: Option<u8>,
    busy: bool,
    can_cancel: bool,
}

impl AccountControls {
    pub(super) fn new(profile: &Path, window: &mut Window, cx: &mut Context<Library>) -> Self {
        let mode = CodexMode::load(profile);
        let selector = cx.new(|cx| {
            let options = vec![ModeChoice(CodexMode::Account), ModeChoice(CodexMode::Cli)];
            let mut state = SelectState::new(options, None, window, cx);
            state.set_selected_value(&mode, window, cx);
            state
        });
        let select = cx.subscribe_in(&selector, window, |library, _, event, window, cx| {
            if let SelectEvent::Confirm(Some(mode)) = event
                && library.settings.account.display.mode != *mode
            {
                library.account_action(AccountAction::Select(*mode), window, cx);
            }
        });
        Self {
            profile: AccountProfile {
                #[cfg(not(windows))]
                path: profile.to_path_buf(),
                #[cfg(windows)]
                service: Arc::new(CodexAccount::new(profile.to_path_buf())),
            },
            selector,
            _select: select,
            display: AccountDisplay::empty(mode),
            refresh: None,
            work: None,
            message: None,
            percent: None,
            can_cancel: false,
        }
    }

    #[cfg(windows)]
    pub(super) fn service(&self) -> Arc<CodexAccount> {
        self.profile.service.clone()
    }

    pub(super) fn focus_handle(&self, app: &App) -> FocusHandle {
        self.selector.read(app).focus_handle(app)
    }

    pub(super) fn busy(&self) -> bool {
        self.work.is_some()
    }

    pub(super) fn cancel_setup(&self) {
        if self.can_cancel
            && let Some(work) = &self.work
        {
            work.cancel.store(true, Ordering::Release);
        }
    }

    pub(super) fn section(&self) -> AccountSection {
        AccountSection {
            selector: self.selector.clone(),
            display: self.display.clone(),
            message: self.message.clone(),
            percent: self.percent,
            busy: self.work.is_some(),
            can_cancel: self.can_cancel,
        }
    }

    fn apply_status(
        &mut self,
        status: io::Result<AccountDisplay>,
        window: &mut Window,
        cx: &mut App,
    ) {
        match status {
            Ok(status) => {
                self.display = status;
                let mode = self.display.mode;
                self.selector.update(cx, |selector, cx| {
                    selector.set_selected_value(&mode, window, cx)
                });
            }
            Err(_) if self.message.is_none() => {
                self.message = Some("Could not read Codex settings.".into());
            }
            Err(_) => {}
        }
    }
}

impl AccountSection {
    pub(super) fn contents(&self, library: WeakEntity<Library>, palette: Palette) -> AnyElement {
        let account_button = |id: &'static str, label: String, action: AccountAction| {
            let owner = library.clone();
            account_action_button(id, label, palette).on_click(move |_, window, cx| {
                let _ = owner.update(cx, |library, cx| library.account_action(action, window, cx));
            })
        };
        let cancel = library.clone();
        let message = self
            .message
            .clone()
            .unwrap_or_else(|| self.display.status());
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .pb(px(14.))
            .mb(px(6.))
            .border_b_1()
            .border_color(rgb(palette.border))
            .child(
                div()
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .font_weight(FontWeight::MEDIUM)
                    .child("Codex connection"),
            )
            .child(settings_select(&self.selector, palette, self.busy))
            .when(self.display.mode == CodexMode::Account, |section| {
                section
                    .child(
                        div()
                            .mt(px(4.))
                            .text_size(px(13.))
                            .line_height(px(20.8))
                            .text_color(rgb(palette.muted))
                            .child(message),
                    )
                    .when_some(self.percent, |section, percent| {
                        section.child(
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
                            .gap(px(8.))
                            .when(self.busy && self.can_cancel, |actions| {
                                actions.child(
                                    account_action_button("cancel-codex", "Cancel".into(), palette)
                                        .on_click(move |_, _, app| {
                                            let _ = cancel.update(app, |library, cx| {
                                                if let Some(work) = &library.settings.account.work {
                                                    work.cancel.store(true, Ordering::Release);
                                                    library.settings.account.can_cancel = false;
                                                    cx.notify();
                                                }
                                            });
                                        }),
                                )
                            })
                            .when(!self.busy && !self.display.installed, |actions| {
                                actions.child(account_button(
                                    "download-codex",
                                    "Download Codex".into(),
                                    AccountAction::Download,
                                ))
                            })
                            .when(
                                !self.busy && self.display.installed && !self.display.signed_in,
                                |actions| {
                                    actions
                                        .child(account_button(
                                            "continue-with-chatgpt",
                                            self.display.remembered.as_ref().map_or_else(
                                                || "Continue with ChatGPT".into(),
                                                |email| format!("Reconnect {email}"),
                                            ),
                                            AccountAction::SignIn {
                                                new_registration: false,
                                                install_first: false,
                                            },
                                        ))
                                        .when(self.display.has_registration, |actions| {
                                            actions.child(account_button(
                                                "another-chatgpt-account",
                                                "Use another account".into(),
                                                AccountAction::SignIn {
                                                    new_registration: true,
                                                    install_first: false,
                                                },
                                            ))
                                        })
                                },
                            )
                            .when(!self.busy && self.display.signed_in, |actions| {
                                actions.child(account_button(
                                    "sign-out-chatgpt",
                                    "Sign out".into(),
                                    AccountAction::SignOut,
                                ))
                            }),
                    )
            })
            .into_any_element()
    }
}

fn account_action_button(id: &'static str, label: String, palette: Palette) -> Button {
    Button::new(id)
        .ghost()
        .with_size(Size::Size(px(33.5)))
        .h(px(33.5))
        .px(px(10.))
        .child(
            div()
                .text_size(px(13.))
                .line_height(px(19.5))
                .text_color(rgb(palette.text))
                .child(label),
        )
}

impl Library {
    pub(super) fn refresh_account_settings(&mut self, window: &Window, cx: &mut Context<Self>) {
        let account = &mut self.settings.account;
        if account.refresh.is_some() || account.work.is_some() {
            return;
        }
        let profile = account.profile.clone();
        let status = cx
            .background_executor()
            .spawn(async move { profile.status() });
        account.refresh = Some(cx.spawn_in(window, async move |library, cx| {
            let status = status.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.settings.account.refresh = None;
                library.settings.account.apply_status(status, window, cx);
                cx.notify();
            });
        }));
    }

    pub(super) fn account_action(
        &mut self,
        action: AccountAction,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if self.preparing_exit() {
            return;
        }
        let account = &mut self.settings.account;
        if account.work.is_some() {
            return;
        }
        account.refresh = None;
        if let AccountAction::Select(mode) = action {
            account.display.mode = mode;
        }
        account.can_cancel = !matches!(
            action,
            AccountAction::SignOut | AccountAction::Select(CodexMode::Cli)
        );
        account.message = Some(
            match action {
                AccountAction::SignOut => "Signing out of ChatGPT…",
                AccountAction::SignIn {
                    install_first: false,
                    ..
                } => "Preparing ChatGPT sign-in…",
                _ => "Updating Codex settings…",
            }
            .into(),
        );
        account.percent = None;
        let profile = account.profile.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancelled = cancel.clone();
        let (updates, events) = async_channel::bounded(104);
        let work = cx.background_executor().spawn(async move {
            let result = profile.run(action, &cancelled, &updates);
            let status = profile.status();
            (result, status)
        });
        account.work =
            Some(AccountWork {
                cancel,
                _task: cx.spawn_in(window, async move |library, cx| {
                    while let Ok(event) = events.recv().await {
                        let _ =
                            library.update_in(cx, |library, _, cx| {
                                match event {
                                    AccountEvent::Status(message, percent) => {
                                        library.settings.account.message = Some(message.into());
                                        library.settings.account.percent = percent;
                                    }
                                    #[cfg(windows)]
                                    AccountEvent::OpenBrowser(url) => {
                                        if library.settings.account.work.as_ref().is_some_and(
                                            |work| !work.cancel.load(Ordering::Acquire),
                                        ) {
                                            cx.open_url(&url);
                                        }
                                    }
                                }
                                cx.notify();
                            });
                    }
                    let (result, status) = work.await;
                    let _ = library.update_in(cx, |library, window, cx| {
                        let account = &mut library.settings.account;
                        account.work = None;
                        account.can_cancel = false;
                        account.percent = None;
                        account.message = match result {
                            Ok(notice) => notice.map(str::to_owned),
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => None,
                            Err(error) => Some(error.to_string()),
                        };
                        account.apply_status(status, window, cx);
                        if !library.preparing_exit() {
                            library.refresh_account_models(window, cx);
                        }
                        library.resume_navigation(window, cx);
                        cx.notify();
                    });
                }),
            });
        cx.notify();
    }
}
