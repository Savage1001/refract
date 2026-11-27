use super::{
    ConversationPane, Library, Palette,
    assistant::{InstalledProvider, PreparedProvider},
};
use crate::account_settings_view::AccountAction;
use gpui::{
    Context, Corner, Entity, FontWeight, Render, Subscription, Task, WeakEntity, Window, div,
    prelude::*, px, rgb,
};
use gpui_component::{
    Disableable, Selectable, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    popover::Popover,
};
use refract_native::{
    assistant_preferences::{
        AssistantConnection, AssistantPreferences, load_assistant_preferences,
        save_assistant_preferences, valid_codex_effort,
    },
    assistant_runtime::{
        codex_catalog::discover_codex_models, opencode::catalog::discover_opencode_models,
    },
    conversation::Provider,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

struct Model {
    id: String,
    name: String,
    identity: String,
    description: String,
    efforts: Vec<String>,
    default_effort: Option<String>,
    is_default: bool,
}

struct Catalog {
    options: Vec<Model>,
    preferences: AssistantPreferences,
}

struct ModelLoad {
    cancel: Arc<AtomicBool>,
    _task: Task<()>,
}

impl Drop for ModelLoad {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

pub(super) struct ModelPicker {
    provider: InstalledProvider,
    project: PathBuf,
    profile: PathBuf,
    parent: WeakEntity<Library>,
    query: Entity<InputState>,
    _query_events: Subscription,
    favorites: bool,
    catalog: Option<Result<Catalog, String>>,
    loading: Option<ModelLoad>,
    save_error: Option<String>,
    pending_save: Option<AssistantPreferences>,
}

impl ModelPicker {
    fn new(
        provider: InstalledProvider,
        project: PathBuf,
        profile: PathBuf,
        parent: WeakEntity<Library>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search models"));
        let events = cx.subscribe(&query, |_, _, _: &InputEvent, cx| cx.notify());
        Self {
            provider,
            project,
            profile,
            parent,
            query,
            _query_events: events,
            favorites: false,
            catalog: None,
            loading: None,
            save_error: None,
            pending_save: None,
        }
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        if self.loading.is_some() {
            return;
        }
        let provider = self.provider.clone();
        let project = self.project.clone();
        let profile = self.profile.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let job_cancel = cancel.clone();
        let checkout = cx.background_executor().spawn(async move {
            let preferences = load_assistant_preferences(&profile, provider.mode())
                .map_err(|error| format!("Couldn’t load model preferences. {error}"))?;
            let options = load_models(&provider, &project, &job_cancel)?;
            Ok(Catalog {
                options,
                preferences,
            })
        });
        let task = cx.spawn(async move |picker, cx| {
            let result = checkout.await;
            let _ = picker.update(cx, |picker, cx| {
                picker.loading = None;
                picker.catalog = Some(result);
                picker.update_parent(cx);
            });
        });
        self.loading = Some(ModelLoad {
            cancel,
            _task: task,
        });
        cx.notify();
    }

    fn catalog(&self) -> Option<&Catalog> {
        self.catalog.as_ref().and_then(|state| state.as_ref().ok())
    }

    fn selected(catalog: &Catalog) -> Option<&Model> {
        if let Some(id) = catalog.preferences.model.as_deref() {
            catalog.options.iter().find(|model| model.id == id)
        } else {
            catalog
                .options
                .iter()
                .find(|model| model.is_default)
                .or_else(|| catalog.options.first())
        }
    }

    fn update_parent(&self, cx: &mut Context<Self>) {
        cx.notify();
        let _ = self.parent.update(cx, |_, cx| cx.notify());
    }

    fn choose(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(catalog) = self.catalog() else {
            return;
        };
        let Some(model) = catalog.options.iter().find(|model| model.id == id) else {
            return;
        };
        let mut next = catalog.preferences.clone();
        if next.model.as_deref() != Some(id) {
            next.model = Some(id.into());
            next.reasoning_effort = model.default_effort.clone();
        }
        self.save(next, cx);
    }

    fn favorite(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(catalog) = self.catalog() else {
            return;
        };
        let mut next = catalog.preferences.clone();
        if let Some(index) = next.favorites.iter().position(|value| value == id) {
            next.favorites.remove(index);
        } else {
            next.favorites.push(id.to_owned());
        }
        self.save(next, cx);
    }

    fn choose_effort(&mut self, effort: &str, cx: &mut Context<Self>) {
        let Some(catalog) = self.catalog() else {
            return;
        };
        let mut next = catalog.preferences.clone();
        if next.model.is_none() {
            next.model = Self::selected(catalog).map(|model| model.id.clone());
        }
        next.reasoning_effort = Some(effort.into());
        self.save(next, cx);
    }

    fn toggle_fast_mode(&mut self, cx: &mut Context<Self>) {
        let Some(catalog) = self.catalog() else {
            return;
        };
        let mut next = catalog.preferences.clone();
        next.fast_mode = Some(!next.fast_mode.unwrap_or(false));
        self.save(next, cx);
    }

    fn save(&mut self, preferences: AssistantPreferences, cx: &mut Context<Self>) {
        if self
            .parent
            .read_with(cx, |parent, _| {
                parent
                    .selection
                    .as_ref()
                    .is_some_and(|selection| selection.conversation.running.is_some())
            })
            .unwrap_or(true)
        {
            return;
        }
        if let Err(error) =
            save_assistant_preferences(&self.profile, self.provider.mode(), &preferences)
        {
            self.save_error = Some(format!("Couldn’t save model preferences. {error}"));
            self.pending_save = Some(preferences);
        } else if let Some(Ok(catalog)) = &mut self.catalog {
            catalog.preferences = preferences;
            self.save_error = None;
            self.pending_save = None;
        }
        self.update_parent(cx);
    }

    fn label(&self) -> String {
        match self.catalog.as_ref() {
            None => self.provider.label().into(),
            Some(Err(error)) if self.needs_account_setup(error) => "Sign in".into(),
            Some(Err(_)) => "Models unavailable".into(),
            Some(Ok(catalog)) => Self::selected(catalog)
                .map(|model| model.name.clone())
                .or_else(|| catalog.preferences.model.clone())
                .unwrap_or_else(|| self.provider.label().into()),
        }
    }

    fn needs_account_setup(&self, error: &str) -> bool {
        self.provider.mode() == AssistantConnection::CodexAccount
            && (error.starts_with("Download the Codex runtime")
                || error.starts_with("Sign in with ChatGPT")
                || error.starts_with("Your ChatGPT session ended.")
                || error.starts_with("ChatGPT plan usage was not granted."))
    }

    pub(super) fn ready_to_send(&self) -> bool {
        self.loading.is_none()
            && self.pending_save.is_none()
            && !self.catalog.as_ref().is_some_and(|result| {
                result
                    .as_ref()
                    .err()
                    .is_some_and(|error| self.needs_account_setup(error))
            })
    }
}

fn load_models(
    provider: &InstalledProvider,
    project: &Path,
    cancel: &AtomicBool,
) -> Result<Vec<Model>, String> {
    match provider
        .prepare(cancel)
        .map_err(|error| error.to_string())?
    {
        PreparedProvider::Codex(codex) => {
            let command = codex.command(project).map_err(|error| error.to_string())?;
            discover_codex_models(command, codex.key().is_some(), cancel).map(|models| {
                models
                    .into_iter()
                    .map(|model| {
                        let efforts: Vec<_> = model
                            .supported_reasoning_efforts
                            .into_iter()
                            .map(|effort| effort.effort)
                            .filter(|effort| valid_codex_effort(effort))
                            .collect();
                        let default_effort =
                            preferred_effort(&efforts, model.default_reasoning_effort.as_deref());
                        Model {
                            id: model.id,
                            name: model.display_name,
                            identity: model.model,
                            description: model.description,
                            default_effort,
                            efforts,
                            is_default: model.is_default,
                        }
                    })
                    .collect()
            })
        }
        PreparedProvider::OpenCode(cli) => {
            discover_opencode_models(cli, project, cancel).map(|models| {
                models
                    .into_iter()
                    .map(|model| Model {
                        identity: model.id.clone(),
                        id: model.id,
                        name: model.name,
                        description: String::new(),
                        default_effort: preferred_effort(&model.variants, None),
                        efforts: model.variants,
                        is_default: false,
                    })
                    .collect()
            })
        }
    }
}

fn preferred_effort(efforts: &[String], default: Option<&str>) -> Option<String> {
    let advertised = |name: &str| efforts.iter().find(|effort| effort.as_str() == name);
    advertised("low")
        .or_else(|| default.and_then(advertised))
        .or_else(|| {
            ["none", "minimal", "medium", "high", "xhigh", "max", "ultra"]
                .into_iter()
                .find_map(advertised)
        })
        .or_else(|| efforts.first())
        .cloned()
}

impl Render for ModelPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_window(window);
        let width = (f32::from(window.viewport_size().width) - 32.).clamp(260., 520.);
        let query = self.query.read(cx).value().to_lowercase();
        let controls = div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                Button::new("all-models")
                    .ghost()
                    .small()
                    .selected(!self.favorites)
                    .label("All models")
                    .on_click(cx.listener(|picker, _, _, cx| {
                        picker.favorites = false;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("favorite-models")
                    .ghost()
                    .small()
                    .selected(self.favorites)
                    .label("Favorites")
                    .on_click(cx.listener(|picker, _, _, cx| {
                        picker.favorites = true;
                        cx.notify();
                    })),
            );
        let mut list = div()
            .id("model-results")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .px_1();
        let mut options = div()
            .id("model-options")
            .w(px(176.))
            .flex_none()
            .border_l_1()
            .border_color(rgb(palette.border))
            .pl_2()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .gap_1()
            .text_size(px(12.));
        match self.catalog.as_ref() {
            None if self.loading.is_none() => {
                list = list.child("Checking model availability…");
            }
            _ if self.loading.is_some() => {
                list = list.child("Loading models…");
            }
            Some(Err(error)) if self.needs_account_setup(error) => {
                let owner = self.parent.clone();
                let busy = owner
                    .read_with(cx, |library, _| library.settings.account.busy())
                    .unwrap_or(false);
                list = list
                    .child("Codex requires sign-in")
                    .child(error.clone())
                    .child(
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
                                    );
                                });
                            }),
                    );
            }
            Some(Err(error)) => {
                list = list.child(error.clone()).child(
                    Button::new("retry-models")
                        .ghost()
                        .small()
                        .label("Retry")
                        .on_click(cx.listener(|picker, _, _, cx| picker.load(cx))),
                );
            }
            Some(Ok(catalog)) => {
                let selected = Self::selected(catalog);
                let terms: Vec<_> = query.split_whitespace().collect();
                let filtered: Vec<_> = catalog
                    .options
                    .iter()
                    .filter(|model| {
                        let favorite = catalog.preferences.favorites.contains(&model.id);
                        if self.favorites && !favorite {
                            return false;
                        }
                        let name = model.name.to_lowercase();
                        let identity = model.identity.to_lowercase();
                        terms
                            .iter()
                            .all(|term| name.contains(*term) || identity.contains(*term))
                    })
                    .collect();
                if filtered.is_empty() {
                    list = list.child(if query.is_empty() && self.favorites {
                        "No favorites yet"
                    } else {
                        "No matching models"
                    });
                }
                for model in filtered {
                    let active = selected.is_some_and(|selected| selected.id == model.id);
                    let favorite = catalog.preferences.favorites.contains(&model.id);
                    let choose = model.id.clone();
                    let star = choose.clone();
                    list = list.child(
                        div()
                            .id(gpui::SharedString::from(model.id.clone()))
                            .flex()
                            .items_center()
                            .gap_1()
                            .rounded(px(5.))
                            .when(active, |row| row.bg(rgb(palette.selection)))
                            .child(
                                Button::new("select-model")
                                    .ghost()
                                    .small()
                                    .w_full()
                                    .child(
                                        div()
                                            .text_size(px(14.))
                                            .font_weight(FontWeight(580.))
                                            .truncate()
                                            .child(model.name.clone()),
                                    )
                                    .when(!model.description.is_empty(), |button| {
                                        button.tooltip(model.description.clone())
                                    })
                                    .on_click(cx.listener(move |picker, _, _, cx| {
                                        picker.choose(&choose, cx)
                                    })),
                            )
                            .child(
                                Button::new("favorite-model")
                                    .ghost()
                                    .small()
                                    .label(if favorite { "★" } else { "☆" })
                                    .tooltip(if favorite {
                                        "Remove from favorites"
                                    } else {
                                        "Add to favorites"
                                    })
                                    .on_click(cx.listener(move |picker, _, _, cx| {
                                        picker.favorite(&star, cx)
                                    })),
                            ),
                    );
                }
                if let Some(model) = selected {
                    let codex = self.provider.kind() == Provider::Codex;
                    options = options
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight(520.))
                                .child(if codex { "Reasoning" } else { "Variant" }),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .child(model.name.clone()),
                        );
                    for effort in &model.efforts {
                        let value = effort.clone();
                        let current = catalog
                            .preferences
                            .reasoning_effort
                            .as_ref()
                            .or(model.default_effort.as_ref());
                        options = options.child(
                            Button::new(gpui::SharedString::from(format!("effort/{effort}")))
                                .ghost()
                                .small()
                                .selected(current == Some(effort))
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight(520.))
                                        .child(title_case(effort)),
                                )
                                .on_click(cx.listener(move |picker, _, _, cx| {
                                    picker.choose_effort(&value, cx)
                                })),
                        );
                    }
                    if codex {
                        let fast = catalog.preferences.fast_mode.unwrap_or(false);
                        options = options.child(
                            Button::new("fast-mode")
                                .ghost()
                                .small()
                                .selected(fast)
                                .label(if fast {
                                    "Fast mode on"
                                } else {
                                    "Fast mode off"
                                })
                                .on_click(
                                    cx.listener(|picker, _, _, cx| picker.toggle_fast_mode(cx)),
                                ),
                        );
                    }
                }
            }
            None => {}
        }
        div()
            .w(px(width))
            .h(px(370.))
            .flex()
            .flex_col()
            .gap_2()
            .text_size(px(13.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w(px(0.)).child(Input::new(&self.query)))
                    .child(controls),
            )
            .children(self.save_error.clone().map(|error| {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_color(rgb(palette.warning))
                    .child(error)
                    .child(
                        Button::new("retry-model-save")
                            .ghost()
                            .small()
                            .label("Retry saving")
                            .on_click(cx.listener(|picker, _, _, cx| {
                                if let Some(preferences) = picker.pending_save.clone() {
                                    picker.save(preferences, cx);
                                }
                            })),
                    )
            }))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.))
                    .child(list)
                    .child(options),
            )
    }
}

impl Library {
    pub(crate) fn refresh_account_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(selection) = &mut self.selection {
            selection.conversation.models.remove(&Provider::Codex);
            self.prepare_model_picker(window, cx);
        }
    }

    pub(super) fn prepare_model_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        let pane = &mut selection.conversation;
        let Some(kind) = pane.provider else {
            return;
        };
        if let Some(picker) = pane.models.get(&kind) {
            if picker.read(cx).catalog.is_none() {
                picker.update(cx, |picker, cx| picker.load(cx));
            }
            return;
        }
        let Some(provider) = pane
            .providers
            .iter()
            .find(|provider| provider.kind() == kind)
            .cloned()
        else {
            return;
        };
        let Some(profile) = self.projects_dir.parent() else {
            return;
        };
        let workspace = self.projects_dir.join(&selection.project.id);
        let parent = cx.weak_entity();
        let picker = cx.new(|cx| {
            ModelPicker::new(provider, workspace, profile.to_owned(), parent, window, cx)
        });
        pane.models.insert(kind, picker.clone());
        picker.update(cx, |picker, cx| picker.load(cx));
        cx.notify();
    }

    pub(super) fn model_picker_button(
        &self,
        pane: &ConversationPane,
        palette: Palette,
        cx: &Context<Self>,
    ) -> Option<Popover> {
        let kind = pane.provider?;
        let provider = pane
            .providers
            .iter()
            .find(|provider| provider.kind() == kind)?;
        let name = pane
            .models
            .get(&kind)
            .map(|picker| picker.read(cx).label())
            .unwrap_or_else(|| provider.label().into());
        let effort = pane
            .models
            .get(&kind)
            .and_then(|picker| picker.read(cx).catalog())
            .and_then(|catalog| {
                let selected = ModelPicker::selected(catalog)?;
                catalog
                    .preferences
                    .reasoning_effort
                    .as_ref()
                    .or(selected.default_effort.as_ref())
                    .map(|effort| title_case(effort))
            });
        let library = cx.weak_entity();
        let content = library.clone();
        Some(
            Popover::new("conversation-models")
                .anchor(Corner::BottomRight)
                .trigger(
                    Button::new("model-picker-trigger")
                        .ghost()
                        .small()
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(name),
                        )
                        .children(effort.map(|effort| {
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(palette.muted))
                                .truncate()
                                .child(effort)
                        }))
                        .disabled(pane.running.is_some()),
                )
                .on_open_change(move |open, window, app| {
                    if *open {
                        let _ = library
                            .update(app, |library, cx| library.prepare_model_picker(window, cx));
                    }
                })
                .content(move |_, _, cx| {
                    let picker = content
                        .read_with(cx, |library, _| {
                            library
                                .selection
                                .as_ref()?
                                .conversation
                                .models
                                .get(&kind)
                                .cloned()
                        })
                        .ok()
                        .flatten();
                    div().children(picker)
                }),
        )
    }
}

fn title_case(value: &str) -> String {
    if value == "xhigh" {
        return "Extra high".into();
    }
    let value = value.replace(['-', '_'], " ");
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return String::new();
    };
    first.to_uppercase().chain(characters).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooses_the_reference_effort_order_and_labels() {
        let supported = vec!["high".to_owned(), "low".to_owned(), "medium".to_owned()];
        assert_eq!(
            preferred_effort(&supported, Some("high")).as_deref(),
            Some("low")
        );
        assert_eq!(
            preferred_effort(&supported[..1], Some("unknown")).as_deref(),
            Some("high")
        );
        assert_eq!(preferred_effort(&[], None), None);
        assert_eq!(title_case("xhigh"), "Extra high");
        assert_eq!(title_case("extra_deep"), "Extra deep");
    }
}
