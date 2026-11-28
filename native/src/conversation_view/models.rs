use super::{
    ConversationPane, Library, Palette,
    assistant::{InstalledProvider, PreparedProvider},
};
use crate::account_settings_view::AccountAction;
use gpui::{
    Context, Corner, Entity, FontWeight, Render, Subscription, Task, WeakEntity, Window, div,
    prelude::*, px, rgb, svg,
};
use gpui_component::{
    Disableable, Selectable, Sizable, Size,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    popover::Popover,
};
use icu_collator::{
    Collator, CollatorBorrowed, CollatorPreferences,
    options::{CollatorOptions, Strength},
    preferences::CollationNumericOrdering,
};
use icu_locale_core::Locale;
use refract_native::{
    assistant_preferences::{
        AssistantConnection, AssistantPreferences, load_assistant_preferences,
        save_assistant_preferences, valid_codex_effort,
    },
    assistant_runtime::{
        codex_catalog::{CodexCatalogModel, discover_codex_models},
        opencode::catalog::{OpenCodeCatalogModel, discover_opencode_models},
    },
    conversation::Provider,
};
use regex::Regex;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) const CHECKING_MODELS: &str = "Checking model availability…";

struct Model {
    id: String,
    name: String,
    source: Option<String>,
    disambiguator: Option<String>,
    legacy_ids: Vec<String>,
    description: String,
    efforts: Vec<String>,
    default_effort: Option<String>,
    is_default: bool,
}

impl Model {
    fn matches_search(&self, provider: &str, terms: &[&str]) -> bool {
        if terms.is_empty() {
            return true;
        }
        let name = self.name.to_lowercase();
        let text = format!(
            "{name} {} {} {} {provider}",
            self.id,
            self.source.as_deref().unwrap_or_default(),
            self.description,
        )
        .to_lowercase();
        terms.iter().all(|&term| {
            text.contains(term) || {
                let normalized = format_model_name(term);
                !normalized.is_empty() && name.contains(&normalized)
            }
        })
    }
}

struct Catalog {
    options: Result<Vec<Model>, DiscoveryError>,
    preferences: AssistantPreferences,
}

enum DiscoveryError {
    Connection(String),
    Catalog(String),
}

impl DiscoveryError {
    fn message(&self) -> &str {
        match self {
            Self::Connection(message) | Self::Catalog(message) => message,
        }
    }
}

impl Catalog {
    fn models(&self) -> &[Model] {
        self.options.as_deref().unwrap_or_default()
    }

    fn selected(&self) -> Option<&Model> {
        let id = self.preferences.model.as_deref()?;
        self.models().iter().find(|model| model.id == id)
    }

    fn resolve_id(&self, id: &str) -> Option<&Model> {
        if let Some(exact) = self.models().iter().find(|model| model.id == id) {
            return Some(exact);
        }
        let mut aliases = self
            .models()
            .iter()
            .filter(|model| model.legacy_ids.iter().any(|legacy_id| legacy_id == id));
        let model = aliases.next()?;
        aliases.next().is_none().then_some(model)
    }

    fn normalize_preferences(&self) -> Option<(AssistantPreferences, bool)> {
        if self.models().is_empty() {
            return None;
        }
        let model = match self.preferences.model.as_deref() {
            Some(id) => self.resolve_id(id),
            None => self
                .models()
                .iter()
                .find(|model| model.is_default)
                .or_else(|| self.models().first()),
        };
        let mut preferences = self.preferences.clone();
        if let Some(model) = model {
            preferences.model = Some(model.id.clone());
            if preferences
                .reasoning_effort
                .as_ref()
                .is_none_or(|effort| !model.efforts.contains(effort))
            {
                preferences.reasoning_effort = model.default_effort.clone();
            }
        }
        let affects_selection = preferences.model != self.preferences.model
            || preferences.reasoning_effort != self.preferences.reasoning_effort;
        if self
            .models()
            .iter()
            .any(|model| !model.legacy_ids.is_empty())
        {
            let mut seen = HashSet::new();
            preferences.favorites.clear();
            for id in &self.preferences.favorites {
                let canonical = self.resolve_id(id).map_or(id, |model| &model.id);
                if seen.insert(canonical) {
                    preferences.favorites.push(canonical.clone());
                }
            }
        }
        (preferences != self.preferences).then_some((preferences, affects_selection))
    }

    fn send_issue(&self, provider: Provider, settings_unsaved: bool) -> Option<&'static str> {
        if settings_unsaved {
            return Some("Retry saving model settings");
        }
        match &self.options {
            Err(DiscoveryError::Catalog(_))
                if provider == Provider::Codex && self.preferences.model.is_some() =>
            {
                None
            }
            Err(_) => Some(match provider {
                Provider::Codex => "Retry loading Codex models",
                Provider::OpenCode => "Retry loading OpenCode models",
            }),
            Ok(_) => match self.selected() {
                None => Some(match provider {
                    Provider::Codex => "Choose an available Codex model",
                    Provider::OpenCode => "Choose an available OpenCode model",
                }),
                Some(model)
                    if self
                        .preferences
                        .reasoning_effort
                        .as_ref()
                        .is_some_and(|effort| !model.efforts.contains(effort)) =>
                {
                    Some(match provider {
                        Provider::Codex => "Choose an available reasoning effort",
                        Provider::OpenCode => "Choose an available model variant",
                    })
                }
                Some(_) => None,
            },
        }
    }

    fn selection(&self, provider: Provider) -> Option<ModelSelection> {
        Some(ModelSelection {
            model: self.preferences.model.clone()?,
            effort: self.preferences.reasoning_effort.clone(),
            fast_mode: (provider == Provider::Codex)
                .then_some(self.preferences.fast_mode.unwrap_or(false)),
        })
    }
}

pub(super) struct ModelSelection {
    pub(super) model: String,
    pub(super) effort: Option<String>,
    pub(super) fast_mode: Option<bool>,
}

struct SaveFailure {
    message: String,
    affects_selection: bool,
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
    save_error: Option<SaveFailure>,
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
        }
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        if self.loading.is_some() {
            return;
        }
        let provider = self.provider.clone();
        let project = self.project.clone();
        let profile = self.profile.clone();
        let preferences = self.catalog().map(|catalog| catalog.preferences.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        let job_cancel = cancel.clone();
        let checkout = cx.background_executor().spawn(async move {
            let preferences = preferences
                .map_or_else(|| load_assistant_preferences(&profile, provider.mode()), Ok)
                .map_err(|error| format!("Couldn’t load model preferences. {error}"))?;
            let options = load_models(&provider, &project, &job_cancel);
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
                if let Some((preferences, affects_selection)) =
                    picker.catalog().and_then(Catalog::normalize_preferences)
                {
                    picker.save(preferences, affects_selection, cx);
                }
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

    fn update_parent(&self, cx: &mut Context<Self>) {
        cx.notify();
        let _ = self.parent.update(cx, |_, cx| cx.notify());
    }

    fn choose(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(catalog) = self.catalog() else {
            return;
        };
        let Some(model) = catalog.models().iter().find(|model| model.id == id) else {
            return;
        };
        if catalog.preferences.model.as_deref() == Some(id) {
            return;
        }
        let mut next = catalog.preferences.clone();
        next.model = Some(id.into());
        next.reasoning_effort = model.default_effort.clone();
        self.save(next, true, cx);
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
        self.save(next, false, cx);
    }

    fn choose_effort(&mut self, effort: &str, cx: &mut Context<Self>) {
        let Some(catalog) = self.catalog().filter(|catalog| {
            catalog
                .selected()
                .is_some_and(|model| model.efforts.iter().any(|candidate| candidate == effort))
        }) else {
            return;
        };
        if catalog.preferences.reasoning_effort.as_deref() == Some(effort) {
            return;
        }
        let mut next = catalog.preferences.clone();
        next.reasoning_effort = Some(effort.into());
        self.save(next, true, cx);
    }

    fn toggle_fast_mode(&mut self, cx: &mut Context<Self>) {
        let Some(catalog) = self.catalog() else {
            return;
        };
        let mut next = catalog.preferences.clone();
        next.fast_mode = Some(!next.fast_mode.unwrap_or(false));
        self.save(next, true, cx);
    }

    fn save(
        &mut self,
        preferences: AssistantPreferences,
        affects_selection: bool,
        cx: &mut Context<Self>,
    ) {
        if self.loading.is_some()
            || self
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
        let Some(Ok(catalog)) = &mut self.catalog else {
            return;
        };
        catalog.preferences = preferences;
        let affects_selection = affects_selection
            || self
                .save_error
                .as_ref()
                .is_some_and(|error| error.affects_selection);
        self.save_error =
            save_assistant_preferences(&self.profile, self.provider.mode(), &catalog.preferences)
                .err()
                .map(|error| SaveFailure {
                    message: format!("Couldn’t save model preferences. {error}"),
                    affects_selection,
                });
        self.update_parent(cx);
    }

    fn label(&self) -> String {
        match self.catalog.as_ref() {
            None => self.provider.label().into(),
            Some(Err(_)) => "Preferences unavailable".into(),
            Some(Ok(catalog)) => {
                if let Err(error) = &catalog.options
                    && self.needs_account_setup(error.message())
                {
                    return "Sign in".into();
                }
                if let Some(model) = catalog.selected() {
                    model.name.clone()
                } else if let Some(model) = &catalog.preferences.model {
                    let name = format_model_name(model.rsplit('/').next().unwrap_or(model));
                    format!("{name} (unavailable)")
                } else if catalog.options.is_err() {
                    "Models unavailable".into()
                } else {
                    "Choose a model".into()
                }
            }
        }
    }

    fn needs_account_setup(&self, error: &str) -> bool {
        self.provider.mode() == AssistantConnection::CodexAccount
            && (error.starts_with("Download the Codex runtime")
                || error.starts_with("Sign in with ChatGPT")
                || error.starts_with("Your ChatGPT session ended.")
                || error.starts_with("ChatGPT plan usage was not granted.")
                || error == refract_native::managed_codex::UNSUPPORTED_PLATFORM)
    }

    pub(super) fn send_issue(&self) -> Option<&'static str> {
        if self.loading.is_some() {
            return Some(CHECKING_MODELS);
        }
        match self.catalog.as_ref() {
            None => Some(CHECKING_MODELS),
            Some(Err(_)) => Some("Retry loading model preferences"),
            Some(Ok(catalog)) => catalog.send_issue(
                self.provider.kind(),
                self.save_error
                    .as_ref()
                    .is_some_and(|error| error.affects_selection),
            ),
        }
    }

    pub(super) fn selection(&self) -> Option<ModelSelection> {
        if self.send_issue().is_some() {
            return None;
        }
        self.catalog()?.selection(self.provider.kind())
    }
}

fn load_models(
    provider: &InstalledProvider,
    project: &Path,
    cancel: &AtomicBool,
) -> Result<Vec<Model>, DiscoveryError> {
    match provider
        .prepare(cancel)
        .map_err(|error| DiscoveryError::Connection(error.to_string()))?
    {
        PreparedProvider::Codex(codex) => {
            let command = codex
                .command(project)
                .map_err(|error| DiscoveryError::Connection(error.to_string()))?;
            discover_codex_models(command, codex.key().is_some(), cancel)
                .map_err(DiscoveryError::Catalog)
                .map(|models| {
                    let mut models = codex_models(models);
                    distinguish_names(&mut models);
                    models
                })
        }
        PreparedProvider::OpenCode(cli) => discover_opencode_models(cli, project, cancel)
            .map_err(DiscoveryError::Catalog)
            .map(open_code_models),
    }
}

fn codex_models(catalog: Vec<CodexCatalogModel>) -> Vec<Model> {
    let mut positions = HashMap::new();
    let mut models: Vec<Model> = Vec::new();
    for model in catalog {
        let legacy_id = (model.id != model.model).then_some(model.id);
        if let Some(&position) = positions.get(&model.model) {
            let existing: &mut Model = &mut models[position];
            existing.legacy_ids.extend(legacy_id);
            existing.is_default |= model.is_default;
            continue;
        }
        positions.insert(model.model.clone(), models.len());
        let efforts: Vec<_> = model
            .supported_reasoning_efforts
            .into_iter()
            .map(|effort| effort.effort)
            .filter(|effort| valid_codex_effort(effort))
            .collect();
        let default_effort = preferred_effort(&efforts, model.default_reasoning_effort.as_deref());
        models.push(Model {
            id: model.model,
            name: format_model_name(&model.display_name),
            source: None,
            disambiguator: None,
            legacy_ids: legacy_id.into_iter().collect(),
            description: model.description,
            default_effort,
            efforts,
            is_default: model.is_default,
        });
    }
    models
}

fn format_model_name(name: &str) -> String {
    static DASHES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"-+").unwrap());
    DASHES.replace_all(name, " ").trim().to_owned()
}

fn open_code_source(id: &str) -> String {
    let prefix = id.split('/').next().unwrap_or(id);
    match prefix.to_ascii_lowercase().as_str() {
        "opencode" => "OpenCode",
        "lmstudio" => "LM Studio",
        "anthropic" => "Anthropic",
        "openai" => "OpenAI",
        "openrouter" => "OpenRouter",
        "google" => "Google",
        _ => prefix,
    }
    .to_owned()
}

fn open_code_models(catalog: Vec<OpenCodeCatalogModel>) -> Vec<Model> {
    let mut models: Vec<_> = catalog
        .into_iter()
        .map(|model| Model {
            name: format_model_name(&model.name),
            source: Some(open_code_source(&model.id)),
            disambiguator: None,
            legacy_ids: Vec::new(),
            id: model.id,
            description: String::new(),
            default_effort: preferred_effort(&model.variants, None),
            efforts: model.variants,
            is_default: false,
        })
        .collect();
    let locale = sys_locale::get_locale()
        .and_then(|locale| locale.parse::<Locale>().ok())
        .unwrap_or(Locale::UNKNOWN);
    let collator = model_collator(&locale);
    let compare = |left: &str, right: &str| {
        collator
            .as_ref()
            .map_or_else(|| left.cmp(right), |collator| collator.compare(left, right))
    };
    // Electron ranks the initial selection by descending version, then by label.
    if let Some(model) = models.iter_mut().min_by(|left, right| {
        compare(model_version(right), model_version(left))
            .then_with(|| compare(&left.name, &right.name))
    }) {
        model.is_default = true;
    }
    models.sort_by(|left, right| {
        compare(&left.name, &right.name).then_with(|| compare(&left.id, &right.id))
    });
    distinguish_names(&mut models);
    models
}

fn model_collator(locale: &Locale) -> Option<CollatorBorrowed<'static>> {
    let mut options = CollatorOptions::default();
    options.strength = Some(Strength::Primary);
    let numeric = |mut preferences: CollatorPreferences| {
        preferences.numeric_ordering = Some(CollationNumericOrdering::True);
        Collator::try_new(preferences, options)
    };
    numeric(locale.into())
        .or_else(|_| numeric(CollatorPreferences::default()))
        .ok()
}

fn model_version(model: &Model) -> &str {
    static VERSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[0-9]+(?:\.[0-9]+)*").unwrap());
    VERSION
        .find(&model.name)
        .or_else(|| VERSION.find(&model.id))
        .map_or("0", |version| version.as_str())
}

fn distinguish_names(models: &mut [Model]) {
    let mut names = HashMap::new();
    let mut sources = HashMap::new();
    for model in models.iter() {
        *names.entry(model.name.clone()).or_insert(0usize) += 1;
        if let Some(source) = &model.source {
            *sources
                .entry((model.name.clone(), source.clone()))
                .or_insert(0usize) += 1;
        }
    }
    for model in models {
        if names[&model.name] > 1 {
            let source = model
                .source
                .as_ref()
                .filter(|source| sources[&(model.name.clone(), (*source).clone())] == 1);
            model.disambiguator = Some(source.unwrap_or(&model.id).clone());
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
                list = list.child(CHECKING_MODELS);
            }
            _ if self.loading.is_some() => {
                list = list.child("Loading models…");
            }
            Some(Ok(Catalog {
                options: Err(error),
                ..
            })) if self.needs_account_setup(error.message()) => {
                let owner = self.parent.clone();
                let busy = owner
                    .read_with(cx, |library, _| library.settings.account.busy())
                    .unwrap_or(false);
                list = list
                    .child("Codex requires sign-in")
                    .child(error.message().to_owned())
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
                    Button::new("retry-preferences")
                        .ghost()
                        .small()
                        .label("Retry preferences")
                        .on_click(cx.listener(|picker, _, _, cx| picker.load(cx))),
                );
            }
            Some(Ok(Catalog {
                options: Err(error),
                ..
            })) => {
                list = list.child(error.message().to_owned()).child(
                    Button::new("retry-models")
                        .ghost()
                        .small()
                        .label("Retry model discovery")
                        .on_click(cx.listener(|picker, _, _, cx| picker.load(cx))),
                );
            }
            Some(Ok(catalog)) => {
                let selected = catalog.selected();
                let terms: Vec<_> = query.split_whitespace().collect();
                let filtered: Vec<_> = catalog
                    .models()
                    .iter()
                    .filter(|model| {
                        let favorite = catalog.preferences.favorites.contains(&model.id);
                        if self.favorites && !favorite {
                            return false;
                        }
                        model.matches_search(self.provider.label(), &terms)
                    })
                    .collect();
                if filtered.is_empty() {
                    list = list.child(if catalog.models().is_empty() {
                        "No models available"
                    } else if query.is_empty() && self.favorites {
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
                    let tooltip = [
                        Some(model.name.as_str()),
                        model
                            .source
                            .as_deref()
                            .filter(|source| *source != self.provider.label()),
                        Some(model.id.as_str()).filter(|id| *id != model.name.as_str()),
                        Some(model.description.as_str())
                            .filter(|description| !description.is_empty()),
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("\n");
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
                                    .with_size(Size::Size(px(33.)))
                                    .flex_1()
                                    .min_w(px(0.))
                                    .min_h(px(33.))
                                    .justify_start()
                                    .child(
                                        div()
                                            .min_w(px(0.))
                                            .child(
                                                div()
                                                    .text_size(px(14.))
                                                    .line_height(px(21.))
                                                    .font_weight(FontWeight(580.))
                                                    .truncate()
                                                    .child(model.name.clone()),
                                            )
                                            .children(model.disambiguator.clone().map(|source| {
                                                div()
                                                    .text_size(px(11.))
                                                    .line_height(px(16.5))
                                                    .text_color(rgb(palette.muted))
                                                    .truncate()
                                                    .child(source)
                                            })),
                                    )
                                    .tooltip(tooltip)
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
                        let current = catalog.preferences.reasoning_effort.as_ref();
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
            .children(self.save_error.as_ref().map(|error| {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_color(rgb(palette.warning))
                    .child(error.message.clone())
                    .child(
                        Button::new("retry-model-save")
                            .ghost()
                            .small()
                            .label("Retry saving")
                            .disabled(self.loading.is_some())
                            .on_click(cx.listener(|picker, _, _, cx| {
                                if let Some(catalog) = picker.catalog() {
                                    picker.save(catalog.preferences.clone(), false, cx);
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
        let catalog = pane
            .models
            .get(&kind)
            .and_then(|picker| picker.read(cx).catalog());
        let (identity, options) = catalog
            .and_then(|catalog| {
                let selected = catalog.selected()?;
                let effort = catalog
                    .preferences
                    .reasoning_effort
                    .as_ref()
                    .filter(|effort| selected.efforts.contains(*effort))
                    .map(|effort| title_case(effort));
                let fast = (kind == Provider::Codex
                    && catalog.preferences.fast_mode.unwrap_or_default())
                .then(|| "Fast".to_owned());
                let options = [effort, fast]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · ");
                Some((
                    selected.disambiguator.clone(),
                    (!options.is_empty()).then_some(options),
                ))
            })
            .unwrap_or_default();
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
                            svg()
                                .path(match kind {
                                    Provider::Codex => "refract/openai.svg",
                                    Provider::OpenCode => "refract/opencode.svg",
                                })
                                .size(px(16.))
                                .text_color(rgb(palette.text)),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(name),
                        )
                        .children([identity, options].into_iter().flatten().map(|label| {
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(palette.muted))
                                .truncate()
                                .child(label)
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

    fn catalog(model: Option<&str>, effort: Option<&str>) -> Catalog {
        Catalog {
            options: Ok(vec![Model {
                id: "gpt-example".into(),
                name: "GPT Example".into(),
                source: None,
                disambiguator: None,
                legacy_ids: Vec::new(),
                description: String::new(),
                efforts: vec!["low".into(), "high".into()],
                default_effort: Some("low".into()),
                is_default: true,
            }]),
            preferences: AssistantPreferences {
                model: model.map(str::to_owned),
                reasoning_effort: effort.map(str::to_owned),
                ..AssistantPreferences::default()
            },
        }
    }

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

    #[test]
    fn open_code_catalog_orders_labels_and_prefers_the_highest_advertised_version() {
        let record = |id: &str, name: &str| OpenCodeCatalogModel {
            id: id.into(),
            name: name.into(),
            variants: vec!["high".into(), "low".into()],
        };
        let models = open_code_models(vec![
            record("openrouter/example-4.9", "Example-4.9"),
            record("lmstudio/example-4.10", "Example--4.10"),
            record("openai/example-4.10", "Example-4.10"),
            record("openai/other-4.10", "Example-4.10"),
        ]);
        assert_eq!(models[0].name, "Example 4.9");
        assert_eq!(models[1].name, "Example 4.10");
        let local = models
            .iter()
            .find(|model| model.id.starts_with("lmstudio/"))
            .unwrap();
        assert!(local.is_default);
        assert_eq!(local.disambiguator.as_deref(), Some("LM Studio"));
        assert!(local.matches_search("OpenCode", &["opencode", "example--4.10", "studio"]));
        assert!(!local.matches_search("OpenCode", &["example", "openai"]));
        let hosted = models
            .iter()
            .find(|model| model.id == "openai/example-4.10")
            .unwrap();
        assert_eq!(hosted.disambiguator.as_deref(), Some("openai/example-4.10"));
    }

    #[test]
    fn normalizes_an_unset_selection_and_uses_the_same_effort_for_turns() {
        let mut catalog = catalog(None, Some("medium"));
        let (normalized, selection_changed) = catalog.normalize_preferences().unwrap();
        assert!(selection_changed);
        catalog.preferences = normalized;
        assert!(catalog.normalize_preferences().is_none());
        assert_eq!(catalog.send_issue(Provider::Codex, false), None);
        let selection = catalog.selection(Provider::Codex).unwrap();
        assert_eq!(catalog.selected().unwrap().id, selection.model);
        assert_eq!(selection.effort.as_deref(), Some("low"));
        assert_eq!(selection.fast_mode, Some(false));

        catalog.preferences.fast_mode = Some(true);
        catalog.preferences.reasoning_effort = Some("high".into());
        assert!(catalog.normalize_preferences().is_none());
        let selection = catalog.selection(Provider::Codex).unwrap();
        assert_eq!(selection.effort.as_deref(), Some("high"));
        assert_eq!(selection.fast_mode, Some(true));
    }

    #[test]
    fn blocks_unavailable_models_efforts_and_unsaved_settings() {
        let mut catalog = catalog(Some("removed"), Some("medium"));
        assert!(catalog.normalize_preferences().is_none());
        assert_eq!(
            catalog.send_issue(Provider::Codex, false),
            Some("Choose an available Codex model")
        );
        catalog.preferences.model = Some("gpt-example".into());
        assert_eq!(
            catalog.send_issue(Provider::Codex, false),
            Some("Choose an available reasoning effort")
        );
        assert_eq!(
            catalog.send_issue(Provider::OpenCode, false),
            Some("Choose an available model variant")
        );

        catalog.preferences = catalog.normalize_preferences().unwrap().0;
        assert_eq!(
            catalog.send_issue(Provider::Codex, true),
            Some("Retry saving model settings")
        );
        assert_eq!(catalog.send_issue(Provider::Codex, false), None);

        catalog.options = Ok(Vec::new());
        assert_eq!(
            catalog.send_issue(Provider::OpenCode, false),
            Some("Choose an available OpenCode model")
        );
    }

    #[test]
    fn allows_saved_codex_selections_only_on_catalog_discovery_failure() {
        let mut catalog = catalog(Some("gpt-example"), Some("high"));
        catalog.options = Err(DiscoveryError::Catalog("Discovery failed".into()));
        assert_eq!(catalog.send_issue(Provider::Codex, false), None);
        let selection = catalog.selection(Provider::Codex).unwrap();
        assert_eq!(selection.model, "gpt-example");
        assert_eq!(selection.effort.as_deref(), Some("high"));
        assert_eq!(
            catalog.send_issue(Provider::OpenCode, false),
            Some("Retry loading OpenCode models")
        );
        catalog.options = Err(DiscoveryError::Connection("Account setup failed".into()));
        assert_eq!(
            catalog.send_issue(Provider::Codex, false),
            Some("Retry loading Codex models")
        );
        catalog.options = Err(DiscoveryError::Catalog("Discovery failed".into()));
        catalog.preferences.model = None;
        assert_eq!(
            catalog.send_issue(Provider::Codex, false),
            Some("Retry loading Codex models")
        );
    }

    #[test]
    fn uses_the_same_canonical_codex_ids_as_electron_preferences() {
        use refract_native::assistant_runtime::codex_catalog::CodexCatalogEffort;

        let record = |id: &str, canonical_id: &str| CodexCatalogModel {
            id: id.into(),
            model: canonical_id.into(),
            display_name: canonical_id.into(),
            description: String::new(),
            supported_reasoning_efforts: vec![CodexCatalogEffort {
                effort: "high".into(),
                description: String::new(),
            }],
            default_reasoning_effort: Some("high".into()),
            is_default: false,
        };
        let models = codex_models(vec![
            record("listing-1", "gpt-example"),
            record("listing-2", "gpt-example"),
            record("gpt-example", "other-model"),
        ]);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].legacy_ids, ["listing-1", "listing-2"]);
        let mut catalog = Catalog {
            options: Ok(models),
            preferences: serde_json::from_str(
                r#"{"model":"gpt-example","reasoningEffort":"high","favorites":["gpt-example"]}"#,
            )
            .unwrap(),
        };
        assert!(catalog.normalize_preferences().is_none());
        assert_eq!(catalog.send_issue(Provider::Codex, false), None);
        assert_eq!(
            catalog.selection(Provider::Codex).unwrap().model,
            "gpt-example"
        );

        catalog.preferences.model = Some("listing-2".into());
        catalog.preferences.favorites = ["retired", "listing-1", "gpt-example", "listing-2"]
            .map(str::to_owned)
            .to_vec();
        let (normalized, selection_changed) = catalog.normalize_preferences().unwrap();
        assert!(selection_changed);
        assert_eq!(normalized.model.as_deref(), Some("gpt-example"));
        assert_eq!(normalized.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(normalized.favorites, ["retired", "gpt-example"]);

        catalog.preferences.model = normalized.model;
        let (favorites_only, selection_changed) = catalog.normalize_preferences().unwrap();
        assert!(!selection_changed);
        assert_eq!(favorites_only.favorites, ["retired", "gpt-example"]);
    }
}
