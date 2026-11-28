use super::{
    Catalog, Library, ModelPicker, Provider, format_model_name, open_code_source, title_case,
};
use crate::{Palette, account_settings_view::AccountAction, conversation_view::ConversationPane};
use gpui::{
    AnyElement, Bounds, BoxShadow, Context, Entity, FocusHandle, Focusable, FontWeight,
    KeyDownEvent, Pixels, Point, Render, RenderImage, ScrollHandle, SharedString, Size,
    Subscription, WeakEntity, Window, anchored, canvas, deferred, div, img, point, prelude::*, px,
    rgb, rgba, size, svg,
};
use gpui_component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    tooltip::Tooltip,
};
use std::sync::{Arc, LazyLock};

const PROVIDERS: [Provider; 2] = [Provider::Codex, Provider::OpenCode];
const FILTERS: [Scope; 3] = [
    Scope::Favorites,
    Scope::Provider(Provider::Codex),
    Scope::Provider(Provider::OpenCode),
];

#[path = "model_menu_keyboard.rs"]
mod keyboard;
use keyboard::{EffortFocus, Keyboard, ModelKey, ResultFocus, ResultItem, ResultRow};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scope {
    All,
    Favorites,
    Provider(Provider),
}

pub(in crate::conversation_view) struct ModelMenu {
    owner: WeakEntity<Library>,
    focus: FocusHandle,
    query: Entity<InputState>,
    scroll: ScrollHandle,
    first: Provider,
    scope: Scope,
    wait_for_opening_scope: bool,
    details: bool,
    keyboard: Keyboard,
    _query_changes: Subscription,
    _focus_out: Subscription,
    _keys: Subscription,
}

struct MenuColors {
    base: Palette,
    outline: u32,
    divider: u32,
    rail: u32,
    selected: u32,
    muted: u32,
    focus: u32,
}

impl MenuColors {
    fn for_window(window: &Window) -> Self {
        let base = Palette::for_window(window);
        let [outline, divider, rail, selected, muted, focus] = if base.canvas == 0x0a0a0a {
            [0x3a3a3a, 0x2c2c2c, 0x161616, 0x262626, 0x8f8f8f, 0x4779ff]
        } else {
            [0xd8d8d9, 0xe5e5e5, 0xfbfbfb, 0xececec, 0x686871, 0x1b4ed8]
        };
        Self {
            base,
            outline,
            divider,
            rail,
            selected,
            muted,
            focus,
        }
    }
}

pub(super) fn provider_label(provider: Provider) -> &'static str {
    match provider {
        Provider::Codex => "Codex",
        Provider::OpenCode => "OpenCode",
    }
}

fn provider_logo(provider: Provider, size: f32, palette: Palette) -> gpui::Div {
    type Icon = LazyLock<Option<Arc<RenderImage>>>;
    fn asset(bytes: &[u8]) -> Option<Arc<RenderImage>> {
        crate::asset_image::render_svg_icon(bytes, 128.).ok()
    }
    static CODEX_LIGHT: Icon = LazyLock::new(|| {
        asset(include_bytes!(
            "../../../resources/provider-logos/openai-black.svg"
        ))
    });
    static CODEX_DARK: Icon = LazyLock::new(|| {
        asset(include_bytes!(
            "../../../resources/provider-logos/openai-white.svg"
        ))
    });
    static OPENCODE_LIGHT: Icon = LazyLock::new(|| {
        asset(include_bytes!(
            "../../../resources/provider-logos/opencode-light.svg"
        ))
    });
    static OPENCODE_DARK: Icon = LazyLock::new(|| {
        asset(include_bytes!(
            "../../../resources/provider-logos/opencode-dark.svg"
        ))
    });

    let dark = palette.canvas == 0x0a0a0a;
    let art = match (provider, dark) {
        (Provider::Codex, false) => &CODEX_LIGHT,
        (Provider::Codex, true) => &CODEX_DARK,
        (Provider::OpenCode, false) => &OPENCODE_LIGHT,
        (Provider::OpenCode, true) => &OPENCODE_DARK,
    };
    let diameter = px(if provider == Provider::Codex {
        size * 2.
    } else {
        size
    });
    let logo = match art.as_ref() {
        Some(image) => img(image.clone())
            .size(diameter)
            .flex_none()
            .into_any_element(),
        None => svg()
            .path(match provider {
                Provider::Codex => "refract/openai.svg",
                Provider::OpenCode => "refract/opencode.svg",
            })
            .size(diameter)
            .flex_none()
            .text_color(rgb(palette.text))
            .into_any_element(),
    };
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .child(logo)
}

fn popup_geometry(anchor: Bounds<Pixels>, viewport: Size<Pixels>) -> (Point<Pixels>, Size<Pixels>) {
    let margin = px(10.);
    let gap = px(8.);
    let width = (viewport.width - margin * 2.).clamp(px(0.), px(554.));
    let above = anchor.top() - margin - gap;
    let below = viewport.height - anchor.bottom() - margin - gap;
    let overlay = above.max(below) < px(310.);
    let place_above = above >= px(450.) || above >= below;
    let available = if overlay {
        viewport.height - margin * 2.
    } else if place_above {
        above
    } else {
        below
    };
    let height = available.clamp(px(0.), px(392.));
    let x = (anchor.right() - width)
        .min(viewport.width - width - margin)
        .max(margin);
    let y = if overlay {
        margin
    } else if place_above {
        anchor.top() - gap - height
    } else {
        anchor.bottom() + gap
    };
    (point(x, y), size(width, height))
}

fn opening_scope(active: Option<Provider>, catalog: Option<&Catalog>) -> Scope {
    let Some(provider) = active else {
        return Scope::All;
    };
    if catalog.is_some_and(|catalog| {
        catalog
            .preferences
            .model
            .as_ref()
            .is_some_and(|id| catalog.preferences.favorites.contains(id))
    }) {
        Scope::Favorites
    } else {
        Scope::Provider(provider)
    }
}

impl ModelMenu {
    fn new(
        owner: WeakEntity<Library>,
        active: Option<Provider>,
        scope: Scope,
        wait_for_opening_scope: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search models..."));
        let focus = cx.focus_handle();
        let exit = owner.clone();
        let focus_out = window.on_focus_out(&focus, cx, move |_, window, app| {
            let _ = exit.update(app, |library, cx| {
                let on_trigger = library
                    .selection
                    .as_ref()
                    .and_then(|selection| selection.conversation.model_trigger.as_ref())
                    .is_some_and(|trigger| trigger.is_focused(window));
                if !on_trigger {
                    library.close_model_menu(false, window, cx);
                }
            });
        });
        let query_changes = cx.subscribe(&query, |menu, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                menu.scroll.set_offset(point(px(0.), px(0.)));
                menu.details = false;
                menu.keyboard.current = None;
            }
            cx.notify();
        });
        // GPUI dispatches the input's bound actions before element key listeners.
        let menu = cx.weak_entity();
        let menu_focus = focus.clone();
        let keys = cx.intercept_keystrokes(move |event, window, app| {
            if menu_focus.contains_focused(window, app) {
                let _ = menu.update(app, |menu, cx| {
                    menu.handle_menu_key(&event.keystroke, window, cx);
                });
            }
        });
        Self {
            owner,
            focus,
            query,
            scroll: ScrollHandle::new(),
            first: active.unwrap_or(Provider::Codex),
            scope,
            wait_for_opening_scope,
            details: false,
            keyboard: Keyboard::new(cx),
            _query_changes: query_changes,
            _focus_out: focus_out,
            _keys: keys,
        }
    }

    fn switch_scope(&mut self, scope: Scope, window: &mut Window, cx: &mut Context<Self>) {
        self.scope = scope;
        self.wait_for_opening_scope = false;
        self.details = false;
        self.keyboard.current = None;
        self.scroll.set_offset(point(px(0.), px(0.)));
        self.query.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn rail(&self, colors: &MenuColors, compact: bool, cx: &Context<Self>) -> impl IntoElement {
        let focus = colors.focus;
        let hover = colors.selected;
        div()
            .w(px(if compact { 40. } else { 44. }))
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .py(px(7.))
            .border_r_1()
            .border_color(rgb(colors.divider))
            .bg(rgb(colors.rail))
            .children(FILTERS.into_iter().enumerate().map(|(index, scope)| {
                let (id, label) = match scope {
                    Scope::Favorites => ("filter-favorites", "Favorites"),
                    Scope::Provider(Provider::Codex) => ("filter-codex", "Codex"),
                    _ => ("filter-opencode", "OpenCode"),
                };
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .relative()
                    .tab_index(1)
                    .track_focus(&self.keyboard.filters[index])
                    .size(px(34.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(hover)))
                    .focus(move |style| style.border_1().border_color(rgb(focus)))
                    .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
                    .when(self.scope == scope, |button| {
                        button.child(
                            div()
                                .absolute()
                                .right_0()
                                .top(px(7.))
                                .bottom(px(7.))
                                .w(px(3.))
                                .rounded(px(2.))
                                .bg(rgb(focus)),
                        )
                    })
                    .child(match scope {
                        Scope::Provider(provider) => {
                            provider_logo(provider, 18., colors.base).into_any_element()
                        }
                        _ => svg()
                            .path("refract/favorite-star.svg")
                            .size(px(20.))
                            .text_color(rgb(colors.base.text))
                            .into_any_element(),
                    })
                    .on_click(
                        cx.listener(move |menu, _, window, cx| {
                            menu.switch_scope(scope, window, cx)
                        }),
                    )
            }))
    }

    fn results(
        &mut self,
        colors: &MenuColors,
        active: Option<Provider>,
        handoff: Option<Provider>,
        sources: &[(Provider, Entity<ModelPicker>)],
        compact: bool,
        window: &mut Window,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let input = self.query.read(cx).value().to_lowercase();
        let terms: Vec<_> = input.split_whitespace().collect();
        let scope = if terms.is_empty() {
            self.scope
        } else {
            Scope::All
        };
        let second = if self.first == Provider::Codex {
            Provider::OpenCode
        } else {
            Provider::Codex
        };
        let mut entries = Vec::new();
        let mut groups = 0;
        let mut count = 0;
        for provider in [self.first, second] {
            if matches!(scope, Scope::Provider(only) if only != provider) {
                continue;
            }
            let Some((_, source)) = sources.iter().find(|(kind, _)| *kind == provider) else {
                if scope == Scope::Provider(provider) {
                    entries.push(ResultItem::Element(
                        status(
                            "Provider unavailable",
                            "Install and authenticate this provider to use it.",
                            colors,
                        )
                        .into_any_element(),
                    ));
                }
                continue;
            };
            let picker = source.read(cx);
            let catalog = picker.catalog();
            let offline = picker.provider.is_none()
                && catalog.is_some_and(|catalog| catalog.options.is_err());
            if scope == Scope::All && offline {
                continue;
            }
            let mut rows = Vec::new();
            if let Some(catalog) = catalog {
                let make_row = |id: &str, model: Option<&super::Model>| {
                    let available = model.is_some() && picker.loading.is_none();
                    let (name, detail) = model.map_or_else(
                        || {
                            let displayed = format_model_name(id.rsplit('/').next().unwrap_or(id));
                            let origin = if provider == Provider::OpenCode {
                                open_code_source(id)
                            } else {
                                id.to_owned()
                            };
                            (displayed, Some(format!("{origin} \u{00b7} Unavailable")))
                        },
                        |model| (model.name.clone(), model.disambiguator.clone()),
                    );
                    ResultRow {
                        key: ModelKey(provider, id.into()),
                        name,
                        detail,
                        available,
                        favorite: catalog
                            .preferences
                            .favorites
                            .iter()
                            .any(|favored| favored == id),
                        active: active == Some(provider)
                            && catalog.preferences.model.as_deref() == Some(id),
                        handoff: available && handoff.is_some_and(|current| current != provider),
                    }
                };
                if scope == Scope::Favorites {
                    rows.extend(catalog.preferences.favorites.iter().map(|id| {
                        make_row(id, catalog.models().iter().find(|model| model.id == *id))
                    }));
                } else {
                    rows.extend(
                        catalog
                            .models()
                            .iter()
                            .filter(|model| model.matches_search(provider_label(provider), &terms))
                            .map(|model| make_row(&model.id, Some(model))),
                    );
                }
            }
            let notice = if scope == Scope::Favorites
                && offline
                && rows.is_empty()
                && picker.save_error.is_none()
            {
                None
            } else {
                self.provider_status(source, colors, cx)
            };
            if rows.is_empty() && notice.is_none() {
                continue;
            }
            if groups > 0 {
                entries.push(ResultItem::Element(
                    div()
                        .mt(px(5.))
                        .mb(px(4.))
                        .h(px(1.))
                        .bg(rgb(colors.divider))
                        .into_any_element(),
                ));
            }
            if matches!(scope, Scope::All | Scope::Favorites) {
                entries.push(ResultItem::Element(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .min_h(px(23.))
                        .px(px(6.))
                        .text_size(px(11.))
                        .line_height(px(16.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(colors.muted))
                        .child(provider_logo(provider, 16., colors.base))
                        .child(provider_label(provider))
                        .into_any_element(),
                ));
            }
            count += rows.len();
            entries.extend(rows.into_iter().map(ResultItem::Model));
            entries.extend(notice.map(ResultItem::Element));
            groups += 1;
        }
        if count == 0 && !terms.is_empty() {
            entries.push(ResultItem::Element(
                status(
                    "No matching models",
                    "Try another name or provider.",
                    colors,
                )
                .child(
                    Button::new("clear-model-search")
                        .ghost()
                        .small()
                        .label("Clear search")
                        .on_click(cx.listener(|menu, _, window, cx| {
                            menu.switch_scope(menu.scope, window, cx)
                        })),
                )
                .into_any_element(),
            ));
        } else if groups == 0 {
            let message = match scope {
                Scope::Favorites => Some((
                    "No favorites yet",
                    "Use the star beside a model to find it here.",
                )),
                Scope::All => Some((
                    "No providers found",
                    "Install and authenticate Codex or OpenCode to use the assistant.",
                )),
                Scope::Provider(_) => None,
            };
            if let Some((title, detail)) = message {
                entries.push(ResultItem::Element(
                    status(title, detail, colors).into_any_element(),
                ));
            }
        }
        let search = self.query.read(cx).focus_handle(cx);
        let tab_target = self.keyboard.rebuild_results(&entries, &search, window, cx);
        let mut focus = self.keyboard.rows.iter();
        let mut nth_choice = 0;
        div()
            .id("model-results")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .px(px(10.))
            .pt(px(7.))
            .pb(px(9.))
            .children(entries.into_iter().map(|item| match item {
                ResultItem::Element(element) => element,
                ResultItem::Model(row) => {
                    let focus = focus.next().expect("focus for each model row");
                    let tab_stop = tab_target.as_ref() == Some(&row.key);
                    let shortcut = if row.available {
                        nth_choice += 1;
                        (!compact && nth_choice <= 9).then_some(nth_choice)
                    } else {
                        None
                    };
                    self.model_row(row, focus, tab_stop, shortcut, colors, cx)
                        .into_any_element()
                }
            }))
    }

    fn provider_status(
        &self,
        source: &Entity<ModelPicker>,
        colors: &MenuColors,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let picker = source.read(cx);
        let retry = source.downgrade();
        let retry_button = |id, label| {
            Button::new(id)
                .small()
                .ghost()
                .label(label)
                .on_click(move |_, _, app| {
                    let _ = retry.update(app, |picker, cx| picker.load(cx));
                })
        };
        if let Some(error) = &picker.save_error {
            let retry_save = source.downgrade();
            return Some(
                status("Settings weren’t saved", error.message.clone(), colors)
                    .child(
                        Button::new("retry-model-save")
                            .small()
                            .ghost()
                            .label("Retry saving")
                            .disabled(picker.loading.is_some())
                            .on_click(move |_, _, app| {
                                let _ = retry_save.update(app, |picker, cx| {
                                    if let Some(catalog) = picker.catalog() {
                                        picker.save(catalog.preferences.clone(), false, cx);
                                    }
                                });
                            }),
                    )
                    .into_any_element(),
            );
        }
        let notice = match picker.catalog.as_ref() {
            _ if picker.loading.is_some() => status(
                "Loading models…",
                format!(
                    "Checking the {} catalog and preferences.",
                    provider_label(picker.kind)
                ),
                colors,
            ),
            Some(Err(error)) => status("Preferences unavailable", error.clone(), colors)
                .child(retry_button("retry-model-preferences", "Retry preferences")),
            Some(Ok(Catalog {
                options: Err(error),
                ..
            })) if picker.provider.is_none() => {
                status("Provider unavailable", error.message().to_owned(), colors)
            }
            Some(Ok(Catalog {
                options: Err(error),
                ..
            })) if picker.needs_account_setup(error.message()) => {
                let owner = self.owner.clone();
                let busy = owner
                    .read_with(cx, |library, _| library.settings.account.busy())
                    .unwrap_or(false);
                status("Codex requires sign-in", error.message().to_owned(), colors).child(
                    Button::new("model-picker-sign-in")
                        .small()
                        .label(if busy {
                            "Connecting…"
                        } else {
                            "Sign in with ChatGPT"
                        })
                        .disabled(busy)
                        .on_click(move |_, window, app| {
                            let _ = owner.update(app, |library, cx| {
                                library.account_action(
                                    AccountAction::SignIn {
                                        new_registration: false,
                                        install_first: true,
                                    },
                                    window,
                                    cx,
                                )
                            });
                        }),
                )
            }
            Some(Ok(Catalog {
                options: Err(error),
                ..
            })) => status(
                "Models couldn’t be loaded",
                error.message().to_owned(),
                colors,
            )
            .child(retry_button(
                "retry-model-discovery",
                "Retry model discovery",
            )),
            Some(Ok(catalog)) if catalog.models().is_empty() => status(
                "No models available",
                "Check your provider setup to add models.",
                colors,
            )
            .child(retry_button("check-models", "Check for models")),
            _ => return None,
        };
        Some(notice.into_any_element())
    }

    fn editable_picker(&self, provider: Provider, app: &gpui::App) -> Option<Entity<ModelPicker>> {
        self.owner
            .read_with(app, |library, _| {
                let pane = &library.selection.as_ref()?.conversation;
                if pane.running.is_some() {
                    return None;
                }
                pane.models.get(&provider).cloned()
            })
            .ok()
            .flatten()
    }

    fn choose_key(&mut self, key: &ModelKey, cx: &mut Context<Self>) {
        let Some(picker) = self.editable_picker(key.0, cx) else {
            return;
        };
        if !picker.update(cx, |picker, cx| picker.choose(&key.1, cx)) {
            return;
        }
        let _ = self.owner.update(cx, |library, cx| {
            if let Some(selection) = &mut library.selection
                && selection.conversation.provider != Some(key.0)
            {
                selection.conversation.provider = Some(key.0);
                library.schedule_chat_draft_save(cx);
                cx.notify();
            }
        });
        self.keyboard.current = Some(key.clone());
        cx.notify();
    }

    fn choose_effort(&self, provider: Provider, value: &str, cx: &mut Context<Self>) {
        if let Some(picker) = self.editable_picker(provider, cx) {
            picker.update(cx, |picker, cx| picker.choose_effort(value, cx));
        }
    }

    fn set_favorite(&mut self, key: &ModelKey, favorite: bool, cx: &mut Context<Self>) {
        if let Some(picker) = self.editable_picker(key.0, cx) {
            picker.update(cx, |picker, cx| picker.set_favorite(&key.1, favorite, cx));
            self.keyboard.current = Some(key.clone());
            cx.notify();
        }
    }

    fn model_row(
        &self,
        row: ResultRow,
        handles: &ResultFocus,
        tab_stop: bool,
        shortcut: Option<usize>,
        colors: &MenuColors,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let ResultRow {
            key,
            name,
            detail,
            available,
            favorite,
            active,
            handoff,
        } = row;
        let row_id = format!("model/{}/{}", key.0.as_str(), key.1);
        let star_label = if favorite {
            format!("Remove {name} from favorites")
        } else {
            format!("Add {name} to favorites")
        };
        let choice_click = key.clone();
        let favorite_click = key.clone();
        let muted = colors.muted;
        let selection = colors.selected;
        let ring = colors.focus;
        div()
            .id(SharedString::from(row_id.clone()))
            .debug_selector(move || row_id.clone())
            .group("model-result")
            .flex()
            .items_center()
            .my(px(1.))
            .pr(px(5.))
            .rounded(px(5.))
            .when(active, |row| row.bg(rgb(selection)))
            .child(
                div()
                    .id("choose-model")
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(33.))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .px(px(7.))
                    .py(px(4.))
                    .rounded(px(5.))
                    .tooltip({
                        let name = name.clone();
                        move |window, cx| Tooltip::new(name.clone()).build(window, cx)
                    })
                    .when(!available, |choice| choice.opacity(0.55))
                    .when(available, |choice| {
                        choice
                            .tab_index(2)
                            .tab_stop(tab_stop)
                            .track_focus(&handles.choice)
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgb(selection)))
                            .focus(move |style| {
                                style.bg(rgb(selection)).border_1().border_color(rgb(ring))
                            })
                            .on_click(
                                cx.listener(move |menu, _, _, cx| {
                                    menu.choose_key(&choice_click, cx)
                                }),
                            )
                    })
                    .child(provider_logo(key.0, 16., colors.base).opacity(0.72))
                    .child(
                        div()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .min_w(px(0.))
                                            .truncate()
                                            .text_size(px(14.))
                                            .line_height(px(21.))
                                            .font_weight(FontWeight(580.))
                                            .child(name),
                                    )
                                    .when(active, |line| {
                                        line.child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(rgb(muted))
                                                .child("\u{2713}"),
                                        )
                                    }),
                            )
                            .children(detail.map(|detail| {
                                div()
                                    .text_size(px(11.))
                                    .line_height(px(16.5))
                                    .text_color(rgb(muted))
                                    .child(detail)
                            })),
                    )
                    .when(handoff, |choice| {
                        choice.child(
                            div()
                                .ml_auto()
                                .text_size(px(10.))
                                .text_color(rgb(muted))
                                .child("Handoff"),
                        )
                    }),
            )
            .child(
                div()
                    .id("toggle-favorite")
                    .tab_index(2)
                    .tab_stop(tab_stop || !available)
                    .track_focus(&handles.star)
                    .w(px(25.))
                    .h(px(28.))
                    .flex_none()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(15.))
                    .line_height(px(15.))
                    .text_color(rgb(if favorite { colors.base.warning } else { muted }))
                    .when(!favorite && !active && !tab_stop, |star| {
                        star.opacity(0.)
                            .group_hover("model-result", |style| style.opacity(1.))
                    })
                    .hover(move |style| style.bg(rgb(selection)))
                    .focus(move |style| style.opacity(1.).border_1().border_color(rgb(ring)))
                    .tooltip(move |window, cx| Tooltip::new(star_label.clone()).build(window, cx))
                    .child(if favorite { "\u{2605}" } else { "\u{2606}" })
                    .on_click(cx.listener(move |menu, _, _, cx| {
                        menu.set_favorite(&favorite_click, !favorite, cx)
                    })),
            )
            .children(shortcut.map(|number| {
                div()
                    .flex_none()
                    .rounded(px(3.))
                    .px(px(4.))
                    .py(px(1.))
                    .bg(rgb(colors.divider))
                    .text_color(rgb(muted))
                    .text_size(px(11.))
                    .line_height(px(16.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("Ctrl+{number}"))
            }))
    }

    fn details(
        &mut self,
        source: Option<&Entity<ModelPicker>>,
        colors: &MenuColors,
        width: Pixels,
        window: &mut Window,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let previous = std::mem::take(&mut self.keyboard.efforts);
        let mut pane = div()
            .w(width)
            .min_h(px(0.))
            .flex_none()
            .id("model-details")
            .overflow_y_scroll()
            .border_l_1()
            .border_color(rgb(colors.divider))
            .px(px(11.))
            .pb(px(10.));
        let Some(source) = source else {
            return pane.child(status(
                "Model settings",
                "Choose a model to see its options.",
                colors,
            ));
        };
        let picker = source.read(cx);
        let Some((catalog, model)) = picker
            .catalog()
            .and_then(|catalog| Some((catalog, catalog.selected()?)))
        else {
            return pane.child(status(
                "Model settings",
                "Choose a model to see its options.",
                colors,
            ));
        };
        let codex = picker.kind == Provider::Codex;
        let provider = picker.kind;
        let ring = colors.focus;
        let selection = colors.selected;
        let chosen = catalog
            .preferences
            .reasoning_effort
            .as_ref()
            .filter(|item| model.efforts.contains(item))
            .or_else(|| model.efforts.first());
        let enabled = picker.loading.is_none();
        let focus_details = (self.details && self.focus.is_focused(window))
            || previous.iter().any(|item| {
                item.focus.is_focused(window)
                    && (item.provider != provider || !model.efforts.contains(&item.value))
            });
        let identity = model.disambiguator.as_ref().or(model.source.as_ref());
        pane = pane.child(
            div()
                .min_h(px(40.))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .border_b_1()
                .border_color(rgb(colors.divider))
                .text_color(rgb(colors.muted))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(13.))
                        .font_weight(FontWeight(520.))
                        .child(if codex { "Reasoning" } else { "Variant" }),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(model.name.clone()),
                ),
        );
        if let Some(identity) =
            identity.filter(|identity| identity.as_str() != provider_label(provider))
        {
            pane = pane.child(
                div()
                    .py(px(6.))
                    .text_size(px(11.))
                    .text_color(rgb(colors.muted))
                    .child(identity.clone()),
            );
        }
        if model.efforts.is_empty() {
            return pane.child(
                div()
                    .mt(px(6.))
                    .text_size(px(12.))
                    .text_color(rgb(colors.muted))
                    .child(if codex {
                        "Reasoning is configured by this model."
                    } else {
                        "This model offers no variants."
                    }),
            );
        }
        pane = pane.child(div().h(px(7.)).flex_none());
        for effort in &model.efforts {
            let value = effort.clone();
            let selected = catalog.preferences.reasoning_effort.as_ref() == Some(effort);
            let default = model.default_effort.as_ref() == Some(effort);
            let focus = previous
                .iter()
                .find(|previous| previous.provider == provider && previous.value == *effort)
                .map_or_else(|| cx.focus_handle(), |previous| previous.focus.clone());
            if enabled {
                if focus_details && chosen == Some(effort) {
                    window.focus(&focus);
                }
                self.keyboard.efforts.push(EffortFocus {
                    provider,
                    value: value.clone(),
                    focus: focus.clone(),
                });
            }
            pane = pane.child(
                div()
                    .id(SharedString::from(format!("effort/{effort}")))
                    .w_full()
                    .h(px(33.))
                    .mb(px(1.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(5.))
                    .rounded(px(5.))
                    .when(enabled, |button| {
                        button
                            .tab_index(3)
                            .tab_stop(chosen == Some(effort))
                            .track_focus(&focus)
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgb(selection)))
                            .focus(move |style| style.border_1().border_color(rgb(ring)))
                            .on_click(cx.listener(move |menu, _, _, cx| {
                                menu.choose_effort(provider, &value, cx)
                            }))
                    })
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight(520.))
                            .child(title_case(effort)),
                    )
                    .when(default, |button| {
                        button.child(
                            div()
                                .ml_auto()
                                .text_size(px(11.))
                                .text_color(rgb(colors.muted))
                                .child("Default"),
                        )
                    })
                    .when(selected, |button| {
                        button.child(
                            div()
                                .when(!default, |mark| mark.ml_auto())
                                .text_size(px(11.))
                                .text_color(rgb(colors.base.text))
                                .child("✓"),
                        )
                    }),
            );
        }
        pane
    }
}

fn speed_control(
    source: &Entity<ModelPicker>,
    colors: &MenuColors,
    app: &gpui::App,
) -> Option<impl IntoElement> {
    let picker = source.read(app);
    if picker.kind != Provider::Codex || picker.provider.is_none() {
        return None;
    }
    let current = picker.catalog()?.preferences.fast_mode.unwrap_or_default();
    let disabled = picker.loading.is_some();
    Some(
        div()
            .flex()
            .gap(px(1.))
            .p(px(2.))
            .rounded(px(6.))
            .bg(rgb(colors.base.selection))
            .children([(false, "Standard"), (true, "Fast")].map(|(fast, label)| {
                let pick = source.downgrade();
                Button::new(label)
                    .ghost()
                    .with_size(gpui_component::Size::Size(px(23.)))
                    .h(px(23.))
                    .px(px(7.))
                    .disabled(disabled)
                    .when(current == fast, |button| button.bg(rgb(colors.selected)))
                    .when(fast, |button| {
                        button.tooltip("Fast may use more of your limit.")
                    })
                    .child(
                        div()
                            .text_size(px(11.))
                            .line_height(px(16.5))
                            .font_weight(if current == fast {
                                FontWeight(650.)
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(rgb(if current == fast {
                                colors.base.text
                            } else {
                                colors.muted
                            }))
                            .child(if fast { "⚡︎ Fast" } else { label }),
                    )
                    .on_click(move |_, _, app| {
                        let _ = pick.update(app, |picker, cx| picker.set_fast_mode(fast, cx));
                    })
            })),
    )
}

fn status(title: &'static str, details: impl Into<SharedString>, colors: &MenuColors) -> gpui::Div {
    div()
        .p(px(7.))
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight(520.))
                .child(title),
        )
        .child(
            div()
                .text_size(px(11.))
                .line_height(px(16.5))
                .text_color(rgb(colors.muted))
                .child(details.into()),
        )
}

impl Render for ModelMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some((anchor, active, handoff, sources)) = self
            .owner
            .read_with(cx, |library, _| {
                let pane = &library.selection.as_ref()?.conversation;
                Some((
                    pane.model_anchor.get()?,
                    pane.provider,
                    pane.history
                        .as_ref()
                        .and_then(|history| history.provider.as_deref())
                        .and_then(|provider| Provider::parse(provider).ok()),
                    pane.models
                        .iter()
                        .map(|(&provider, source)| (provider, source.clone()))
                        .collect::<Vec<_>>(),
                ))
            })
            .ok()
            .flatten()
        else {
            return div().into_any_element();
        };
        if self.wait_for_opening_scope
            && let Some((_, source)) = sources
                .iter()
                .find(|(provider, _)| active == Some(*provider))
            && let Some(catalog) = source.read(cx).catalog.as_ref()
        {
            self.scope = opening_scope(active, catalog.as_ref().ok());
            self.wait_for_opening_scope = false;
        }
        let colors = MenuColors::for_window(window);
        let (position, dimensions) = popup_geometry(anchor, window.viewport_size());
        let compact = dimensions.width < px(470.);
        let searching = !self.query.read(cx).value().trim().is_empty();
        if compact
            && self
                .keyboard
                .efforts
                .iter()
                .any(|effort| effort.focus.is_focused(window))
        {
            self.details = true;
        }
        let close = self.owner.clone();
        let click_out = self.owner.clone();
        let search_active = self.query.read(cx).focus_handle(cx).is_focused(window);
        let settings_width = if compact {
            dimensions.width - px(2.)
        } else {
            (dimensions.width * 0.34).max(px(160.))
        };
        let active_source = sources
            .iter()
            .find(|(provider, _)| active == Some(*provider))
            .map(|(_, source)| source);
        let show_catalog = !(compact && self.details);
        let results = if show_catalog {
            Some(self.results(&colors, active, handoff, &sources, compact, window, cx))
        } else {
            self.keyboard.rows.clear();
            None
        };
        let details = if !compact || self.details {
            Some(self.details(active_source, &colors, settings_width, window, cx))
        } else {
            self.keyboard.efforts.clear();
            None
        };
        let footer =
            div()
                .h(px(44.))
                .flex_none()
                .pl(px(11.))
                .pr(px(8.))
                .flex()
                .gap(px(6.))
                .items_center()
                .border_t_1()
                .border_color(rgb(colors.divider))
                .child(
                    div().flex_1().min_w(px(0.)).flex().children(
                        active_source.and_then(|source| speed_control(source, &colors, cx)),
                    ),
                )
                .when(compact, |footer| {
                    footer.child(
                        Button::new("toggle-model-details")
                            .ghost()
                            .small()
                            .label(if self.details { "Models" } else { "Settings" })
                            .on_click(cx.listener(|menu, _, window, cx| {
                                menu.details = !menu.details;
                                if menu.details {
                                    window.focus(&menu.focus);
                                } else {
                                    menu.query.update(cx, |input, cx| input.focus(window, cx));
                                }
                                cx.notify();
                            })),
                    )
                })
                .child(
                    Button::new("close-model-menu")
                        .ghost()
                        .with_size(gpui_component::Size::Size(px(27.)))
                        .size(px(27.))
                        .px(px(0.))
                        .tooltip("Close model picker")
                        .child(
                            div()
                                .text_size(px(16.))
                                .text_color(rgb(colors.muted))
                                .child("×"),
                        )
                        .on_click(move |_, window, app| {
                            let _ = close.update(app, |library, cx| {
                                library.close_model_menu(true, window, cx)
                            });
                        }),
                );
        let panel = div()
            .id("model-menu")
            .debug_selector(|| "model-menu".into())
            .track_focus(&self.focus)
            .tab_group()
            .occlude()
            .w(dimensions.width)
            .h(dimensions.height)
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(9.))
            .border_1()
            .border_color(rgb(colors.outline))
            .bg(rgb(colors.base.surface))
            .text_color(rgb(colors.base.text))
            .text_size(px(13.))
            .line_height(px(19.5))
            .shadow(vec![
                BoxShadow {
                    color: rgba(colors.base.shadow).into(),
                    offset: point(px(0.), px(14.)),
                    blur_radius: px(38.),
                    spread_radius: px(0.),
                },
                BoxShadow {
                    color: rgba(colors.base.shadow).into(),
                    offset: point(px(0.), px(2.)),
                    blur_radius: px(8.),
                    spread_radius: px(0.),
                },
            ])
            .on_mouse_down_out(move |event, window, cx| {
                if !anchor.contains(&event.position) {
                    let _ = click_out.update(cx, |library, cx| {
                        library.close_model_menu(false, window, cx)
                    });
                }
            })
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .when(!searching && show_catalog, |panel| {
                        panel.child(self.rail(&colors, compact, cx))
                    })
                    .when_some(results, |panel, results| {
                        panel.child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .min_h(px(0.))
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .mx(px(11.))
                                        .h(px(39.))
                                        .flex_none()
                                        .border_b_1()
                                        .border_color(rgb(if search_active {
                                            colors.focus
                                        } else {
                                            colors.divider
                                        }))
                                        .child(
                                            Input::new(&self.query)
                                                .appearance(false)
                                                .h_full()
                                                .px(px(3.))
                                                .text_size(px(13.))
                                                .prefix(
                                                    svg()
                                                        .path("refract/search.svg")
                                                        .size(px(15.))
                                                        .text_color(rgb(colors.muted)),
                                                ),
                                        ),
                                )
                                .child(results),
                        )
                    })
                    .children(details),
            )
            .child(footer);
        deferred(anchored().position(position).child(panel))
            .with_priority(2)
            .into_any_element()
    }
}

impl Library {
    fn toggle_model_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self
            .selection
            .as_ref()
            .map(|selection| &selection.conversation)
        else {
            return;
        };
        if pane.running.is_some() {
            return;
        }
        if pane.model_menu.is_some() {
            self.close_model_menu(true, window, cx);
            return;
        }
        let active = pane.provider;
        let loaded = active
            .and_then(|kind| pane.models.get(&kind))
            .and_then(|source| source.read(cx).catalog.as_ref());
        let scope = opening_scope(active, loaded.and_then(|catalog| catalog.as_ref().ok()));
        let wait_for_opening_scope = active.is_some() && loaded.is_none();
        if cfg!(not(all(test, feature = "ui-tests"))) {
            for kind in PROVIDERS {
                self.prepare_model_catalog(kind, cx);
            }
        }
        let parent = cx.weak_entity();
        let menu =
            cx.new(|cx| ModelMenu::new(parent, active, scope, wait_for_opening_scope, window, cx));
        let query = menu.read(cx).query.clone();
        if let Some(selection) = &mut self.selection {
            selection.conversation.model_menu = Some(menu);
        }
        query.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn close_model_menu(
        &mut self,
        restore_focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(selection) = &mut self.selection
            && selection.conversation.model_menu.take().is_some()
        {
            if restore_focus && let Some(focus) = &selection.conversation.model_trigger {
                window.focus(focus);
            }
            cx.notify();
        }
    }

    pub(crate) fn model_menu_layer(&mut self, window: &Window) -> Option<impl IntoElement> {
        let selection = self.selection.as_ref()?;
        if !self
            .displayed_panes(selection, f32::from(window.viewport_size().width))
            .agent
            || self.settings.dialog_open()
        {
            self.selection.as_mut()?.conversation.model_menu = None;
            return None;
        }
        selection.conversation.model_menu.clone()
    }

    pub(in crate::conversation_view) fn model_picker_button(
        &self,
        pane: &ConversationPane,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let active = pane
            .provider
            .and_then(|kind| pane.models.get(&kind).map(|source| source.read(cx)));
        let label = active.map(ModelPicker::label).unwrap_or_else(|| {
            if !pane.providers_loaded {
                "Checking providers…".into()
            } else if let Some(kind) = pane.provider {
                provider_label(kind).into()
            } else {
                "No providers found".into()
            }
        });
        let (identity, options) = active
            .and_then(ModelPicker::catalog)
            .and_then(|catalog| {
                let model = catalog.selected()?;
                let effort = catalog
                    .preferences
                    .reasoning_effort
                    .as_ref()
                    .filter(|effort| model.efforts.contains(*effort))
                    .map(|effort| title_case(effort));
                let fast = (pane.provider == Some(Provider::Codex)
                    && catalog.preferences.fast_mode.unwrap_or_default())
                .then(|| "Fast".to_owned());
                let options = [effort, fast]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · ");
                Some((
                    model.disambiguator.clone(),
                    (!options.is_empty()).then_some(options),
                ))
            })
            .unwrap_or_default();
        let bounds = pane.model_anchor.clone();
        let anchor_owner = cx.weak_entity();
        let mut trigger = div()
            .id("model-picker-trigger")
            .debug_selector(|| "model-picker-trigger".into())
            .relative()
            .flex()
            .items_center()
            .gap(px(7.))
            .min_w(px(0.))
            .max_w(px(310.))
            .min_h(px(32.))
            .px(px(8.))
            .rounded(px(6.))
            .text_size(px(12.))
            .line_height(px(18.))
            .text_color(rgb(palette.text))
            .child(
                canvas(
                    move |anchor, window, app| {
                        if bounds.replace(Some(anchor)) == Some(anchor) {
                            return;
                        }
                        let menu = anchor_owner
                            .read_with(app, |library, _| {
                                library
                                    .selection
                                    .as_ref()?
                                    .conversation
                                    .model_menu
                                    .as_ref()
                                    .map(Entity::entity_id)
                            })
                            .ok()
                            .flatten();
                        if let Some(menu) = menu {
                            window.defer(app, move |_, app| app.notify(menu));
                        }
                    },
                    |_, (), _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .children(
                pane.provider
                    .map(|provider| provider_logo(provider, 16., palette)),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(label),
            )
            .children([identity, options].into_iter().flatten().map(|text| {
                div()
                    .min_w(px(0.))
                    .truncate()
                    .text_color(rgb(palette.muted))
                    .child(text)
            }));
        if let Some(focus) = &pane.model_trigger
            && pane.running.is_none()
        {
            trigger = trigger
                .track_focus(focus)
                .cursor_pointer()
                .hover(move |style| style.bg(rgb(palette.hover)))
                .focus(move |style| style.border_1().border_color(rgb(palette.accent)))
                .on_click(
                    cx.listener(|library, _, window, cx| library.toggle_model_menu(window, cx)),
                )
                .on_key_down(cx.listener(|library, event: &KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        library.toggle_model_menu(window, cx);
                        window.prevent_default();
                        cx.stop_propagation();
                    } else if event.keystroke.key == "escape"
                        && library
                            .selection
                            .as_ref()
                            .is_some_and(|selection| selection.conversation.model_menu.is_some())
                    {
                        library.close_model_menu(true, window, cx);
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                }));
        }
        trigger
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_the_menu_above_below_and_across_short_windows() {
        let viewport = size(px(900.), px(720.));
        let trigger = |x, y| Bounds::new(point(px(x), px(y)), size(px(120.), px(32.)));
        let (position, bounds) = popup_geometry(trigger(700., 650.), viewport);
        assert_eq!(position, point(px(266.), px(250.)));
        assert_eq!(bounds, size(px(554.), px(392.)));
        let (position, _) = popup_geometry(trigger(100., 90.), viewport);
        assert_eq!(position, point(px(10.), px(130.)));
        let (position, bounds) = popup_geometry(trigger(100., 200.), size(px(420.), px(400.)));
        assert_eq!(position, point(px(10.), px(10.)));
        assert_eq!(bounds, size(px(400.), px(380.)));
    }

    #[cfg(feature = "ui-tests")]
    #[gpui::test]
    fn searches_both_providers_and_changes_provider_only_when_a_model_is_chosen(
        cx: &mut gpui::TestAppContext,
    ) {
        use super::super::{AssistantConnection, AssistantPreferences, Model};
        use crate::{Root, initialize_app};
        use gpui::{Modifiers, VisualTestContext};

        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = refract_native::create_project(&projects, "Model picker").unwrap();
        let workspace = projects.join(&project.id);
        cx.update(initialize_app);
        let mut library = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Library::new(projects, window, cx));
            library = Some(view.clone());
            Root::new(view, window, cx)
        });
        let library = library.unwrap();
        cx.update(|window, app| {
            library.update(app, |view, cx| view.open_project(project, window, cx))
        });
        cx.run_until_parked();
        cx.update(|_, app| {
            library.update(app, |view, cx| {
                let owner = cx.weak_entity();
                let pane = &mut view.selection.as_mut().unwrap().conversation;
                pane.provider = Some(Provider::Codex);
                for (kind, mode, id, favorite) in [
                    (
                        Provider::Codex,
                        AssistantConnection::CodexCli,
                        "research-codex",
                        "retired-codex",
                    ),
                    (
                        Provider::OpenCode,
                        AssistantConnection::OpenCode,
                        "openai/research",
                        "openai/research",
                    ),
                ] {
                    let model = cx.new(|_| {
                        let mut picker = ModelPicker::new(
                            None,
                            kind,
                            mode,
                            workspace.clone(),
                            profile.path().to_owned(),
                            owner.clone(),
                        );
                        picker.catalog = Some(Ok(Catalog {
                            options: Ok(vec![Model {
                                id: id.into(),
                                name: "Research".into(),
                                source: None,
                                disambiguator: None,
                                legacy_ids: Vec::new(),
                                description: String::new(),
                                efforts: vec!["low".into(), "high".into()],
                                default_effort: Some("low".into()),
                                is_default: true,
                            }]),
                            preferences: AssistantPreferences {
                                model: Some(id.into()),
                                reasoning_effort: Some("low".into()),
                                favorites: vec![favorite.into()],
                                ..Default::default()
                            },
                        }));
                        picker
                    });
                    pane.models.insert(kind, model);
                }
                cx.notify();
            })
        });
        let click = |cx: &mut VisualTestContext, id: &'static str| {
            let bounds = cx.debug_bounds(id).expect(id);
            cx.simulate_click(bounds.center(), Modifiers::none());
            cx.run_until_parked();
            cx.refresh().unwrap();
        };
        cx.refresh().unwrap();
        click(cx, "model-picker-trigger");
        assert!(cx.debug_bounds("model/codex/research-codex").is_some());
        assert!(cx.debug_bounds("model/opencode/openai/research").is_none());

        click(cx, "filter-favorites");
        assert!(cx.debug_bounds("model/codex/retired-codex").is_some());
        assert!(cx.debug_bounds("model/opencode/openai/research").is_some());
        cx.simulate_keystrokes("down");
        cx.update(|window, app| {
            let pane = &library.read(app).selection.as_ref().unwrap().conversation;
            let menu = pane.model_menu.as_ref().unwrap().read(app);
            assert!(menu.keyboard.rows[1].choice.is_focused(window));
        });
        cx.simulate_keystrokes("right");
        cx.update(|window, app| {
            let pane = &library.read(app).selection.as_ref().unwrap().conversation;
            let menu = pane.model_menu.as_ref().unwrap().read(app);
            assert!(menu.keyboard.rows[1].star.is_focused(window));
        });
        cx.simulate_keystrokes("enter");
        cx.refresh().unwrap();
        cx.update(|window, app| {
            let pane = &library.read(app).selection.as_ref().unwrap().conversation;
            let menu = pane.model_menu.as_ref().unwrap().read(app);
            assert_eq!(pane.provider, Some(Provider::Codex));
            // GPUI's debug bounds can retain selectors from previous frames.
            assert_eq!(menu.keyboard.rows.len(), 1);
            assert!(
                pane.models[&Provider::OpenCode]
                    .read(app)
                    .catalog()
                    .unwrap()
                    .preferences
                    .favorites
                    .is_empty()
            );
            assert!(menu.keyboard.rows[0].star.is_focused(window));
            let input = menu.query.clone();
            input.update(app, |input, cx| input.focus(window, cx));
        });
        cx.simulate_input("Research");
        cx.run_until_parked();
        cx.refresh().unwrap();
        cx.update(|_, app| {
            let pane = &library.read(app).selection.as_ref().unwrap().conversation;
            let menu = pane.model_menu.as_ref().unwrap().read(app);
            assert_eq!(pane.provider, Some(Provider::Codex));
            assert_eq!(
                menu.keyboard
                    .rows
                    .iter()
                    .map(|row| row.key.0)
                    .collect::<Vec<_>>(),
                vec![Provider::Codex, Provider::OpenCode]
            );
        });

        cx.simulate_keystrokes("enter");
        cx.update(|window, app| {
            let pane = &library.read(app).selection.as_ref().unwrap().conversation;
            assert_eq!(pane.provider, Some(Provider::Codex));
            assert!(
                pane.model_menu.as_ref().unwrap().read(app).keyboard.rows[0]
                    .choice
                    .is_focused(window)
            );
        });
        cx.simulate_keystrokes("ctrl-2");
        cx.refresh().unwrap();
        cx.update(|window, app| {
            let pane = &library.read(app).selection.as_ref().unwrap().conversation;
            assert_eq!(pane.provider, Some(Provider::OpenCode));
            let menu = pane.model_menu.as_ref().unwrap().read(app);
            assert!(menu.keyboard.rows[1].choice.is_focused(window));
            window.focus(&menu.keyboard.efforts[0].focus);
        });
        cx.simulate_keystrokes("right");
        cx.update(|window, app| {
            let pane = &library.read(app).selection.as_ref().unwrap().conversation;
            let menu = pane.model_menu.as_ref().unwrap().read(app);
            assert!(menu.keyboard.efforts[1].focus.is_focused(window));
            let source = pane.models[&Provider::OpenCode].read(app);
            assert_eq!(
                source
                    .catalog()
                    .unwrap()
                    .preferences
                    .reasoning_effort
                    .as_deref(),
                Some("high")
            );
        });
        cx.simulate_keystrokes("escape");
        cx.update(|window, app| {
            let pane = &library.read(app).selection.as_ref().unwrap().conversation;
            assert!(pane.model_menu.is_none());
            assert!(pane.model_trigger.as_ref().unwrap().is_focused(window));
        });
    }
}
