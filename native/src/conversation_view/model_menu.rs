use super::{
    Catalog, Library, ModelPicker, Provider, format_model_name, open_code_source, title_case,
};
use crate::{Palette, account_settings_view::AccountAction, conversation_view::ConversationPane};
use gpui::{
    AnyElement, Bounds, BoxShadow, Context, Entity, FocusHandle, Focusable, FontWeight,
    KeyDownEvent, Pixels, Point, Render, ScrollHandle, SharedString, Size, Subscription,
    WeakEntity, Window, anchored, canvas, deferred, div, point, prelude::*, px, rgb, rgba, size,
    svg,
};
use gpui_component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    tooltip::Tooltip,
};

const PROVIDERS: [Provider; 2] = [Provider::Codex, Provider::OpenCode];

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
    details: bool,
    _query_changes: Subscription,
    _focus_out: Subscription,
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

fn provider_logo(provider: Provider, size: f32, color: u32) -> impl IntoElement {
    svg()
        .path(match provider {
            Provider::Codex => "refract/openai.svg",
            Provider::OpenCode => "refract/opencode.svg",
        })
        .size(px(size))
        .flex_none()
        .text_color(rgb(color))
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

impl ModelMenu {
    fn new(
        owner: WeakEntity<Library>,
        active: Option<Provider>,
        scope: Scope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search models"));
        let focus = cx.focus_handle();
        let exit = owner.clone();
        let focus_out = window.on_focus_out(&focus, cx, move |_, window, app| {
            let _ = exit.update(app, |library, cx| {
                library.close_model_menu(false, window, cx)
            });
        });
        let query_changes = cx.subscribe(&query, |menu, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                menu.scroll.set_offset(point(px(0.), px(0.)));
                menu.details = false;
            }
            cx.notify();
        });
        Self {
            owner,
            focus,
            query,
            scroll: ScrollHandle::new(),
            first: active.unwrap_or(Provider::Codex),
            scope,
            details: false,
            _query_changes: query_changes,
            _focus_out: focus_out,
        }
    }

    fn switch_scope(&mut self, scope: Scope, window: &mut Window, cx: &mut Context<Self>) {
        self.scope = scope;
        self.details = false;
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
            .children(
                [
                    Scope::Favorites,
                    Scope::Provider(Provider::Codex),
                    Scope::Provider(Provider::OpenCode),
                ]
                .into_iter()
                .map(|scope| {
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
                                provider_logo(provider, 20., colors.base.text).into_any_element()
                            }
                            _ => svg()
                                .path("refract/favorite-star.svg")
                                .size(px(20.))
                                .text_color(rgb(colors.base.text))
                                .into_any_element(),
                        })
                        .on_click(cx.listener(move |menu, _, window, cx| {
                            menu.switch_scope(scope, window, cx)
                        }))
                        .on_key_down(cx.listener(move |menu, event: &KeyDownEvent, window, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space" | "right") {
                                menu.switch_scope(scope, window, cx);
                                window.prevent_default();
                                cx.stop_propagation();
                            }
                        }))
                }),
            )
    }

    fn results(
        &self,
        colors: &MenuColors,
        active: Option<Provider>,
        handoff: Option<Provider>,
        sources: &[(Provider, Entity<ModelPicker>)],
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let input = self.query.read(cx).value().to_lowercase();
        let terms: Vec<_> = input.split_whitespace().collect();
        let scope = if terms.is_empty() {
            self.scope
        } else {
            Scope::All
        };
        let mut list = div()
            .id("model-results")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .px(px(10.))
            .pt(px(7.))
            .pb(px(9.));
        let other = if self.first == Provider::Codex {
            Provider::OpenCode
        } else {
            Provider::Codex
        };
        let mut groups = 0;
        let mut rows = 0;
        for provider in [self.first, other] {
            if matches!(scope, Scope::Provider(only) if only != provider) {
                continue;
            }
            let Some((_, source)) = sources.iter().find(|(kind, _)| *kind == provider) else {
                if scope == Scope::Provider(provider) {
                    list = list.child(status(
                        "Provider unavailable",
                        "Install and authenticate this provider to use it.",
                        colors,
                    ));
                }
                continue;
            };
            let picker = source.read(cx);
            if scope == Scope::All
                && picker.provider.is_none()
                && picker
                    .catalog()
                    .is_some_and(|catalog| catalog.options.is_err())
            {
                continue;
            }
            let catalog = picker.catalog();
            let mut models = Vec::new();
            if let Some(catalog) = catalog {
                if scope == Scope::Favorites {
                    models.extend(catalog.preferences.favorites.iter().map(|id| {
                        catalog
                            .models()
                            .iter()
                            .find(|model| model.id == *id)
                            .map_or_else(
                                || {
                                    let name =
                                        format_model_name(id.rsplit('/').next().unwrap_or(id));
                                    let detail = if provider == Provider::OpenCode {
                                        open_code_source(id)
                                    } else {
                                        id.clone()
                                    };
                                    (None, id.clone(), name, Some(detail))
                                },
                                |model| {
                                    (
                                        Some(model),
                                        model.id.clone(),
                                        model.name.clone(),
                                        model.disambiguator.clone(),
                                    )
                                },
                            )
                    }));
                } else {
                    models.extend(
                        catalog
                            .models()
                            .iter()
                            .filter(|model| model.matches_search(provider_label(provider), &terms))
                            .map(|model| {
                                (
                                    Some(model),
                                    model.id.clone(),
                                    model.name.clone(),
                                    model.disambiguator.clone(),
                                )
                            }),
                    );
                }
            }
            let notice = if scope == Scope::Favorites
                && picker.provider.is_none()
                && models.is_empty()
                && catalog.is_some()
                && picker.save_error.is_none()
            {
                None
            } else {
                self.provider_status(source, colors, cx)
            };
            if models.is_empty() && notice.is_none() {
                continue;
            }
            let mut group = div().when(groups > 0, |group| {
                group
                    .mt(px(5.))
                    .pt(px(4.))
                    .border_t_1()
                    .border_color(rgb(colors.divider))
            });
            if matches!(scope, Scope::All | Scope::Favorites) {
                group = group.child(
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
                        .child(provider_logo(provider, 14., colors.muted))
                        .child(provider_label(provider)),
                );
            }
            for (model, id, name, detail) in models {
                let favorite =
                    catalog.is_some_and(|catalog| catalog.preferences.favorites.contains(&id));
                let selected = active == Some(provider)
                    && catalog.is_some_and(|catalog| {
                        catalog.preferences.model.as_deref() == Some(id.as_str())
                    });
                let can_choose = model.is_some() && picker.loading.is_none();
                let detail = if model.is_none() {
                    Some(detail.map_or_else(
                        || "Unavailable".to_owned(),
                        |detail| format!("{detail} · Unavailable"),
                    ))
                } else {
                    detail
                };
                group = group.child(self.model_row(
                    source, provider, id, name, detail, can_choose, favorite, selected, handoff,
                    colors, cx,
                ));
                rows += 1;
            }
            groups += 1;
            list = list.child(group.children(notice));
        }
        if rows == 0 && !terms.is_empty() {
            list = list.child(
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
                ),
            );
        } else if groups == 0 && scope == Scope::Favorites {
            list = list.child(status(
                "No favorites yet",
                "Use the star beside a model to find it here.",
                colors,
            ));
        } else if groups == 0 && scope == Scope::All {
            list = list.child(status(
                "No providers found",
                "Install and authenticate Codex or OpenCode to use the assistant.",
                colors,
            ));
        }
        list
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

    fn model_row(
        &self,
        source: &Entity<ModelPicker>,
        provider: Provider,
        id: String,
        name: String,
        detail: Option<String>,
        available: bool,
        favorite: bool,
        active: bool,
        handoff: Option<Provider>,
        colors: &MenuColors,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let owner = self.owner.clone();
        let chooser = source.clone();
        let starred = source.downgrade();
        let choice_id = id.clone();
        let star_id = id.clone();
        let row_id = format!("model/{}/{id}", provider.as_str());
        let star_label = if favorite {
            format!("Remove {name} from favorites")
        } else {
            format!("Add {name} to favorites")
        };
        let text = colors.base.text;
        let hover = colors.selected;
        div()
            .id(SharedString::from(row_id.clone()))
            .debug_selector(move || row_id.clone())
            .flex()
            .items_center()
            .my(px(1.))
            .pr(px(5.))
            .rounded(px(5.))
            .when(active, |row| row.bg(rgb(colors.selected)))
            .child(
                Button::new("choose-model")
                    .ghost()
                    .with_size(gpui_component::Size::Size(px(33.)))
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(33.))
                    .justify_start()
                    .gap(px(7.))
                    .px(px(6.))
                    .disabled(!available)
                    .tooltip(name.clone())
                    .child(provider_logo(provider, 16., colors.muted))
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
                                                .text_color(rgb(colors.muted))
                                                .child("✓"),
                                        )
                                    }),
                            )
                            .children(detail.map(|detail| {
                                div()
                                    .text_size(px(11.))
                                    .line_height(px(16.5))
                                    .text_color(rgb(colors.muted))
                                    .child(detail)
                            })),
                    )
                    .when(
                        available && handoff.is_some_and(|current| current != provider),
                        |button| {
                            button.child(
                                div()
                                    .ml_auto()
                                    .text_size(px(10.))
                                    .text_color(rgb(colors.muted))
                                    .child("Handoff"),
                            )
                        },
                    )
                    .on_click(move |_, _, app| {
                        let current = owner
                            .read_with(app, |library, _| {
                                library.selection.as_ref().is_some_and(|selection| {
                                    selection.conversation.running.is_none()
                                        && selection.conversation.models.get(&provider)
                                            == Some(&chooser)
                                })
                            })
                            .unwrap_or(false);
                        if !current {
                            return;
                        }
                        chooser.update(app, |picker, cx| picker.choose(&choice_id, cx));
                        let _ = owner.update(app, |library, cx| {
                            if let Some(selection) = &mut library.selection
                                && selection.conversation.provider != Some(provider)
                            {
                                selection.conversation.provider = Some(provider);
                                library.schedule_chat_draft_save(cx);
                                cx.notify();
                            }
                        });
                    }),
            )
            .child(
                div()
                    .id("toggle-favorite")
                    .tab_index(2)
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
                    .text_color(rgb(if favorite {
                        colors.base.warning
                    } else {
                        colors.muted
                    }))
                    .hover(move |style| style.bg(rgb(hover)).text_color(rgb(text)))
                    .tooltip(move |window, cx| Tooltip::new(star_label.clone()).build(window, cx))
                    .child(if favorite { "★" } else { "☆" })
                    .on_click({
                        let source = starred.clone();
                        let id = star_id.clone();
                        move |_, _, app| {
                            let _ = source.update(app, |picker, cx| picker.favorite(&id, cx));
                        }
                    })
                    .on_key_down(cx.listener(move |_, event: &KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            let _ = starred.update(cx, |picker, cx| picker.favorite(&star_id, cx));
                            window.prevent_default();
                            cx.stop_propagation();
                        }
                    })),
            )
    }

    fn details(
        &self,
        source: Option<&Entity<ModelPicker>>,
        colors: &MenuColors,
        width: Pixels,
        cx: &Context<Self>,
    ) -> impl IntoElement {
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
        pane = pane
            .child(
                div()
                    .min_h(px(39.))
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(colors.divider))
                    .text_size(px(13.))
                    .font_weight(FontWeight(520.))
                    .child(if codex { "Reasoning" } else { "Variant" }),
            )
            .child(
                div()
                    .py(px(6.))
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(colors.muted))
                    .child(model.name.clone()),
            );
        for effort in &model.efforts {
            let value = effort.clone();
            let pick = source.downgrade();
            let selected = catalog.preferences.reasoning_effort.as_ref() == Some(effort);
            pane = pane.child(
                Button::new(SharedString::from(format!("effort/{effort}")))
                    .ghost()
                    .with_size(gpui_component::Size::Size(px(33.)))
                    .w_full()
                    .h(px(33.))
                    .justify_start()
                    .when(selected, |button| button.bg(rgb(colors.selected)))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight(520.))
                            .child(title_case(effort)),
                    )
                    .when(model.default_effort.as_ref() == Some(effort), |button| {
                        button.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(colors.muted))
                                .child("Default"),
                        )
                    })
                    .when(selected, |button| {
                        button.child(
                            div()
                                .ml_auto()
                                .text_size(px(11.))
                                .text_color(rgb(colors.muted))
                                .child("✓"),
                        )
                    })
                    .on_click(move |_, _, app| {
                        let _ = pick.update(app, |picker, cx| picker.choose_effort(&value, cx));
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
        let colors = MenuColors::for_window(window);
        let (position, dimensions) = popup_geometry(anchor, window.viewport_size());
        let compact = dimensions.width < px(470.);
        let searching = !self.query.read(cx).value().trim().is_empty();
        let owner = self.owner.clone();
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
                            .on_click(cx.listener(|menu, _, _, cx| {
                                menu.details = !menu.details;
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
            .capture_key_down(cx.listener(move |_, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    let _ =
                        owner.update(cx, |library, cx| library.close_model_menu(true, window, cx));
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }))
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
                    .when(!searching && !(compact && self.details), |panel| {
                        panel.child(self.rail(&colors, compact, cx))
                    })
                    .when(!(compact && self.details), |panel| {
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
                                .child(self.results(&colors, active, handoff, &sources, cx)),
                        )
                    })
                    .when(!compact || self.details, |panel| {
                        panel.child(self.details(active_source, &colors, settings_width, cx))
                    }),
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
        let scope = active.map_or(Scope::All, |provider| {
            if pane
                .models
                .get(&provider)
                .and_then(|source| source.read(cx).catalog())
                .is_some_and(|catalog| {
                    catalog
                        .preferences
                        .model
                        .as_ref()
                        .is_some_and(|id| catalog.preferences.favorites.contains(id))
                })
            {
                Scope::Favorites
            } else {
                Scope::Provider(provider)
            }
        });
        if cfg!(not(all(test, feature = "ui-tests"))) {
            for kind in PROVIDERS {
                self.prepare_model_catalog(kind, cx);
            }
        }
        let parent = cx.weak_entity();
        let menu = cx.new(|cx| ModelMenu::new(parent, active, scope, window, cx));
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

    pub(crate) fn model_menu_layer(&self) -> Option<impl IntoElement> {
        self.selection.as_ref()?.conversation.model_menu.clone()
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
                    .map(|provider| provider_logo(provider, 16., palette.text)),
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
                                efforts: Vec::new(),
                                default_effort: None,
                                is_default: true,
                            }]),
                            preferences: AssistantPreferences {
                                model: Some(id.into()),
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
        cx.simulate_input("Research");
        cx.run_until_parked();
        cx.refresh().unwrap();
        assert!(cx.debug_bounds("filter-favorites").is_none());
        assert!(cx.debug_bounds("model/codex/research-codex").is_some());
        assert!(cx.debug_bounds("model/opencode/openai/research").is_some());
        cx.update(|_, app| {
            assert_eq!(
                library
                    .read(app)
                    .selection
                    .as_ref()
                    .unwrap()
                    .conversation
                    .provider,
                Some(Provider::Codex)
            )
        });

        click(cx, "model/opencode/openai/research");
        cx.update(|window, app| {
            let pane = &library.read(app).selection.as_ref().unwrap().conversation;
            assert_eq!(pane.provider, Some(Provider::OpenCode));
            let input = pane.model_menu.as_ref().unwrap().read(app).query.clone();
            input.update(app, |input, cx| input.focus(window, cx));
        });
        cx.simulate_keystrokes("escape");
        cx.update(|window, app| {
            let pane = &library.read(app).selection.as_ref().unwrap().conversation;
            assert!(pane.model_menu.is_none());
            assert!(pane.model_trigger.as_ref().unwrap().is_focused(window));
        });
    }
}
