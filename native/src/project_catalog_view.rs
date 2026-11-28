use super::{Library, Palette};
use gpui::{Context, Entity, SharedString, Subscription, Task, Window, div, prelude::*, px, rgb};
use gpui_component::{
    Icon, Sizable, Size,
    select::{Select, SelectEvent, SelectItem, SelectState},
};
use icu_collator::{Collator, CollatorBorrowed};
use icu_locale_core::Locale;
use refract_native::{
    Project,
    project_catalog::{LibraryPreferences, ProjectSort, latest_edits, visible_projects},
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::SystemTime,
};

#[derive(Clone, Copy)]
struct SortChoice(ProjectSort);

impl SelectItem for SortChoice {
    type Value = ProjectSort;

    fn title(&self) -> SharedString {
        match self.0 {
            ProjectSort::Newest => "Newest first",
            ProjectSort::Oldest => "Oldest first",
            ProjectSort::Name => "Name",
            ProjectSort::Edited => "Recently edited",
        }
        .into()
    }

    fn value(&self) -> &ProjectSort {
        &self.0
    }
}

pub(super) struct ProjectCatalogView {
    preferences: LibraryPreferences,
    path: PathBuf,
    sort: Entity<SelectState<Vec<SortChoice>>>,
    edits: HashMap<String, SystemTime>,
    edit_scan: Option<EditScan>,
    collator: Option<CollatorBorrowed<'static>>,
    _sort_changed: Subscription,
}

struct EditScan {
    cancel: Arc<AtomicBool>,
    _task: Task<()>,
}

impl Drop for EditScan {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl ProjectCatalogView {
    pub(super) fn new(projects_dir: &Path, window: &mut Window, cx: &mut Context<Library>) -> Self {
        let path = projects_dir.with_file_name(".refract-native-library.json");
        let preferences = LibraryPreferences::read(&path);
        let choices = [
            ProjectSort::Newest,
            ProjectSort::Oldest,
            ProjectSort::Name,
            ProjectSort::Edited,
        ]
        .into_iter()
        .map(SortChoice)
        .collect::<Vec<_>>();
        let sort = cx.new(|cx| {
            let mut state = SelectState::new(choices, None, window, cx);
            state.set_selected_value(&preferences.sort, window, cx);
            state
        });
        let sort_changed = cx.subscribe(&sort, |library, _, event, cx| {
            if let SelectEvent::Confirm(Some(choice)) = event
                && library.catalog.preferences.sort != *choice
            {
                library.catalog.preferences.sort = *choice;
                library.catalog.save_preferences();
                library.refresh_project_edits(cx);
                cx.notify();
            }
        });
        let locale = sys_locale::get_locale()
            .and_then(|locale| locale.parse::<Locale>().ok())
            .unwrap_or(Locale::UNKNOWN);
        let collator = Collator::try_new((&locale).into(), Default::default())
            .or_else(|_| Collator::try_new(Default::default(), Default::default()))
            .ok();
        Self {
            preferences,
            path,
            sort,
            edits: HashMap::new(),
            edit_scan: None,
            collator,
            _sort_changed: sort_changed,
        }
    }

    pub(super) fn is_grid(&self) -> bool {
        self.preferences.grid
    }

    pub(super) fn reload_preferences(&mut self, window: &mut Window, cx: &mut Context<Library>) {
        self.preferences = LibraryPreferences::read(&self.path);
        self.sort.update(cx, |state, cx| {
            state.set_selected_value(&self.preferences.sort, window, cx)
        });
    }

    pub(super) fn visible_projects<'a>(
        &self,
        projects: &'a [Project],
        query: &str,
    ) -> Vec<&'a Project> {
        visible_projects(
            projects,
            query,
            self.preferences.sort,
            &self.edits,
            |a, b| {
                self.collator
                    .as_ref()
                    .map_or_else(|| a.cmp(b), |collator| collator.compare(a, b))
            },
        )
    }

    pub(super) fn cancel_edit_scan(&mut self) {
        self.edit_scan = None;
    }

    fn save_preferences(&self) {
        if let Err(error) = self.preferences.save(&self.path) {
            eprintln!("Couldn’t save the project library preferences. {error}");
        }
    }

    pub(super) fn sort_control(&self, palette: Palette) -> impl IntoElement {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .text_size(px(12.))
            .child(div().text_color(rgb(palette.muted)).child("Sort"))
            .child(
                div()
                    .w(px(if self.preferences.sort == ProjectSort::Edited {
                        132.
                    } else {
                        108.
                    }))
                    .child(
                        Select::new(&self.sort)
                            .with_size(Size::Small)
                            .icon(Icon::empty().path("refract/chevron-down.svg"))
                            .h(px(36.))
                            .text_size(px(12.))
                            .text_color(rgb(palette.text))
                            .bg(rgb(palette.surface))
                            .border_color(rgb(palette.input_border)),
                    ),
            )
    }
}

impl Library {
    pub(super) fn set_library_grid(&mut self, grid: bool, cx: &mut Context<Self>) {
        if self.catalog.preferences.grid != grid {
            self.catalog.preferences.grid = grid;
            self.catalog.save_preferences();
            cx.notify();
        }
    }

    pub(super) fn refresh_project_edits(&mut self, cx: &mut Context<Self>) {
        self.catalog.cancel_edit_scan();
        if self.catalog.preferences.sort != ProjectSort::Edited || self.selection.is_some() {
            return;
        }
        let projects = match &self.projects {
            Some(Ok(projects)) if !projects.is_empty() => projects.clone(),
            _ => {
                self.catalog.edits.clear();
                return;
            }
        };
        let root = self.projects_dir.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let scan_cancel = cancel.clone();
        let scan = cx
            .background_executor()
            .spawn(async move { latest_edits(&root, &projects, &scan_cancel) });
        let guard = cancel.clone();
        let task = cx.spawn(async move |library, cx| {
            let dates = scan.await;
            if guard.load(Ordering::Relaxed) {
                return;
            }
            let _ = library.update(cx, |library, cx| {
                library.catalog.edits = dates;
                library.catalog.edit_scan = None;
                cx.notify();
            });
        });
        self.catalog.edit_scan = Some(EditScan {
            cancel,
            _task: task,
        });
    }
}
