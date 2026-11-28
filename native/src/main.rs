#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

use gpui::{
    AnyElement, App, Application, Bounds, Context, Entity, FocusHandle, FontWeight, Image,
    ImageFormat, KeyBinding, PromptLevel, SharedString, Subscription, Task, Window,
    WindowAppearance, WindowBounds, WindowOptions, actions, div, img, prelude::*, px, rgb, size,
    svg,
};
use gpui_component::{
    Root,
    input::{Input, InputEvent, InputState},
};
use refract_native::comments::{Comment, read_comments};
use refract_native::project_catalog::matches_query;
use refract_native::versions::{
    Version, list_versions, recover_project_restore, restore_project_version, restore_version_file,
    retry_retained_restore_stages, snapshot_project,
};
use refract_native::workspace_layout::{
    Divider, Pane, PaneVisibility, WORKSPACE_HEADER_HEIGHT, WorkspaceLayout,
};
use refract_native::{Project, ProjectEntry, list_projects, project_tree, save_source};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    env, io,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

mod account_settings_view;
mod asset_image;
mod asset_view;
mod build_log_view;
mod commands_view;
mod comments_view;
mod compile_view;
mod conversation_view;
mod deleted_projects_view;
mod editor_find_view;
mod file_creation_view;
mod file_move_view;
mod file_search_view;
mod file_transfer_view;
mod file_trash_view;
mod instance;
mod outline_view;
mod pdf_view;
mod project_catalog_view;
mod project_import_view;
mod project_info_view;
mod project_management_view;
mod project_thumbnails;
mod settings_view;
mod source_theme;
mod synctex_view;
mod tracked_label;
mod version_view;
#[cfg(windows)]
mod window_chrome;
mod workspace_layout_view;
mod workspace_tools;
use asset_view::{AssetPreview, LoadedFile, load_file};
use build_log_view::BuildLogView;
use commands_view::Commands;
use comments_view::CommentDrafts;
use compile_view::{RunningBuild, WorkspaceBuild};
use conversation_view::ConversationPane;
use deleted_projects_view::DeletedProjectsDialog;
use editor_find_view::EditorFind;
use file_creation_view::EntryDraft;
use file_move_view::{EntryMenu, EntryMoveDraft};
use file_search_view::{FileSearch, RefractAssets};
use file_trash_view::DeletedFilesDialog;
use outline_view::OutlinePane;
use project_catalog_view::ProjectCatalogView;
use project_import_view::ImportDraft;
use project_info_view::ProjectInfoState;
use project_management_view::{ProjectFocus, RenameDraft};
use project_thumbnails::ProjectThumbnails;
use settings_view::SettingsView;
use synctex_view::RunningSync;
use version_view::Comparison;
use workspace_layout_view::DividerDrag;
use workspace_tools::WorkspaceTool;

actions!(editor, [SaveSource]);

struct Library {
    projects_dir: PathBuf,
    projects: Option<io::Result<Vec<Project>>>,
    project_query: Entity<InputState>,
    _project_query_events: Subscription,
    project_open_sequence: u64,
    project_library_task: Option<Task<()>>,
    import_draft: Option<ImportDraft>,
    import_focus: FocusHandle,
    project_action_prompt: Option<Task<()>>,
    project_menu: Option<String>,
    project_action_source: Option<String>,
    deleted_projects_focus: FocusHandle,
    deleted_projects_dialog: Option<Rc<RefCell<DeletedProjectsDialog>>>,
    project_rename_task: Option<Task<()>>,
    project_action_error: Option<String>,
    project_name_focus: FocusHandle,
    keyboard_focus: bool,
    rename_draft: Option<RenameDraft>,
    selection: Option<Selection>,
    tree_task: Option<Task<()>>,
    entry_write_task: Option<Task<()>>,
    entry_draft: Option<EntryDraft>,
    entry_menu: Option<EntryMenu>,
    entry_delete_prompt: Option<Task<()>>,
    deleted_files_focus: FocusHandle,
    deleted_files_dialog: Option<Rc<RefCell<DeletedFilesDialog>>>,
    move_draft: Option<EntryMoveDraft>,
    preview_task: Option<Task<()>>,
    editor_find: EditorFind,
    commands: Commands,
    outline: OutlinePane,
    comments_task: Option<Task<()>>,
    comment_write_task: Option<Task<()>>,
    comment_drafts: HashMap<String, CommentDrafts>,
    save_task: Option<Task<()>>,
    save_queue: VecDeque<String>,
    build_task: Option<RunningBuild>,
    pdf_task: Option<Task<()>>,
    sync_task: Option<RunningSync>,
    #[cfg(all(test, feature = "ui-tests"))]
    test_compiler: Option<compile_view::Compiler>,
    snapshot_task: Option<Task<()>>,
    snapshot_timer: Option<Task<()>>,
    snapshot_needed: bool,
    comparison_task: Option<Task<()>>,
    restore_task: Option<Task<()>>,
    navigation_after_save: Option<Navigation>,
    discard_prompt: Option<Task<()>>,
    focus_handle: FocusHandle,
    project_focus: HashMap<String, ProjectFocus>,
    home_focus: FocusHandle,
    logo: Arc<Image>,
    catalog: ProjectCatalogView,
    settings: SettingsView,
    thumbnails: ProjectThumbnails,
    workspace_layout: WorkspaceLayout,
    divider_drag: Option<DividerDrag>,
    preview_pane_focus: FocusHandle,
    sidebar_collapse_focus: FocusHandle,
    last_tool: WorkspaceTool,
}

struct Selection {
    project: Project,
    tree: Option<io::Result<Vec<ProjectEntry>>>,
    file_action_error: Option<String>,
    expanded_folders: HashSet<String>,
    file_search: FileSearch,
    conversation: ConversationPane,
    file: Option<String>,
    asset_preview: Option<AssetPreview>,
    tabs: Vec<FileTab>,
    tool: Option<WorkspaceTool>,
    tools_expanded: Option<bool>,
    project_info: Option<ProjectInfoState>,
    build_logs: Option<BuildLogView>,
    focus_source_on_load: bool,
    versions: Option<io::Result<Vec<Version>>>,
    restore_error: Option<String>,
    retained_restore_stages: bool,
    comparison: Option<Comparison>,
    comments: Option<io::Result<Vec<Comment>>>,
    comment_error: Option<String>,
    pending_comment: Option<Comment>,
    pending_source_jump: Option<refract_native::tex::synctex::SourcePosition>,
    comment_status: Option<&'static str>,
    build: WorkspaceBuild,
}

struct FileTab {
    path: String,
    open_focus: FocusHandle,
    close_focus: FocusHandle,
    editor: Option<io::Result<EditorDocument>>,
}

struct EditorDocument {
    state: Entity<InputState>,
    saved: String,
    save_error: Option<String>,
    _autosave: Subscription,
    _editor_selection: Subscription,
}

enum Navigation {
    CloseFile(String),
    Projects,
    CloseWindow,
    RestartForUpdate,
}

impl Navigation {
    fn exits(&self) -> bool {
        matches!(self, Self::CloseWindow | Self::RestartForUpdate)
    }
}

#[derive(Clone, Copy)]
enum RestoreKind {
    File,
    Project,
}

impl Library {
    fn new(projects_dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let path = projects_dir.clone();
        let load = cx.background_executor().spawn(async move {
            if let Err(error) = refract_native::migrate_legacy_projects(&path) {
                eprintln!(
                    "Couldn’t import projects from the Refract prototype ({:?}).",
                    error.kind()
                );
            }
            if let Err(error) = refract_native::legacy_profile::migrate_electron_profile(&path) {
                eprintln!(
                    "Couldn’t import saved Refract preferences ({:?}).",
                    error.kind()
                );
            }
            let mut projects = list_projects(&path)?;
            for project in &mut projects {
                if let Some(recovered) = recover_project_restore(&path, project)? {
                    *project = recovered.project;
                }
            }
            Ok(projects)
        });
        cx.spawn_in(window, async move |library, cx| {
            let projects = load.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.catalog.reload_preferences(window, cx);
                library.outline.reload_height();
                library.workspace_layout.panes = PaneVisibility::read(
                    &library
                        .projects_dir
                        .with_file_name(".refract-native-workspace.json"),
                );
                library.set_projects(projects, cx);
                library.resume_navigation(window, cx);
                cx.notify();
            });
        })
        .detach();

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        cx.observe_window_bounds(window, |_, _, cx| cx.notify())
            .detach();
        let outline = OutlinePane::new(&projects_dir);
        let project_query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search")
                .clean_on_escape()
        });
        let project_query_events = cx.subscribe(&project_query, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let catalog = ProjectCatalogView::new(&projects_dir, window, cx);
        let settings = SettingsView::new(&projects_dir, window, cx);
        let workspace_layout = WorkspaceLayout::new(PaneVisibility::read(
            &projects_dir.with_file_name(".refract-native-workspace.json"),
        ));
        Self {
            projects_dir,
            projects: None,
            project_query,
            _project_query_events: project_query_events,
            project_open_sequence: 0,
            project_library_task: None,
            import_draft: None,
            import_focus: cx.focus_handle().tab_stop(true).tab_index(1),
            project_action_prompt: None,
            project_menu: None,
            project_action_source: None,
            deleted_projects_focus: cx.focus_handle().tab_stop(true).tab_index(1),
            deleted_projects_dialog: None,
            project_rename_task: None,
            project_action_error: None,
            project_name_focus: cx.focus_handle().tab_stop(true).tab_index(1),
            keyboard_focus: false,
            rename_draft: None,
            selection: None,
            tree_task: None,
            entry_write_task: None,
            entry_draft: None,
            entry_menu: None,
            entry_delete_prompt: None,
            deleted_files_focus: cx.focus_handle().tab_stop(true).tab_index(1),
            deleted_files_dialog: None,
            move_draft: None,
            preview_task: None,
            editor_find: EditorFind::new(window, cx),
            commands: Commands::default(),
            outline,
            comments_task: None,
            comment_write_task: None,
            comment_drafts: HashMap::new(),
            save_task: None,
            save_queue: VecDeque::new(),
            build_task: None,
            pdf_task: None,
            sync_task: None,
            #[cfg(all(test, feature = "ui-tests"))]
            test_compiler: None,
            snapshot_task: None,
            snapshot_timer: None,
            snapshot_needed: false,
            comparison_task: None,
            restore_task: None,
            navigation_after_save: None,
            discard_prompt: None,
            focus_handle,
            project_focus: HashMap::new(),
            home_focus: cx.focus_handle().tab_stop(true).tab_index(0),
            logo: Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("../../resources/logo.png").to_vec(),
            )),
            catalog,
            settings,
            thumbnails: ProjectThumbnails::default(),
            workspace_layout,
            divider_drag: None,
            preview_pane_focus: cx.focus_handle().tab_stop(true).tab_index(3),
            sidebar_collapse_focus: cx.focus_handle().tab_stop(true).tab_index(1),
            last_tool: WorkspaceTool::Logs,
        }
    }

    fn open_project(&mut self, project: Project, window: &mut Window, cx: &mut Context<Self>) {
        if self.project_action_prompt.is_some()
            || self.deleted_projects_dialog.is_some()
            || self.project_action_source.as_deref() == Some(project.id.as_str())
        {
            return;
        }
        self.catalog.cancel_edit_scan();
        self.thumbnails.cancel();
        self.project_menu = None;
        self.entry_menu = None;
        self.project_open_sequence = self.project_open_sequence.wrapping_add(1);
        let file = project.root_file().replace('\\', "/");
        self.selection = Some(Selection {
            project: project.clone(),
            tree: None,
            file_action_error: None,
            expanded_folders: HashSet::new(),
            file_search: FileSearch::new(window, cx),
            conversation: ConversationPane::default(),
            file: None,
            asset_preview: None,
            tabs: Vec::new(),
            tool: None,
            tools_expanded: None,
            project_info: None,
            build_logs: None,
            focus_source_on_load: false,
            versions: None,
            restore_error: None,
            retained_restore_stages: false,
            comparison: None,
            comments: None,
            comment_error: None,
            pending_comment: None,
            pending_source_jump: None,
            comment_status: None,
            build: WorkspaceBuild::new(),
        });
        let path = self.projects_dir.clone();
        let load = cx.background_executor().spawn(async move {
            let retained =
                retry_retained_restore_stages(&path, &project).is_ok_and(|count| count > 0);
            (project_tree(&path, &project), retained)
        });
        self.tree_task = Some(cx.spawn(async move |library, cx| {
            let (tree, retained) = load.await;
            let _ = library.update(cx, |library, cx| {
                if let Some(selection) = &mut library.selection {
                    if let Ok(entries) = &tree {
                        collect_expanded_folders(entries, &mut selection.expanded_folders);
                    }
                    selection.tree = Some(tree);
                    selection.retained_restore_stages = retained;
                    cx.notify();
                }
            });
        }));
        self.load_project_conversation(Default::default(), window, cx);
        self.open_file(file, window, cx);
        self.snapshot_needed = true;
        self.start_snapshot(window, cx);
        window.focus(&self.project_name_focus);
    }

    fn open_file(&mut self, file: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        if !selection.tabs.iter().any(|tab| tab.path == file) {
            selection.tabs.push(FileTab {
                path: file.clone(),
                open_focus: cx.focus_handle().tab_stop(true).tab_index(2),
                close_focus: cx.focus_handle().tab_stop(true).tab_index(2),
                editor: None,
            });
        }
        selection.file = Some(file.clone());
        if let Some(asset) = selection.asset_preview.take() {
            asset.release(window);
        }
        selection.pending_comment = None;
        selection.pending_source_jump = None;
        selection.file_search.pending_hit = None;
        selection.focus_source_on_load = false;
        selection.comment_status = None;
        self.preview_task = None;
        self.commands.close();
        if selection
            .tabs
            .iter()
            .any(|tab| tab.path == file && tab.editor.as_ref().is_some_and(Result::is_ok))
        {
            if let Some(editor) = self.active_editor() {
                self.refresh_outline(&editor, window, cx);
            }
            self.refresh_find_for_file(window, cx);
            cx.notify();
            return;
        }
        self.outline.clear(window);
        let project = selection.project.clone();
        let path = self.projects_dir.clone();
        let opened_file = file.clone();
        let load = cx
            .background_executor()
            .spawn(async move { load_file(&path, &project, &opened_file) });
        self.preview_task = Some(cx.spawn_in(window, async move |library, cx| {
            let source = load.await;
            let _ = library.update_in(cx, |library, window, cx| {
                let Some(selection) = &mut library.selection else {
                    return;
                };
                let Some(tab) = selection.tabs.iter_mut().find(|tab| tab.path == file) else {
                    return;
                };
                let source = match source {
                    Ok(LoadedFile::Text(source)) => Ok(source),
                    Err(error) => Err(error),
                    Ok(LoadedFile::Asset(asset)) => {
                        selection.asset_preview = Some(asset);
                        selection.pending_comment = None;
                        selection.pending_source_jump = None;
                        selection.file_search.pending_hit = None;
                        selection.focus_source_on_load = false;
                        tab.editor = None;
                        library.editor_find.visible = false;
                        cx.notify();
                        return;
                    }
                };
                tab.editor = Some(source.map(|source| {
                    let state = cx.new(|cx| {
                        InputState::new(window, cx)
                            .code_editor(refract_native::latex_syntax::NAME)
                            .searchable(false)
                            .soft_wrap(false)
                            .default_value(source.clone())
                    });
                    let autosave = Self::watch_source(&state, &file, window, cx);
                    let editor_selection =
                        cx.observe_in(&state, window, |library, editor, window, cx| {
                            library.sync_find_selection(&editor, false, window, cx);
                            library.sync_commands_selection(&editor, window, cx);
                        });
                    EditorDocument {
                        state,
                        saved: source,
                        save_error: None,
                        _autosave: autosave,
                        _editor_selection: editor_selection,
                    }
                }));
                if tab.editor.as_ref().is_some_and(Result::is_err) {
                    library.editor_find.visible = false;
                }
                let has_source_destination = selection.pending_comment.is_some()
                    || selection.pending_source_jump.is_some()
                    || selection.file_search.pending_hit.is_some();
                if let Some(comment) = selection.pending_comment.take()
                    && let Some(Ok(editor)) = tab.editor.as_ref()
                {
                    selection.comment_status =
                        Self::jump_to_comment(&editor.state, &comment, window, cx);
                }
                if let Some(target) = selection.pending_source_jump.take()
                    && target.file == file
                    && let Some(Ok(editor)) = tab.editor.as_ref()
                {
                    Self::select_source_line(&editor.state, target.line, window, cx);
                }
                if let Some(hit) = selection.file_search.pending_hit.take()
                    && hit.file == file
                    && let Some(Ok(editor)) = tab.editor.as_ref()
                {
                    Self::select_search_hit(&editor.state, &hit, window, cx);
                }
                if std::mem::take(&mut selection.focus_source_on_load)
                    && selection.file.as_deref() == Some(file.as_str())
                    && let Some(Ok(editor)) = tab.editor.as_ref()
                {
                    editor
                        .state
                        .update(cx, |editor, cx| editor.focus(window, cx));
                }
                cx.notify();
                if let Some(editor) = library.active_editor() {
                    library.refresh_outline(&editor, window, cx);
                }
                if !has_source_destination {
                    library.refresh_find_for_file(window, cx);
                }
            });
        }));
        cx.notify();
    }

    fn watch_source(
        state: &Entity<InputState>,
        file: &str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        let file = file.to_owned();
        cx.subscribe_in(
            state,
            window,
            move |library, editor, event: &InputEvent, window, cx| {
                library.update_commands_for_input(&editor, event, window, cx);
                if matches!(event, InputEvent::Change) {
                    library.queue_save(&file, window, cx);
                    library.sync_find_selection(&editor, true, window, cx);
                    library.refresh_outline(&editor, window, cx);
                }
            },
        )
    }

    fn close_file(&mut self, file: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        let Some(index) = selection.tabs.iter().position(|tab| tab.path == file) else {
            return;
        };
        let had_focus = selection.tabs[index].close_focus.is_focused(window);
        selection.tabs.remove(index);
        if selection.file.as_deref() == Some(file) {
            if let Some(last) = selection.tabs.last() {
                let path = last.path.clone();
                self.open_file(path, window, cx);
            } else {
                self.preview_task = None;
                self.editor_find.visible = false;
                self.commands.close();
                self.outline.clear(window);
                if let Some(asset) = selection.asset_preview.take() {
                    asset.release(window);
                }
                selection.file = None;
                selection.pending_comment = None;
                selection.comment_status = None;
            }
        }
        if had_focus {
            let focus = self
                .selection
                .as_ref()
                .and_then(|selection| {
                    let file = selection.file.as_deref()?;
                    selection
                        .tabs
                        .iter()
                        .find(|tab| tab.path == file)
                        .map(|tab| &tab.open_focus)
                })
                .unwrap_or(if self.workspace_layout.sidebar_collapsed {
                    &self.sidebar_collapse_focus
                } else {
                    &self.home_focus
                });
            window.focus(focus);
        }
        cx.notify();
    }

    fn return_to_projects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tree_task = None;
        self.divider_drag = None;
        self.entry_menu = None;
        self.preview_task = None;
        self.editor_find.visible = false;
        self.commands.close();
        self.outline.clear(window);
        self.comments_task = None;
        self.comparison_task = None;
        if let Some(mut previous) = self.selection.take() {
            previous.build.preview.release(window);
            if let Some(asset) = previous.asset_preview {
                asset.release(window);
            }
            let query = self.project_query.read(cx).value().to_lowercase();
            if let Some(focus) = self
                .project_focus
                .get(&previous.project.id)
                .filter(|_| matches_query(&previous.project, &query))
            {
                window.focus(&focus.open);
            } else {
                self.project_query
                    .update(cx, |input, cx| input.focus(window, cx));
            }
        }
        self.refresh_project_edits(cx);
        self.refresh_project_thumbnails(cx);
        cx.notify();
    }

    fn toggle_folder(&mut self, path: &str, cx: &mut Context<Self>) {
        if let Some(selection) = &mut self.selection {
            if !selection.expanded_folders.remove(path) {
                selection.expanded_folders.insert(path.to_owned());
            }
            cx.notify();
        }
    }

    fn file_to_restore<'a>(&'a self, version: &Version, cx: &App) -> Option<&'a str> {
        let selection = self.selection.as_ref()?;
        let file = selection.file.as_deref()?;
        (self.project_writes_idle()
            && !self.has_unsaved_changes(&Navigation::CloseFile(file.to_owned()), cx)
            && version.files.iter().any(|name| name == file))
        .then_some(file)
    }

    fn project_writes_idle(&self) -> bool {
        self.project_writes_ready()
            && self.rename_draft.is_none()
            && self.entry_draft.is_none()
            && self.move_draft.is_none()
            && self.entry_delete_prompt.is_none()
            && self.deleted_files_dialog.is_none()
    }

    fn project_writes_ready(&self) -> bool {
        self.restore_task.is_none()
            && self.build_task.is_none()
            && self.sync_task.is_none()
            && self.comment_write_task.is_none()
            && self.project_rename_task.is_none()
            && self.entry_write_task.is_none()
            && self.save_task.is_none()
            && self.save_queue.is_empty()
            && self.snapshot_task.is_none()
            && self.discard_prompt.is_none()
    }

    fn can_restore_project(&self, cx: &App) -> bool {
        self.project_writes_idle() && !self.has_unsaved_changes(&Navigation::Projects, cx)
    }

    fn request_version_restore(
        &mut self,
        version_id: &str,
        kind: RestoreKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(version) = self
            .selection
            .as_ref()
            .and_then(|selection| selection.versions.as_ref()?.as_ref().ok())
            .and_then(|versions| versions.iter().find(|version| version.id == version_id))
        else {
            return;
        };
        let file = match kind {
            RestoreKind::File => {
                let Some(file) = self.file_to_restore(version, cx) else {
                    return;
                };
                Some(file.to_owned())
            }
            RestoreKind::Project if self.can_restore_project(cx) => None,
            RestoreKind::Project => return,
        };
        let Some(selection) = &mut self.selection else {
            return;
        };
        selection.restore_error = None;
        let project = selection.project.clone();
        let version_id = version_id.to_owned();
        let (message, detail, action) = match &file {
            Some(file) => (
                format!("Restore {file} from this version?"),
                None,
                "Restore file",
            ),
            None => (
                "Restore all project files from this version?".into(),
                Some("The current project will be saved as a version first."),
                "Restore project",
            ),
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &message,
            detail,
            &["Cancel", action],
            cx,
        );
        let executor = cx.background_executor().clone();
        let path = self.projects_dir.clone();
        self.restore_task = Some(cx.spawn_in(window, async move |library, cx| {
            let restored = if matches!(answer.await, Ok(1)) {
                let restored_file = file.clone();
                Some(
                    executor
                        .spawn(async move {
                            let result = match restored_file {
                                Some(file) => restore_version_file(&path, &project, &version_id, &file).map(|()| None),
                                None => restore_project_version(&path, &project, &version_id).map(Some),
                            };
                            (
                                result,
                                project_tree(&path, &project),
                                list_versions(&path, &project),
                            )
                        })
                        .await,
                )
            } else {
                None
            };
            let _ = library.update_in(cx, |library, window, cx| {
                library.restore_task = None;
                if let Some((result, tree, versions)) = restored
                    && let Some(selection) = &mut library.selection
                {
                    library.tree_task = None;
                    library.comparison_task = None;
                    selection.comparison = None;
                    selection.tree = Some(tree);
                    selection.versions = Some(versions);
                    let mut reload_source = None;
                    match result {
                        Ok(Some(restored)) => {
                            selection.build.invalidate();
                            selection.restore_error = restored.comment_warning.map(|error| format!("Project restored. Comment anchors could not be updated. {error}"));
                            selection.retained_restore_stages |= restored.cleanup_pending != 0;
                            selection.project = restored.project;
                            selection.file = None;
                            selection.tabs.clear();
                            selection.pending_comment = None;
                            selection.comment_status = None;
                            library.preview_task = None;
                            if let Some(Ok(entries)) = &selection.tree {
                                let root = selection.project.root_file().replace('\\', "/");
                                reload_source = preferred_source(entries, &root).map(str::to_owned);
                            }
                            if let Some(path) = &reload_source {
                                selection.expanded_folders.extend(path.match_indices('/').map(|(index, _)| path[..index].to_owned()));
                            }
                        }
                        Ok(None) => selection.build.invalidate(),
                        Err(error) => selection.restore_error = Some(format!("{action} failed. {error}")),
                    }
                    if let Some(file) = file {
                        if let Some(tab) = selection.tabs.iter_mut().find(|tab| tab.path == file) {
                            tab.editor = None;
                        }
                        if selection.file.as_deref() == Some(file.as_str()) {
                            reload_source = Some(file);
                        }
                    }
                    let reload_comments = selection.tool == Some(WorkspaceTool::Comments);
                    if let Some(file) = reload_source {
                        library.open_file(file, window, cx);
                    }
                    if reload_comments {
                        library.load_comments(cx);
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

    fn load_comments(&mut self, cx: &mut Context<Self>) {
        let Some(selection) = &mut self.selection else {
            return;
        };
        let project = selection.project.clone();
        let path = self.projects_dir.clone();
        let load = cx
            .background_executor()
            .spawn(async move { read_comments(&path, &project) });
        self.comments_task = Some(cx.spawn(async move |library, cx| {
            let comments = load.await;
            let _ = library.update(cx, |library, cx| {
                if let Some(selection) = &mut library.selection {
                    selection.comments = Some(comments);
                    cx.notify();
                }
            });
        }));
    }

    fn save_active_file(&mut self, window: &Window, cx: &mut Context<Self>) {
        if let Some(file) = self
            .selection
            .as_ref()
            .and_then(|selection| selection.file.clone())
        {
            self.queue_save(&file, window, cx);
        }
    }

    fn queue_save(&mut self, file: &str, window: &Window, cx: &mut Context<Self>) {
        if let Some(selection) = &mut self.selection
            && selection.tabs.iter().any(|tab| {
                tab.path == file
                    && tab
                        .editor
                        .as_ref()
                        .and_then(|editor| editor.as_ref().ok())
                        .is_some_and(|editor| {
                            editor.saved != editor.state.read(cx).value().as_ref()
                        })
            })
        {
            selection.build.invalidate();
        }
        // Queue the path so later edits can share one write of the latest document.
        if !self.save_queue.iter().any(|queued| queued == file) {
            self.save_queue.push_back(file.to_owned());
        }
        self.start_next_save(window, cx);
    }

    fn start_next_save(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.save_task.is_some()
            || self.build_task.is_some()
            || self.snapshot_task.is_some()
            || self.restore_task.is_some()
            || self.comment_write_task.is_some()
            || self.entry_write_task.is_some()
        {
            return;
        }
        let Some(selection) = &mut self.selection else {
            self.save_queue.clear();
            return;
        };
        let (file, expected, edited) = loop {
            let Some(file) = self.save_queue.pop_front() else {
                return;
            };
            let Some(document) = selection
                .tabs
                .iter_mut()
                .find(|tab| tab.path == file)
                .and_then(|tab| tab.editor.as_mut()?.as_mut().ok())
            else {
                continue;
            };
            let edited = document.state.read(cx).value();
            if document.saved == edited.as_ref() {
                continue;
            }
            document.save_error = None;
            break (file, document.saved.clone(), edited.to_string());
        };
        let project = selection.project.clone();
        let path = self.projects_dir.clone();
        let saved_file = file.clone();
        let save = cx.background_executor().spawn(async move {
            save_source(&path, &project, &saved_file, &expected, &edited).map(|()| edited)
        });
        self.save_task = Some(cx.spawn_in(window, async move |library, cx| {
            let result = save.await;
            let reload_comments = result.is_ok();
            let _ = library.update_in(cx, |library, window, cx| {
                library.save_task = None;
                if result.is_err() {
                    library.save_queue.retain(|queued| queued != &file);
                }
                if let Some(selection) = &mut library.selection {
                    if let Some(document) = selection
                        .tabs
                        .iter_mut()
                        .find(|tab| tab.path == file)
                        .and_then(|tab| tab.editor.as_mut()?.as_mut().ok())
                    {
                        match result {
                            Ok(saved) => document.saved = saved,
                            Err(error) => document.save_error = Some(error.to_string()),
                        }
                    }
                    if reload_comments
                        && selection.tool == Some(WorkspaceTool::Comments)
                        && library.save_queue.is_empty()
                    {
                        library.load_comments(cx);
                    }
                }
                if reload_comments {
                    library.schedule_snapshot(window, cx);
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

    fn schedule_snapshot(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.snapshot_needed = true;
        let delay = cx.background_executor().timer(Duration::from_secs(3));
        self.snapshot_timer = Some(cx.spawn_in(window, async move |library, cx| {
            delay.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.snapshot_timer = None;
                library.start_snapshot(window, cx);
            });
        }));
    }

    fn start_snapshot(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.snapshot_needed
            || self.build_task.is_some()
            || self.save_task.is_some()
            || self.snapshot_task.is_some()
            || self.restore_task.is_some()
            || self.comment_write_task.is_some()
            || self.entry_write_task.is_some()
        {
            return;
        }
        let Some(selection) = &self.selection else {
            return;
        };
        let project = selection.project.clone();
        let path = self.projects_dir.clone();
        self.snapshot_needed = false;
        self.snapshot_timer = None;
        let snapshot = cx.background_executor().spawn(async move {
            snapshot_project(&path, &project)?;
            list_versions(&path, &project)
        });
        self.snapshot_task = Some(cx.spawn_in(window, async move |library, cx| {
            let result = snapshot.await;
            let _ = library.update_in(cx, |library, window, cx| {
                library.snapshot_task = None;
                if let Some(selection) = &mut library.selection {
                    selection.versions = Some(result);
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

    fn resume_navigation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_pending_project_setting(window, cx);
        if self.save_task.is_none()
            && self.build_task.is_none()
            && self.sync_task.is_none()
            && self.snapshot_task.is_none()
            && self.restore_task.is_none()
            && self.comment_write_task.is_none()
            && self.project_rename_task.is_none()
            && self.entry_write_task.is_none()
            && (!self.preparing_exit() || self.ready_to_exit())
            && let Some(navigation) = self.navigation_after_save.take()
        {
            let closing_window = navigation.exits();
            self.request_navigation(navigation, window, cx);
            if closing_window {
                return;
            }
        }
        if self.navigation_after_save.is_none() {
            self.start_queued_build(window, cx);
        }
    }

    fn preparing_exit(&self) -> bool {
        self.navigation_after_save
            .as_ref()
            .is_some_and(Navigation::exits)
    }

    fn ready_to_exit(&self) -> bool {
        self.projects.is_some()
            && self.project_library_task.is_none()
            && self.settings.ready_to_exit()
    }

    fn has_unsaved_changes(&self, navigation: &Navigation, cx: &App) -> bool {
        self.selection.as_ref().is_some_and(|selection| {
            selection.tabs.iter().any(|tab| {
                if let Navigation::CloseFile(file) = navigation
                    && tab.path != *file
                {
                    return false;
                }
                tab.editor
                    .as_ref()
                    .and_then(|editor| editor.as_ref().ok())
                    .is_some_and(|document| {
                        document.state.read(cx).value().as_ref() != document.saved.as_str()
                    })
            })
        })
    }

    fn request_navigation(
        &mut self,
        navigation: Navigation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.preparing_exit()
            || self.discard_prompt.is_some()
            || self.project_action_prompt.is_some()
            || self.entry_delete_prompt.is_some()
            || (self.rename_draft.is_some()
                || self.deleted_projects_dialog.is_some()
                || self.entry_draft.is_some()
                || self.move_draft.is_some()
                || self.deleted_files_dialog.is_some())
                && !navigation.exits()
        {
            return;
        }
        if navigation.exits() {
            self.settings.prepare_exit();
        }
        if (self.build_task.is_some() || self.sync_task.is_some())
            && !matches!(navigation, Navigation::CloseFile(_))
        {
            self.stop_build();
            self.navigation_after_save = Some(navigation);
            return;
        }
        if self.save_task.is_some()
            || !self.save_queue.is_empty()
            || self.restore_task.is_some()
            || self.comment_write_task.is_some()
            || self.project_rename_task.is_some()
            || self.entry_write_task.is_some()
            || navigation.exits() && !self.ready_to_exit()
        {
            self.navigation_after_save = Some(navigation);
            return;
        }
        if !matches!(navigation, Navigation::CloseFile(_))
            && (self.snapshot_needed || self.snapshot_task.is_some())
        {
            self.navigation_after_save = Some(navigation);
            self.start_snapshot(window, cx);
            return;
        }
        if !self.has_unsaved_changes(&navigation, cx) {
            self.navigate(navigation, window, cx);
            return;
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            "Discard unsaved changes?",
            Some("Refract couldn’t save these changes. You can return to the editor to keep your draft."),
            &["Keep editing", "Discard changes"],
            cx,
        );
        self.discard_prompt = Some(cx.spawn_in(window, async move |library, cx| {
            let should_discard = matches!(answer.await, Ok(1));
            let _ = library.update_in(cx, |library, window, cx| {
                library.discard_prompt = None;
                if should_discard {
                    if library.save_task.is_some() {
                        library.navigation_after_save = Some(navigation);
                    } else {
                        library.navigate(navigation, window, cx);
                    }
                }
            });
        }));
    }

    fn navigate(&mut self, navigation: Navigation, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(&navigation, Navigation::CloseFile(_)) {
            let saved = self.flush_chat_draft(cx);
            if !saved && navigation.exits() {
                let _ = window.prompt(
                    PromptLevel::Warning,
                    "Refract could not finish closing safely.",
                    Some("Refract couldn’t save your chat draft. Try again before closing."),
                    &["Return to Refract"],
                    cx,
                );
                return;
            }
        }
        match navigation {
            Navigation::CloseFile(file) => self.close_file(&file, window, cx),
            Navigation::Projects => self.return_to_projects(window, cx),
            Navigation::CloseWindow => window.remove_window(),
            Navigation::RestartForUpdate => self.launch_prepared_update(window, cx),
        }
    }

    fn open_comment(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(comment) = self
            .selection
            .as_ref()
            .and_then(|selection| selection.comments.as_ref()?.as_ref().ok())
            .and_then(|comments| comments.iter().find(|comment| comment.id == id))
            .cloned()
        else {
            return;
        };
        let file = comment.file.replace('\\', "/");
        self.reveal_workspace_pane(Pane::Editor, window, cx);
        self.open_file(file.clone(), window, cx);
        let Some(selection) = &mut self.selection else {
            return;
        };
        let tab = selection.tabs.iter().find(|tab| tab.path == file);
        if let Some(Ok(editor)) = tab.and_then(|tab| tab.editor.as_ref()) {
            selection.comment_status = Self::jump_to_comment(&editor.state, &comment, window, cx);
        } else {
            selection.pending_comment = Some(comment);
        }
        cx.notify();
    }

    fn jump_to_comment(
        editor: &Entity<InputState>,
        comment: &Comment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<&'static str> {
        let position = comment.source_position(editor.read(cx).value().as_ref());
        editor.update(cx, |editor, cx| {
            if let Some((line, column)) = position {
                editor.set_cursor_position(
                    gpui_component::input::Position::new(line, column),
                    window,
                    cx,
                );
            } else {
                editor.focus(window, cx);
            }
        });
        (position.is_none() && !comment.quote.is_empty())
            .then_some("The comment’s original selection is unavailable in this file.")
    }
}

#[derive(Clone, Copy)]
struct Palette {
    canvas: u32,
    library: u32,
    sidebar: u32,
    surface: u32,
    text: u32,
    muted: u32,
    border: u32,
    input_border: u32,
    selection: u32,
    focus: u32,
    warning: u32,
    warning_surface: u32,
}

impl Palette {
    fn for_window(window: &Window) -> Self {
        // Use Refract's desktop palette.
        match window.appearance() {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self {
                canvas: 0x0a0a0a,
                library: 0x000000,
                sidebar: 0x000000,
                surface: 0x111111,
                text: 0xf5f5f5,
                muted: 0xa3a3a3,
                border: 0x191919,
                input_border: 0x1e1e1e,
                selection: 0x1a1b1b,
                focus: 0x79a8ff,
                warning: 0xffb900,
                warning_surface: 0x312108,
            },
            _ => Self {
                canvas: 0xfcfcfc,
                library: 0xfcfcfc,
                sidebar: 0xfafafa,
                surface: 0xffffff,
                text: 0x27272a,
                muted: 0x71717b,
                border: 0xe4e4e7,
                input_border: 0xd4d4d8,
                selection: 0xf0f0f2,
                focus: 0x1b4ed8,
                warning: 0xbb4d00,
                warning_surface: 0xfcf4e8,
            },
        }
    }
}

impl Render for Library {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.thumbnails.release_replaced(window);
        if !self.pdf_is_visible(window)
            && let Some(selection) = &mut self.selection
        {
            selection.build.preview.hide(window);
        }
        let palette = Palette::for_window(window);
        let view = match &self.selection {
            Some(selection) => self
                .workspace(selection, window, palette, cx)
                .into_any_element(),
            None => self.projects(window, palette, cx).into_any_element(),
        };
        self.settings.refresh_dialog();
        let dialog = Root::render_dialog_layer(window, cx);
        let root = Self::find_shortcuts(cx)
            .id("library")
            .track_focus(&self.focus_handle)
            .capture_key_down(cx.listener(|library, event: &gpui::KeyDownEvent, _, cx| {
                let modifiers = event.keystroke.modifiers;
                if !library.keyboard_focus
                    && !(modifiers.control || modifiers.alt || modifiers.platform)
                {
                    library.keyboard_focus = true;
                    cx.notify();
                }
            }))
            .capture_any_mouse_down(cx.listener(|library, _, _, cx| {
                if library.keyboard_focus {
                    library.keyboard_focus = false;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|library, _: &SaveSource, window, cx| {
                library.save_active_file(window, cx)
            }))
            .size_full()
            .flex()
            .bg(rgb(if self.selection.is_some() {
                palette.canvas
            } else {
                palette.library
            }))
            .text_color(rgb(palette.text))
            .text_size(px(13.))
            .line_height(gpui::DefiniteLength::Fraction(1.5))
            .child(view)
            .children(self.entry_menu_layer(palette, cx))
            .when(self.settings.dialog_open(), |root| {
                root.child(div().absolute().inset_0().bg(gpui::rgba(0x00000066)))
            })
            .children(dialog);
        #[cfg(windows)]
        let root = root
            .relative()
            .children(window_chrome::overlay(window, self.selection.is_none()));
        root
    }
}

impl Library {
    fn projects(
        &self,
        window: &Window,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let width = f32::from(window.viewport_size().width);
        let narrow = width <= 800.;
        let padding = if narrow {
            18.
        } else {
            (width * 0.05).clamp(24., 80.)
        };
        let columns = (((width - padding * 2.).min(1120.) + 16.) / 256.)
            .floor()
            .max(1.) as u16;
        let query = self.project_query.read(cx).value().to_lowercase();
        let page = div()
            .size_full()
            .id("projects")
            .overflow_y_scroll()
            .px(px(padding))
            .pt(px(if narrow { 66. } else { 86. }))
            .pb(px(if narrow { 24. } else { 44. }))
            .text_size(px(13.))
            .on_click(cx.listener(|library, _, _, cx| library.dismiss_project_menu(cx)));
        let content = div()
            .w_full()
            .max_w(px(1120.))
            .mx_auto()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(24.))
                    .mb(px(20.))
                    .child(img(self.logo.clone()).size(px(50.)).rounded(px(14.)))
                    .child(
                        div()
                            .text_size(px(32.))
                            .line_height(px(48.))
                            .font_weight(FontWeight::BOLD)
                            .child("Refract"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .when(narrow, |header| header.flex_wrap().gap(px(16.)))
                    .mb(px(32.))
                    .child(
                        div()
                            .text_size(px(28.))
                            .line_height(px(42.))
                            .mb(px(19.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(tracked_label::tracked_label("Your projects", -0.025)),
                    )
                    .child(self.project_library_actions(palette, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.))
                    .border_y_1()
                    .border_color(rgb(palette.border))
                    .py(px(16.))
                    .child(self.project_search_field(palette))
                    .child(div().flex_1())
                    .child(self.catalog.sort_control(palette))
                    .child(
                        div()
                            .flex()
                            .gap(px(2.))
                            .p(px(2.))
                            .border_1()
                            .border_color(rgb(palette.border))
                            .rounded_md()
                            .child(self.view_toggle(false, "List", palette, cx))
                            .child(self.view_toggle(true, "Grid", palette, cx)),
                    )
                    .child(self.deleted_projects_button(palette, cx)),
            )
            .children(self.project_action_error.clone())
            .child(match &self.projects {
                Some(Ok(projects)) if !projects.is_empty() => {
                    let projects = self.catalog.visible_projects(projects, &query);
                    let count = if projects.len() == 1 {
                        "1 project".to_owned()
                    } else {
                        format!("{} projects", projects.len())
                    };
                    div()
                        .when(projects.is_empty(), |content| {
                            content.child(self.no_project_matches(palette, cx))
                        })
                        .when(!projects.is_empty(), |content| {
                            content
                                .child(
                                    div()
                                        .pt(px(20.))
                                        .pb(px(8.))
                                        .text_size(px(12.))
                                        .line_height(px(18.))
                                        .text_color(rgb(palette.muted))
                                        .child(count),
                                )
                                .when(!self.catalog.is_grid(), |content| {
                                    content.child(
                                        div()
                                            .px(px(14.))
                                            .py(px(10.))
                                            .flex()
                                            .border_b_1()
                                            .border_color(rgb(palette.border))
                                            .text_size(px(11.))
                                            .line_height(px(16.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(palette.muted))
                                            .child(
                                                div().flex_1().child(tracked_label::tracked_label(
                                                    "NAME", 0.07,
                                                )),
                                            )
                                            .child(div().w(px(210.)).child(
                                                tracked_label::tracked_label("CREATED", 0.07),
                                            )),
                                    )
                                })
                                .child(
                                    div()
                                        .when(self.catalog.is_grid(), |list| {
                                            list.grid().grid_cols(columns).gap(px(16.)).pt(px(16.))
                                        })
                                        .children(projects.into_iter().map(|project| {
                                            self.project_row(project, !narrow, palette, cx)
                                        })),
                                )
                        })
                }
                Some(Ok(_)) => {
                    div()
                        .py_6()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Make space for your ideas"),
                        )
                        .child(div().text_color(rgb(palette.muted)).child(
                            "Create your first project to start writing and compiling LaTeX.",
                        ))
                        .child(self.create_project_button(
                            "start-project",
                            "Start from Scratch",
                            palette,
                            cx,
                        ))
                }
                None => div()
                    .py_6()
                    .text_color(rgb(palette.muted))
                    .child("Loading projects…"),
                Some(Err(error)) => div()
                    .py_6()
                    .text_color(rgb(palette.muted))
                    .child(format!("Couldn’t read projects: {error}")),
            });
        page.child(content)
    }

    fn project_search_field(&self, palette: Palette) -> impl IntoElement {
        div()
            .debug_selector(|| "project-search".into())
            .w(px(280.))
            .max_w_full()
            .rounded_lg()
            .hover(move |style| style.bg(rgb(palette.selection)))
            .child(
                Input::new(&self.project_query)
                    .appearance(false)
                    .tab_index(1)
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .h(px(36.))
                    .prefix(
                        svg()
                            .path("refract/search.svg")
                            .size(px(18.))
                            .text_color(rgb(palette.muted)),
                    ),
            )
    }

    fn no_project_matches(&self, palette: Palette, cx: &Context<Self>) -> impl IntoElement {
        div()
            .py_6()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("No projects found"),
            )
            .child(
                div()
                    .text_color(rgb(palette.muted))
                    .child("Try a different search or clear your filter."),
            )
            .child(workspace_tools::tool_button(
                "clear-project-search",
                "Clear search",
                false,
                palette,
                cx,
                |library, window, cx| {
                    library.project_query.update(cx, |input, cx| {
                        input.set_value("", window, cx);
                        input.focus(window, cx);
                    });
                },
            ))
    }

    fn view_toggle(
        &self,
        grid: bool,
        label: &'static str,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(label)
            .debug_selector(|| label.to_owned())
            .tab_index(1)
            .cursor_pointer()
            .rounded_md()
            .border_1()
            .border_color(gpui::transparent_black())
            .w(px(42.))
            .h(px(30.))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(12.))
            .text_color(rgb(palette.muted))
            .focus(move |style| style.bg(rgb(palette.selection)))
            .hover(move |style| style.bg(rgb(palette.selection)))
            .when(self.catalog.is_grid() == grid, |button| {
                button
                    .bg(rgb(palette.surface))
                    .border_color(rgb(palette.input_border))
                    .text_color(rgb(palette.text))
                    .shadow_sm()
            })
            .child(label)
            .on_click(cx.listener(move |library, _, _, cx| {
                library.set_library_grid(grid, cx);
            }))
            .on_key_down(
                cx.listener(move |library, event: &gpui::KeyDownEvent, _, cx| {
                    if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                        library.set_library_grid(grid, cx);
                    }
                }),
            )
    }

    fn project_row(
        &self,
        project: &Project,
        show_date: bool,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let keyboard_project = project.clone();
        let click_project = project.clone();
        let grid = self.catalog.is_grid();
        let date = project
            .created
            .as_deref()
            .and_then(|created| chrono::DateTime::parse_from_rfc3339(created).ok())
            .map(|created| {
                created
                    .with_timezone(&chrono::Local)
                    .format("%-m/%-d/%Y")
                    .to_string()
            });

        let project_icon = self
            .thumbnails
            .image(&project.id, grid, palette)
            .map(IntoElement::into_any_element)
            .unwrap_or_else(|| {
                div()
                    .flex()
                    .flex_none()
                    .justify_center()
                    .items_center()
                    .w(px(40.))
                    .h(px(50.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(rgb(palette.input_border))
                    .bg(rgb(palette.surface))
                    .text_color(rgb(palette.muted))
                    .font_family(monospace_font())
                    .text_size(px(9.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("TEX")
                    .into_any_element()
            });

        div()
            .id(SharedString::from(project.id.clone()))
            .track_focus(&self.project_focus[&project.id].open)
            .cursor_pointer()
            .flex()
            .rounded_lg()
            .border_b_1()
            .border_color(rgb(palette.border))
            .focus(move |style| style.border_color(rgb(palette.focus)))
            .hover(move |style| style.bg(rgb(palette.selection)))
            .when(!grid, |row| {
                row.min_h(px(85.))
                    .items_center()
                    .gap(px(16.))
                    .px(px(14.))
                    .py(px(12.))
            })
            .when(grid, |row| {
                row.min_h(px(170.))
                    .relative()
                    .flex_col()
                    .justify_between()
                    .gap(px(20.))
                    .p(px(20.))
                    .border_1()
                    .bg(rgb(palette.surface))
            })
            .child(
                div()
                    .flex()
                    .min_w(px(0.))
                    .when(grid, |body| body.w_full().flex_none().gap(px(16.)))
                    .when(!grid, |body| body.flex_1().gap(px(14.)))
                    .items_center()
                    .child(project_icon)
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex()
                            .flex_1()
                            .flex_col()
                            .child(
                                div()
                                    .when(!grid, |name| name.truncate())
                                    .text_size(px(14.))
                                    .line_height(px(21.))
                                    .font_weight(FontWeight(550.))
                                    .child(project.name.clone()),
                            )
                            .when(grid, |name| {
                                name.child(
                                    div()
                                        .mt(px(6.))
                                        .text_size(px(12.))
                                        .line_height(px(18.))
                                        .text_color(rgb(palette.muted))
                                        .child("Open project →"),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(16.))
                    .when(grid, |footer| footer.w_full().justify_between())
                    .when(show_date, |footer| {
                        footer.child(
                            div()
                                .when(!grid, |date| date.w(px(140.)).flex_none())
                                .text_size(px(12.))
                                .line_height(px(18.))
                                .text_color(rgb(palette.muted))
                                .children(date),
                        )
                    })
                    .child(self.project_actions(project, palette, cx)),
            )
            .on_click(cx.listener(move |library, _, window, cx| {
                library.open_project(click_project.clone(), window, cx);
            }))
            .on_key_down(
                cx.listener(move |library, event: &gpui::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                        library.open_project(keyboard_project.clone(), window, cx);
                    }
                }),
            )
    }

    fn workspace(
        &self,
        selection: &Selection,
        window: &Window,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let width = f32::from(window.viewport_size().width);
        let collapsed = self.workspace_layout.sidebar_collapsed;
        let sidebar = self.workspace_layout.sidebar_width(width);
        let editor_width = self.workspace_layout.editor_width(width);
        let visible = self.displayed_panes(selection, width);
        let left = visible.editor || visible.agent;
        let split = left && (visible.preview || selection.tool.is_some());
        let home = div()
            .id("home")
            .track_focus(&self.home_focus)
            .cursor_pointer()
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded_md()
            .py_1()
            .text_size(px(15.))
            .font_weight(FontWeight(650.))
            .focus(move |style| style.bg(rgb(palette.selection)))
            .hover(move |style| style.bg(rgb(palette.selection)))
            .child(img(self.logo.clone()).size(px(28.)).rounded(px(7.)))
            .child(tracked_label::tracked_label("Refract", -0.02))
            .on_click(cx.listener(|library, _, window, cx| {
                library.request_navigation(Navigation::Projects, window, cx)
            }))
            .on_key_down(
                cx.listener(|library, event: &gpui::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                        library.request_navigation(Navigation::Projects, window, cx);
                    }
                }),
            );
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .h(px(WORKSPACE_HEADER_HEIGHT))
                    .flex_none()
                    .items_center()
                    .bg(rgb(palette.sidebar))
                    .child(
                        div()
                            .w(px(sidebar))
                            .flex_none()
                            .h_full()
                            .flex()
                            .items_center()
                            .relative()
                            .top(px(11.))
                            .px(px(18.))
                            .when(collapsed, |bar| bar.justify_center())
                            .when(!collapsed, |bar| bar.justify_between().child(home))
                            .map(|bar| {
                                #[cfg(windows)]
                                let bar = bar.when(!collapsed && !window.is_fullscreen(), |bar| {
                                    bar.child(window_chrome::drag_space(
                                        px(WORKSPACE_HEADER_HEIGHT - 11.),
                                        px(0.),
                                    ))
                                });
                                bar
                            })
                            .child(self.sidebar_collapse_button(palette, cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .h_full()
                            .flex()
                            .items_center()
                            .pl_3()
                            .overflow_hidden()
                            .border_b_1()
                            .border_color(rgb(palette.border))
                            .child(self.project_name_control(&selection.project, palette, cx))
                            .map(|title| {
                                #[cfg(windows)]
                                let title = title.when(!window.is_fullscreen(), |title| {
                                    title.child(window_chrome::drag_space(
                                        px(WORKSPACE_HEADER_HEIGHT),
                                        window_chrome::resize_strip(window),
                                    ))
                                });
                                title
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h_full()
                            .gap(px(6.))
                            .pr_3()
                            .map(|actions| {
                                #[cfg(windows)]
                                let actions =
                                    actions.pr(px(12.) + window_chrome::controls_width(window));
                                actions
                            })
                            .flex_none()
                            .border_b_1()
                            .border_color(rgb(palette.border))
                            .child(self.workspace_pane_buttons(
                                visible,
                                WorkspaceLayout::compact(width),
                                palette,
                                cx,
                            ))
                            .child(self.workspace_tools_button(selection, palette, cx))
                            .child(self.export_project_button(palette, cx)),
                    ),
            )
            .when(!visible.preview, |workspace| {
                workspace.children(selection.build.sync_error.clone().map(|error| {
                    div()
                        .px_3()
                        .py_1()
                        .text_color(rgb(palette.warning))
                        .child(error)
                }))
            })
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.))
                    .child(
                        div()
                            .w(px(sidebar))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .bg(rgb(palette.sidebar))
                            .border_r_1()
                            .border_color(rgb(palette.border))
                            .when(!collapsed, |panel| {
                                panel
                                    .px_3()
                                    .pb_3()
                                    .child(
                                        div()
                                            .flex_none()
                                            .pt(px(59.))
                                            .pb_2()
                                            .px(px(10.))
                                            .text_size(px(11.))
                                            .line_height(px(16.5))
                                            .font_weight(FontWeight(650.))
                                            .text_color(rgb(palette.muted))
                                            .child(tracked_label::tracked_label("PROJECT", 0.12)),
                                    )
                                    .children(self.project_action_error.clone())
                                    .child(self.project_sidebar_tabs(selection, palette, cx))
                                    .when(!selection.conversation.sidebar_open, |sidebar| {
                                        sidebar
                                            .children(selection.file_action_error.clone())
                                            .child(self.file_search_panel(selection, palette, cx))
                                            .child(self.file_tree(selection, palette, cx))
                                            .child(self.outline_pane(selection, palette, cx))
                                    })
                                    .when(selection.conversation.sidebar_open, |sidebar| {
                                        sidebar
                                            .child(self.conversation_list(selection, palette, cx))
                                    })
                            }),
                    )
                    .when(!collapsed, |body| {
                        body.child(self.workspace_divider(Divider::Sidebar, sidebar, palette, cx))
                    })
                    .when(left, |body| {
                        body.child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w(px(0.))
                                .min_h(px(0.))
                                .when(split, |pane| pane.w(px(editor_width)).flex_none())
                                .when(!split, |pane| pane.flex_1())
                                .when(visible.editor, |pane| {
                                    pane.child(self.source_pane(selection, width, palette, cx))
                                })
                                .when(visible.agent, |pane| {
                                    pane.child(self.conversation_pane(
                                        selection,
                                        visible.editor,
                                        window,
                                        palette,
                                        cx,
                                    ))
                                }),
                        )
                    })
                    .when(split, |body| {
                        body.child(self.workspace_divider(
                            Divider::Editor,
                            editor_width,
                            palette,
                            cx,
                        ))
                    })
                    .when_some(selection.tool, |body, tool| {
                        body.child(self.workspace_tools_pane(selection, tool, palette, cx))
                    })
                    .when(visible.preview, |body| {
                        let pane_width = if split {
                            width - sidebar - editor_width
                        } else {
                            width - sidebar
                        };
                        body.child(self.compile_pane(&selection.build, pane_width, palette, cx))
                    }),
            )
    }

    fn versions_pane(
        &self,
        selection: &Selection,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let pane = div()
            .id("versions-pane")
            .size_full()
            .flex()
            .flex_col()
            .p_4()
            .gap_2()
            .child(tool_heading("Versions"));
        let list = div()
            .id("version-list")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll();
        pane.children(selection.restore_error.clone())
            .child(match &selection.versions {
                None => list.child("Loading versions…"),
                Some(Err(error)) => list.child(format!("Couldn’t load version history: {error}")),
                Some(Ok(versions)) if versions.is_empty() => list.child("No saved versions."),
                Some(Ok(versions)) => {
                    list.children(versions.iter().enumerate().map(|(index, version)| {
                        self.version_row(
                            version,
                            versions.len() - index,
                            selection.comparison.as_ref(),
                            palette,
                            cx,
                        )
                    }))
                }
            })
    }

    fn file_tree(
        &self,
        selection: &Selection,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let tree = div()
            .id("file-tree")
            .track_scroll(&self.outline.file_scroll)
            .flex_1()
            .min_h(px(48.))
            .pt(px(10.))
            .overflow_y_scroll();
        match &selection.tree {
            None => tree.child(div().text_color(rgb(palette.muted)).child("Loading files…")),
            Some(Err(error)) => tree.child(format!("Couldn’t list project files: {error}")),
            Some(Ok(entries)) => tree.children(
                entries
                    .iter()
                    .map(|entry| Self::file_tree_entry(entry, selection, palette, 0, cx)),
            ),
        }
    }

    fn file_tree_entry(
        entry: &ProjectEntry,
        selection: &Selection,
        palette: Palette,
        depth: usize,
        cx: &Context<Self>,
    ) -> AnyElement {
        let path = entry.path.clone();
        let keyboard_path = path.clone();
        let is_folder = entry.children.is_some();
        let expanded = selection.expanded_folders.contains(&entry.path);
        let active = selection.file.as_deref() == Some(entry.path.as_str());
        let inset = if is_folder { 4. } else { 9. } + 12. * depth as f32;
        let row = div()
            .id(SharedString::from(format!("tree/{}", entry.path)))
            .debug_selector(|| format!("tree/{}", entry.path))
            .tab_index(1)
            .cursor_pointer()
            .flex()
            .items_center()
            .relative()
            .overflow_hidden()
            .gap_2()
            .rounded(px(5.))
            .pl(px(inset))
            .pr(px(32.))
            .py(px(6.))
            .min_h(px(33.))
            .text_size(px(13.))
            .hover(move |style| style.bg(rgb(palette.selection)))
            .focus(move |style| style.text_color(rgb(palette.focus)))
            .when(active, |row| {
                row.bg(rgb(palette.selection))
                    .font_weight(FontWeight(550.))
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .bottom_0()
                            .w(px(2.))
                            .bg(rgb(palette.focus)),
                    )
            })
            .when(is_folder, |row| {
                row.text_color(rgb(palette.muted))
                    .child(
                        svg()
                            .path(if expanded {
                                "refract/chevron-down.svg"
                            } else {
                                "refract/chevron-right.svg"
                            })
                            .size(px(14.))
                            .flex_none()
                            .text_color(rgb(palette.muted)),
                    )
                    .child(
                        svg()
                            .path("refract/folder.svg")
                            .size(px(15.))
                            .flex_none()
                            .text_color(rgb(palette.muted)),
                    )
            })
            .when(!is_folder, |row| {
                let extension: String = entry
                    .name
                    .rsplit('.')
                    .next()
                    .unwrap_or("file")
                    .chars()
                    .take(4)
                    .collect();
                row.child(
                    div()
                        .flex_none()
                        .rounded(px(3.))
                        .px(px(3.))
                        .py(px(1.))
                        .border_1()
                        .border_color(rgb(palette.input_border))
                        .font_family(monospace_font())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(8.))
                        .line_height(px(12.))
                        .text_color(rgb(palette.muted))
                        .child(extension.to_uppercase()),
                )
            })
            .child(div().min_w(px(0.)).truncate().child(entry.name.clone()))
            .on_click(cx.listener(move |library, _, window, cx| {
                if is_folder {
                    library.toggle_folder(&path, cx);
                } else {
                    library.reveal_workspace_pane(Pane::Editor, window, cx);
                    library.open_file(path.clone(), window, cx);
                }
            }))
            .on_key_down(
                cx.listener(move |library, event: &gpui::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                        if is_folder {
                            library.toggle_folder(&keyboard_path, cx);
                        } else {
                            library.reveal_workspace_pane(Pane::Editor, window, cx);
                            library.open_file(keyboard_path.clone(), window, cx);
                        }
                    }
                }),
            );
        let row = Self::entry_menu(row, entry.path.clone(), is_folder, active, palette, cx);
        let mut node = div()
            .id(SharedString::from(format!("entry/{}", entry.path)))
            .child(row);
        if let Some(children) = entry.children.as_ref().filter(|_| expanded) {
            node = node.children(
                children
                    .iter()
                    .map(|entry| Self::file_tree_entry(entry, selection, palette, depth + 1, cx)),
            );
        }
        node.into_any_element()
    }

    fn file_tabs(
        &self,
        selection: &Selection,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("file-tabs")
            .flex()
            .flex_none()
            .w_full()
            .min_h(px(44.))
            .gap_1()
            .overflow_x_scroll()
            .children(selection.tabs.iter().map(|tab| {
                let is_active = selection.file.as_deref() == Some(tab.path.as_str());
                let text = if is_active {
                    palette.text
                } else {
                    palette.muted
                };
                let name = tab.path.rsplit('/').next().unwrap_or(&tab.path).to_owned();
                let open = tab.path.clone();
                let open_by_key = open.clone();
                let close = open.clone();
                let close_by_key = open.clone();
                div()
                    .id(SharedString::from(format!("tab/{}", tab.path)))
                    .flex()
                    .flex_none()
                    .items_center()
                    .border_b_2()
                    .border_color(gpui::transparent_black())
                    .text_color(rgb(text))
                    .when(is_active, |tab| {
                        tab.bg(rgb(palette.selection))
                            .border_color(rgb(palette.focus))
                    })
                    .child(
                        div()
                            .id("open")
                            .track_focus(&tab.open_focus)
                            .cursor_pointer()
                            .py(px(9.))
                            .px(px(8.))
                            .text_size(px(12.))
                            .focus(move |style| style.text_color(rgb(palette.focus)))
                            .hover(move |style| style.bg(rgb(palette.selection)))
                            .child(name)
                            .on_click(cx.listener(move |library, _, window, cx| {
                                library.open_file(open.clone(), window, cx)
                            }))
                            .on_key_down(cx.listener(
                                move |library, event: &gpui::KeyDownEvent, window, cx| {
                                    if event.keystroke.key == "enter"
                                        || event.keystroke.key == "space"
                                    {
                                        library.open_file(open_by_key.clone(), window, cx);
                                    }
                                },
                            )),
                    )
                    .child(
                        div()
                            .id("close")
                            .track_focus(&tab.close_focus)
                            .cursor_pointer()
                            .px(px(6.))
                            .text_size(px(16.))
                            .focus(move |style| style.text_color(rgb(palette.focus)))
                            .hover(move |style| style.bg(rgb(palette.selection)))
                            .child("×")
                            .on_click(cx.listener(move |library, _, window, cx| {
                                library.request_navigation(
                                    Navigation::CloseFile(close.clone()),
                                    window,
                                    cx,
                                )
                            }))
                            .on_key_down(cx.listener(
                                move |library, event: &gpui::KeyDownEvent, window, cx| {
                                    if event.keystroke.key == "enter"
                                        || event.keystroke.key == "space"
                                    {
                                        library.request_navigation(
                                            Navigation::CloseFile(close_by_key.clone()),
                                            window,
                                            cx,
                                        );
                                    }
                                },
                            )),
                    )
            }))
    }

    fn source_pane(
        &self,
        selection: &Selection,
        viewport_width: f32,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let pane = div()
            .relative()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .flex()
            .flex_col()
            .overflow_hidden()
            .when(selection.retained_restore_stages, |pane| {
                pane.child(
                    div().p_2().bg(rgb(palette.warning_surface)).child(
                        "Temporary restore data remains in this project. It has been preserved because it could not be safely cleaned.",
                    ),
                )
            });
        let Some(path) = &selection.file else {
            return pane.child(
                div()
                    .text_color(rgb(palette.muted))
                    .child("Choose a file from the sidebar."),
            );
        };
        let editor = selection
            .tabs
            .iter()
            .find(|tab| &tab.path == path)
            .and_then(|tab| tab.editor.as_ref());
        let mut header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .min_h(px(40.))
            .gap(px(6.))
            .py(px(5.))
            .border_t_1()
            .border_color(rgb(palette.border))
            .text_size(px(12.))
            .text_color(rgb(palette.muted))
            .child(self.show_in_pdf_button(selection, palette, cx));
        if let Some(Ok(editor)) = editor {
            header = header
                .child(self.commands_button(palette, cx))
                .child(self.source_find_button(palette, cx));
            if editor.save_error.is_some() {
                header = header.child(workspace_tools::tool_button(
                    "save-file",
                    "Retry save",
                    false,
                    palette,
                    cx,
                    |library, window, cx| library.save_active_file(window, cx),
                ));
            }
        }
        let preview = match (&selection.asset_preview, editor) {
            (Some(asset), _) => self.asset_preview(asset, palette, cx).into_any_element(),
            (_, None) => div().child("Loading source…").into_any_element(),
            (_, Some(Err(error))) => div()
                .child(format!("Couldn’t open source: {error}"))
                .into_any_element(),
            (_, Some(Ok(editor))) => self.source_editor(editor, cx).into_any_element(),
        };
        let header_inset = if viewport_width <= 800. { 10. } else { 16. };
        pane.child(
            div()
                .flex_none()
                .px(px(header_inset))
                .border_b_1()
                .border_color(rgb(palette.border))
                .child(self.file_tabs(selection, palette, cx))
                .child(header),
        )
        .when(
            self.editor_find.visible && editor.is_some_and(Result::is_ok),
            |pane| pane.child(self.source_find_panel(palette, cx)),
        )
        .children(
            editor
                .and_then(|editor| editor.as_ref().ok()?.save_error.clone())
                .map(|error| format!("Couldn’t save source: {error}")),
        )
        .children(selection.comment_status)
        .children(self.commands_popup(palette, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .pt(px(20.))
                .child(preview),
        )
    }
}

fn monospace_font() -> &'static str {
    if cfg!(target_os = "windows") {
        "Consolas"
    } else if cfg!(target_os = "macos") {
        "Menlo"
    } else {
        "Liberation Mono"
    }
}

fn dialog_title(text: impl Into<SharedString>) -> impl gpui::IntoElement {
    div()
        .text_size(px(18.))
        .line_height(px(27.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(tracked_label::tracked_label(text, -0.025))
}

fn tool_heading(text: &'static str) -> impl gpui::IntoElement {
    div()
        .text_size(px(18.))
        .line_height(px(27.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(tracked_label::tracked_label(text, -0.02))
}

fn collect_expanded_folders(entries: &[ProjectEntry], expanded: &mut HashSet<String>) {
    for entry in entries {
        if let Some(children) = &entry.children {
            expanded.insert(entry.path.clone());
            collect_expanded_folders(children, expanded);
        }
    }
}

fn preferred_source<'a>(entries: &'a [ProjectEntry], preferred: &str) -> Option<&'a str> {
    let mut first = None;
    for entry in entries {
        let file = if let Some(children) = &entry.children {
            preferred_source(children, preferred)
        } else {
            Some(entry.path.as_str())
        };
        if file == Some(preferred) {
            return file;
        }
        first = first.or(file);
    }
    first
}

fn projects_dir() -> Option<PathBuf> {
    if let Some(profile) = env::var_os("REFRACT_DATA_DIR").filter(|path| !path.is_empty()) {
        return Some(PathBuf::from(profile).join("projects"));
    }
    if cfg!(target_os = "windows") {
        return env::var_os("APPDATA")
            .map(|dir| PathBuf::from(dir).join("Refract").join("projects"));
    }
    let config_dir = if cfg!(target_os = "macos") {
        env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        env::var_os("XDG_CONFIG_HOME")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    };
    config_dir.map(|dir| refract_native::profile_path::unix_profile_dir(&dir).join("projects"))
}

fn initialize_app(cx: &mut App) {
    gpui_component::init(cx);
    source_theme::init(cx);
    conversation_view::init(cx);
    commands_view::init(cx);
    editor_find_view::init(cx);
    pdf_view::init(cx);
    file_trash_view::init(cx);
    comments_view::init(cx);
    deleted_projects_view::init(cx);
    file_creation_view::init(cx);
    cx.bind_keys([KeyBinding::new("secondary-s", SaveSource, None)]);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let projects_dir = projects_dir().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "Couldn’t locate the Refract profile. Set REFRACT_DATA_DIR to its folder.",
        )
    })?;
    let Some(instance) = instance::Instance::claim(projects_dir.parent().unwrap_or(&projects_dir))?
    else {
        return Ok(());
    };
    let activations = instance.activations();
    let app = Application::new().with_assets(RefractAssets);
    app.run(move |cx: &mut App| {
        initialize_app(cx);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
        let window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                #[cfg(windows)]
                titlebar: Some(window_chrome::titlebar_options()),
                #[cfg(target_os = "linux")]
                app_id: Some("refract".into()),
                ..Default::default()
            },
            |window, cx| {
                window.set_window_title("Refract");
                let library = cx.new(|cx| Library::new(projects_dir, window, cx));
                library.update(cx, |library, cx| library.start_update_checks(window, cx));
                let close_guard = library.downgrade();
                window.on_window_should_close(cx, move |window, cx| {
                    let guard = close_guard.clone();
                    window.defer(cx, move |window, cx| {
                        let _ = guard.update(cx, |library, cx| {
                            library.request_navigation(Navigation::CloseWindow, window, cx);
                        });
                    });
                    false
                });
                cx.new(|cx| Root::new(library, window, cx))
            },
        );
        match window {
            Ok(window) => {
                cx.activate(true);
                cx.spawn(async move |cx| {
                    while activations.wait().await {
                        if window
                            .update(cx, |_, window, cx| {
                                cx.activate(true);
                                window.activate_window();
                            })
                            .is_err()
                        {
                            break;
                        }
                        activations.acknowledge();
                    }
                })
                .detach();
            }
            Err(error) => {
                eprintln!("Couldn’t open the Refract window: {error}");
                std::process::exit(1);
            }
        }
    });
    Ok(())
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use std::fs;

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        cx.refresh().unwrap();
        let bounds = cx.debug_bounds(selector).expect(selector);
        cx.simulate_click(bounds.center(), Modifiers::none());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn closing_waits_for_project_writes(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let marker = profile.path().join("write-completed");
        let saved = marker.clone();
        let (finish, wait) = async_channel::bounded::<()>(1);

        cx.update(initialize_app);
        let mut library = None;
        let (_, view) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| Library::new(projects, window, cx));
            library = Some(app.clone());
            Root::new(app, window, cx)
        });
        let library = library.unwrap();
        view.update(|window, app| {
            library.update(app, |library, cx| {
                library.entry_write_task = Some(cx.spawn_in(window, async move |library, cx| {
                    wait.recv().await.unwrap();
                    fs::write(saved, "Saved").unwrap();
                    library
                        .update_in(cx, |library, window, cx| {
                            library.entry_write_task = None;
                            library.resume_navigation(window, cx);
                        })
                        .unwrap();
                }));
                library.request_navigation(Navigation::CloseWindow, window, cx);
                library.request_navigation(Navigation::Projects, window, cx);
                assert!(library.preparing_exit());
            });
        });
        cx.run_until_parked();
        assert!(!marker.exists());
        cx.update(|app| assert_eq!(app.windows().len(), 1));

        finish.try_send(()).unwrap();
        cx.run_until_parked();
        assert_eq!(fs::read_to_string(marker).unwrap(), "Saved");
        cx.update(|app| assert!(app.windows().is_empty()));
    }

    #[gpui::test]
    fn confirms_version_restores_and_reloads_the_editor(cx: &mut TestAppContext) {
        let projects = tempfile::tempdir().unwrap();
        let id = "b3b9ffb5-13c0-4d22-b427-54520347684a";
        let folder = projects.path().join(id);
        fs::create_dir(&folder).unwrap();
        fs::write(
            folder.join(".refract.json"),
            serde_json::json!({"id": id, "name": "Paper"}).to_string(),
        )
        .unwrap();
        let source = folder.join("main.tex");
        fs::write(&source, "Original\n").unwrap();
        let project = list_projects(projects.path()).unwrap().pop().unwrap();
        let version = snapshot_project(projects.path(), &project).unwrap();
        fs::write(&source, "Current\n").unwrap();
        let notes = folder.join("notes.tex");
        fs::write(&notes, "Later notes").unwrap();

        cx.update(initialize_app);
        let mut library = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Library::new(projects.path().to_path_buf(), window, cx));
            library = Some(view.clone());
            Root::new(view, window, cx)
        });
        let library = library.unwrap();
        cx.update(|window, app| {
            library.update(app, |library, cx| {
                library.open_project(project, window, cx);
                library.select_workspace_tool(WorkspaceTool::Versions, window, cx);
            })
        });
        cx.run_until_parked();
        let request = |window: &mut Window, app: &mut App| {
            library.update(app, |library, cx| {
                library.request_version_restore(&version.id, RestoreKind::File, window, cx)
            });
        };
        cx.update(request);
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert_eq!(fs::read_to_string(&source).unwrap(), "Current\n");

        cx.update(request);
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Restore file");
        cx.run_until_parked();
        assert_eq!(fs::read_to_string(&source).unwrap(), "Original\n");
        cx.update(|_, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            let document = selection.tabs[0].editor.as_ref().unwrap().as_ref().unwrap();
            assert_eq!(document.state.read(app).value().as_ref(), "Original\n");
            assert_eq!(document.saved, "Original\n");
            assert_eq!(
                selection.versions.as_ref().unwrap().as_ref().unwrap().len(),
                3
            );
            assert!(selection.restore_error.is_none());
        });
        cx.update(|window, app| {
            library.update(app, |library, cx| {
                library.open_file("notes.tex".into(), window, cx)
            });
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            library.update(app, |library, cx| {
                library.request_version_restore(&version.id, RestoreKind::Project, window, cx)
            });
        });
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Restore project");
        cx.run_until_parked();
        assert!(!notes.exists());
        cx.update(|_, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            assert_eq!(selection.file.as_deref(), Some("main.tex"));
            assert_eq!(selection.tabs.len(), 1);
            let document = selection.tabs[0].editor.as_ref().unwrap().as_ref().unwrap();
            assert_eq!(document.state.read(app).value().as_ref(), "Original\n");
            assert!(selection.restore_error.is_none());
        });
    }

    #[gpui::test]
    fn autosaves_edit_bursts_and_keeps_conflicting_drafts_until_confirmed(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let id = "b3b9ffb5-13c0-4d22-b427-54520347684a";
        let project = projects.join(id);
        fs::create_dir_all(&project).unwrap();
        fs::write(
            project.join(".refract.json"),
            serde_json::json!({"id": id, "name": "A paper", "created": "2026-01-01T00:00:00Z"})
                .to_string(),
        )
        .unwrap();
        fs::write(project.join("main.tex"), "A line of LaTeX\n").unwrap();
        fs::create_dir(project.join("sections")).unwrap();
        fs::write(project.join("sections/abstract.tex"), "An abstract\n").unwrap();
        fs::write(
            project.join(".refract-comments.json"),
            serde_json::json!([{
                "id": "fixture-comment",
                "file": "sections/abstract.tex",
                "quote": "abstract",
                "text": "Check this statement",
                "resolved": true,
            "replies": [{"text": "Checked"}],
            "anchor": {"start": 3, "end": 11, "prefix": "An ", "suffix": "\n", "sourceHash": "", "status": "attached"}
            }])
            .to_string(),
        )
        .unwrap();

        cx.update(initialize_app);
        let mut library = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Library::new(projects, window, cx));
            library = Some(view.clone());
            Root::new(view, window, cx)
        });
        let library = library.unwrap();
        cx.refresh().unwrap();
        click(cx, "project-search");
        cx.simulate_input("Missing project");
        click(cx, "clear-project-search");
        cx.update(|_, app| assert!(library.read(app).project_query.read(app).value().is_empty()));
        cx.simulate_input("PAPER");
        click(cx, "Grid");
        cx.update(|_, app| assert!(library.read(app).catalog.is_grid()));
        cx.simulate_keystrokes("tab tab enter");
        cx.update(|_, app| {
            let selection = library.read(app).selection.as_ref().expect("open project");
            assert_eq!(selection.project.id, id);
            let editor = selection.tabs[0].editor.as_ref().unwrap().as_ref().unwrap();
            assert_eq!(editor.state.read(app).value().as_ref(), "A line of LaTeX\n");
        });
        click(cx, "tree/sections");
        cx.update(|_, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            assert!(!selection.expanded_folders.contains("sections"));
        });
        cx.simulate_keystrokes("space");
        cx.update(|_, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            assert!(selection.expanded_folders.contains("sections"));
        });
        cx.simulate_keystrokes("tab tab enter");
        cx.update(|window, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            assert_eq!(selection.file.as_deref(), Some("sections/abstract.tex"));
            assert_eq!(selection.tabs.len(), 2);
            let editor = selection.tabs[1]
                .editor
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .state
                .clone();
            assert_eq!(editor.read(app).value().as_ref(), "An abstract\n");
            editor.update(app, |editor, cx| {
                editor.set_cursor_position(gpui_component::input::Position::new(0, 11), window, cx);
            });
        });
        cx.update(|window, app| {
            window.dispatch_keystroke(gpui::Keystroke::parse("enter").unwrap(), app);
        });
        cx.update(|window, app| {
            assert!(library.read(app).save_task.is_some());
            window.dispatch_keystroke(gpui::Keystroke::parse("A").unwrap(), app);
        });
        cx.update(|_, app| assert_eq!(library.read(app).save_queue.len(), 1));
        cx.simulate_input(" draft");
        assert_eq!(
            fs::read_to_string(project.join("sections/abstract.tex")).unwrap(),
            "An abstract\nA draft\n"
        );
        cx.executor().advance_clock(Duration::from_secs(3));
        cx.run_until_parked();
        cx.update(|_, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            let versions = selection.versions.as_ref().unwrap().as_ref().unwrap();
            assert_eq!(versions.len(), 2);
            assert_eq!(versions[0].sequence, 2);
        });
        cx.update(|window, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            window.focus(&selection.tabs[0].open_focus);
        });
        cx.simulate_keystrokes("enter");
        cx.update(|_, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            assert_eq!(selection.file.as_deref(), Some("main.tex"));
            assert_eq!(selection.tabs.len(), 2);
        });
        cx.simulate_keystrokes("tab enter");
        cx.update(|window, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            assert_eq!(selection.file.as_deref(), Some("sections/abstract.tex"));
            assert_eq!(selection.tabs.len(), 1);
            assert!(selection.tabs[0].open_focus.is_focused(window));
            let editor = selection.tabs[0].editor.as_ref().unwrap().as_ref().unwrap();
            assert_eq!(
                editor.state.read(app).value().as_ref(),
                "An abstract\nA draft\n"
            );
        });
        cx.simulate_keystrokes("tab enter");
        cx.update(|window, app| {
            let library = library.read(app);
            let selection = library.selection.as_ref().unwrap();
            assert!(selection.file.is_none());
            assert!(selection.tabs.is_empty());
            assert!(library.home_focus.is_focused(window));
        });
        click(cx, "project-tools");
        cx.update(|_, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            assert_eq!(selection.tool, Some(WorkspaceTool::Logs));
        });
        click(cx, "tool-versions");
        cx.update(|_, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            assert_eq!(selection.tool, Some(WorkspaceTool::Versions));
        });
        click(cx, "compare-version-1");
        cx.update(|_, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            let comparison = selection.comparison.as_ref().unwrap();
            let changes = comparison.changes.as_ref().unwrap().as_ref().unwrap();
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[0].name, "sections/abstract.tex");
        });
        click(cx, "compared-file-0");
        cx.update(|_, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            let comparison = selection.comparison.as_ref().unwrap();
            let file = comparison.selection.as_ref().unwrap();
            let lines = file.lines.as_ref().unwrap().as_ref().unwrap();
            let current = &lines.current.as_ref().unwrap().content;
            assert_eq!(current.lines[current.changed_indices[0]].text, "A draft");
        });
        click(cx, "tool-comments");
        cx.update(|_, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            assert_eq!(selection.tool, Some(WorkspaceTool::Comments));
            let comments = selection.comments.as_ref().unwrap().as_ref().unwrap();
            assert_eq!(comments[0].replies[0].text, "Checked");
            assert!(comments[0].resolved);
        });
        click(cx, "comment-reference-fixture-comment");
        cx.update(|_, app| {
            let library = library.read(app);
            let selection = library.selection.as_ref().unwrap();
            assert_eq!(selection.file.as_deref(), Some("sections/abstract.tex"));
            let editor = selection.tabs[0].editor.as_ref().unwrap().as_ref().unwrap();
            assert_eq!(
                editor.state.read(app).cursor_position(),
                gpui_component::input::Position::new(0, 3)
            );
            assert!(selection.comment_status.is_none());
        });
        fs::write(project.join("sections/abstract.tex"), "An external edit\n").unwrap();
        cx.simulate_input("x");
        cx.update(|window, app| {
            let library = library.read(app);
            let selection = library.selection.as_ref().unwrap();
            let editor = selection.tabs[0].editor.as_ref().unwrap().as_ref().unwrap();
            assert!(
                editor
                    .save_error
                    .as_ref()
                    .unwrap()
                    .contains("changed on disk")
            );
            assert!(
                editor
                    .state
                    .read(app)
                    .value()
                    .as_ref()
                    .contains("xabstract")
            );
            window.focus(&library.home_focus);
        });
        cx.simulate_keystrokes("enter");
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Keep editing");
        cx.run_until_parked();
        cx.update(|_, app| assert!(library.read(app).selection.is_some()));
        cx.simulate_keystrokes("enter");
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Discard changes");
        cx.run_until_parked();
        cx.update(|window, app| {
            let library = library.read(app);
            assert!(library.selection.is_none());
            assert!(library.catalog.is_grid());
            assert!(library.project_focus[id].open.is_focused(window));
        });
        assert_eq!(
            fs::read_to_string(project.join("sections/abstract.tex")).unwrap(),
            "An external edit\n"
        );
    }
}
