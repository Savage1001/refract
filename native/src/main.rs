use gpui::{
    AnyElement, App, Application, Bounds, Context, Entity, FocusHandle, FontWeight, Image,
    ImageFormat, SharedString, Task, Window, WindowAppearance, WindowBounds, WindowOptions, div,
    img, prelude::*, px, rgb, size,
};
use gpui_component::{
    Root,
    input::{Input, InputState},
};
use refract_native::{Project, ProjectEntry, list_projects, project_tree, read_source};
use std::{
    collections::{HashMap, HashSet},
    env, io,
    path::PathBuf,
    sync::Arc,
};

struct Library {
    projects_dir: PathBuf,
    projects: Option<io::Result<Vec<Project>>>,
    selection: Option<Selection>,
    tree_task: Option<Task<()>>,
    preview_task: Option<Task<()>>,
    focus_handle: FocusHandle,
    project_focus: HashMap<String, FocusHandle>,
    home_focus: FocusHandle,
    logo: Arc<Image>,
    grid_view: bool,
}

struct Selection {
    project: Project,
    tree: Option<io::Result<Vec<ProjectEntry>>>,
    expanded_folders: HashSet<String>,
    file: Option<String>,
    tabs: Vec<FileTab>,
}

struct FileTab {
    path: String,
    open_focus: FocusHandle,
    close_focus: FocusHandle,
    editor: Option<io::Result<Entity<InputState>>>,
}

impl Library {
    fn new(projects_dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let path = projects_dir.clone();
        let load = cx
            .background_executor()
            .spawn(async move { list_projects(&path) });
        cx.spawn(async move |library, cx| {
            let projects = load.await;
            let _ = library.update(cx, |library, cx| {
                if let Ok(projects) = &projects {
                    library.project_focus.extend(projects.iter().map(|project| {
                        (
                            project.id.clone(),
                            cx.focus_handle().tab_stop(true).tab_index(2),
                        )
                    }));
                }
                library.projects = Some(projects);
                cx.notify();
            });
        })
        .detach();

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        Self {
            projects_dir,
            projects: None,
            selection: None,
            tree_task: None,
            preview_task: None,
            focus_handle,
            project_focus: HashMap::new(),
            home_focus: cx.focus_handle().tab_stop(true).tab_index(0),
            logo: Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("../../resources/logo.png").to_vec(),
            )),
            grid_view: false,
        }
    }

    fn open_project(&mut self, project: Project, window: &mut Window, cx: &mut Context<Self>) {
        let file = project.root_file().replace('\\', "/");
        let expanded_folders = file
            .match_indices('/')
            .map(|(index, _)| file[..index].to_owned())
            .collect();
        self.selection = Some(Selection {
            project: project.clone(),
            tree: None,
            expanded_folders,
            file: None,
            tabs: Vec::new(),
        });
        let path = self.projects_dir.clone();
        let load = cx
            .background_executor()
            .spawn(async move { project_tree(&path, &project) });
        self.tree_task = Some(cx.spawn(async move |library, cx| {
            let tree = load.await;
            let _ = library.update(cx, |library, cx| {
                if let Some(selection) = &mut library.selection {
                    selection.tree = Some(tree);
                    cx.notify();
                }
            });
        }));
        self.open_file(file, window, cx);
        window.focus(&self.home_focus);
    }

    fn open_file(&mut self, file: String, window: &Window, cx: &mut Context<Self>) {
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
        self.preview_task = None;
        if selection
            .tabs
            .iter()
            .any(|tab| tab.path == file && tab.editor.as_ref().is_some_and(Result::is_ok))
        {
            cx.notify();
            return;
        }
        let project = selection.project.clone();
        let path = self.projects_dir.clone();
        let opened_file = file.clone();
        let load = cx
            .background_executor()
            .spawn(async move { read_source(&path, &project, &opened_file) });
        self.preview_task = Some(cx.spawn_in(window, async move |library, cx| {
            let source = load.await;
            let _ = library.update_in(cx, |library, window, cx| {
                let Some(selection) = &mut library.selection else {
                    return;
                };
                let Some(tab) = selection.tabs.iter_mut().find(|tab| tab.path == file) else {
                    return;
                };
                tab.editor = Some(source.map(|source| {
                    cx.new(|cx| {
                        InputState::new(window, cx)
                            .code_editor("text")
                            .soft_wrap(false)
                            .default_value(source)
                    })
                }));
                cx.notify();
            });
        }));
        cx.notify();
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
                selection.file = None;
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
                .unwrap_or(&self.home_focus);
            window.focus(focus);
        }
        cx.notify();
    }

    fn return_to_projects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tree_task = None;
        self.preview_task = None;
        if let Some(previous) = self.selection.take()
            && let Some(focus) = self.project_focus.get(&previous.project.id)
        {
            window.focus(focus);
        }
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
}

#[derive(Clone, Copy)]
struct Palette {
    canvas: u32,
    sidebar: u32,
    surface: u32,
    text: u32,
    muted: u32,
    border: u32,
    input_border: u32,
    selection: u32,
    focus: u32,
}

impl Palette {
    fn for_window(window: &Window) -> Self {
        // Use Refract's desktop palette.
        match window.appearance() {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self {
                canvas: 0x0a0a0a,
                sidebar: 0x000000,
                surface: 0x111111,
                text: 0xf5f5f5,
                muted: 0xa3a3a3,
                border: 0x191919,
                input_border: 0x1e1e1e,
                selection: 0x1a1b1b,
                focus: 0x79a8ff,
            },
            _ => Self {
                canvas: 0xfcfcfc,
                sidebar: 0xfafafa,
                surface: 0xffffff,
                text: 0x27272a,
                muted: 0x71717b,
                border: 0xe4e4e7,
                input_border: 0xd4d4d8,
                selection: 0xf0f0f2,
                focus: 0x1b4ed8,
            },
        }
    }
}

impl Render for Library {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_window(window);
        let view = match &self.selection {
            Some(selection) => self.workspace(selection, palette, cx).into_any_element(),
            None => self.projects(window, palette, cx).into_any_element(),
        };
        div()
            .id("library")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .bg(rgb(palette.canvas))
            .text_color(rgb(palette.text))
            .text_sm()
            .child(view)
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
        let padding = (width * 0.05).clamp(24., 80.);
        let columns = (((width - padding * 2.).min(1120.) + 16.) / 256.)
            .floor()
            .max(1.) as u16;
        let page = div()
            .size_full()
            .id("projects")
            .overflow_y_scroll()
            .px(px(padding))
            .py(px(44.));
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
                            .font_weight(FontWeight::BOLD)
                            .child("Refract"),
                    ),
            )
            .child(
                div()
                    .text_size(px(28.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .mb(px(40.))
                    .child("Your projects"),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .border_y_1()
                    .border_color(rgb(palette.border))
                    .py(px(16.))
                    .child(
                        div()
                            .flex()
                            .p(px(2.))
                            .border_1()
                            .border_color(rgb(palette.border))
                            .rounded_md()
                            .child(self.view_toggle(false, "List", palette, cx))
                            .child(self.view_toggle(true, "Grid", palette, cx)),
                    ),
            )
            .child(match &self.projects {
                Some(Ok(projects)) if !projects.is_empty() => {
                    let count = if projects.len() == 1 {
                        "1 project".to_owned()
                    } else {
                        format!("{} projects", projects.len())
                    };
                    div()
                        .child(
                            div()
                                .pt(px(20.))
                                .pb(px(8.))
                                .text_size(px(12.))
                                .text_color(rgb(palette.muted))
                                .child(count),
                        )
                        .when(!self.grid_view, |content| {
                            content.child(
                                div()
                                    .px(px(14.))
                                    .py(px(10.))
                                    .flex()
                                    .border_b_1()
                                    .border_color(rgb(palette.border))
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.muted))
                                    .child(div().flex_1().child("NAME"))
                                    .child(div().w(px(210.)).child("CREATED")),
                            )
                        })
                        .child(
                            div()
                                .when(self.grid_view, |list| {
                                    list.grid().grid_cols(columns).gap(px(16.)).pt(px(16.))
                                })
                                .children(
                                    projects
                                        .iter()
                                        .map(|project| self.project_row(project, palette, cx)),
                                ),
                        )
                }
                state => {
                    let message = match state {
                        None => "Loading projects…".into(),
                        Some(Ok(_)) => "No projects found".into(),
                        Some(Err(error)) => format!("Couldn’t read projects: {error}"),
                    };
                    div().py_6().text_color(rgb(palette.muted)).child(message)
                }
            });
        page.child(content)
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
            .tab_index(1)
            .cursor_pointer()
            .rounded_md()
            .px_3()
            .py(px(6.))
            .focus(move |style| style.bg(rgb(palette.selection)))
            .hover(move |style| style.bg(rgb(palette.selection)))
            .when(self.grid_view == grid, |button| {
                button.bg(rgb(palette.surface)).shadow_sm()
            })
            .child(label)
            .on_click(cx.listener(move |library, _, _, cx| {
                library.grid_view = grid;
                cx.notify();
            }))
            .on_key_down(
                cx.listener(move |library, event: &gpui::KeyDownEvent, _, cx| {
                    if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                        library.grid_view = grid;
                        cx.notify();
                    }
                }),
            )
    }

    fn project_row(
        &self,
        project: &Project,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let keyboard_project = project.clone();
        let click_project = project.clone();
        let grid = self.grid_view;
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

        div()
            .id(SharedString::from(project.id.clone()))
            .track_focus(&self.project_focus[&project.id])
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
                    .flex_1()
                    .min_w(px(0.))
                    .items_center()
                    .gap(px(14.))
                    .child(
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
                            .child("TEX"),
                    )
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .when(!grid, |name| name.truncate())
                                    .font_weight(FontWeight(550.))
                                    .child(project.name.clone()),
                            )
                            .when(grid, |name| {
                                name.child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(rgb(palette.muted))
                                        .child("Open project →"),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .when(!grid, |date| date.w(px(188.)).flex_none())
                    .text_size(px(12.))
                    .text_color(rgb(palette.muted))
                    .children(date),
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
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let home = div()
            .id("home")
            .track_focus(&self.home_focus)
            .cursor_pointer()
            .flex()
            .items_center()
            .gap_2()
            .rounded_md()
            .px_2()
            .py_1()
            .focus(move |style| style.bg(rgb(palette.selection)))
            .hover(move |style| style.bg(rgb(palette.selection)))
            .child(img(self.logo.clone()).size(px(24.)))
            .child("Refract")
            .on_click(cx.listener(|library, _, window, cx| library.return_to_projects(window, cx)))
            .on_key_down(
                cx.listener(|library, event: &gpui::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                        library.return_to_projects(window, cx);
                    }
                }),
            );
        div()
            .flex()
            .size_full()
            .child(
                div()
                    .w(px(250.))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .bg(rgb(palette.sidebar))
                    .border_r_1()
                    .border_color(rgb(palette.border))
                    .child(home)
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Files"))
                    .child(self.file_tree(selection, palette, cx)),
            )
            .child(self.source_pane(selection, palette, cx))
    }

    fn file_tree(
        &self,
        selection: &Selection,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let tree = div()
            .id("file-tree")
            .flex_1()
            .min_h(px(0.))
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
        let indicator = match (is_folder, expanded) {
            (true, true) => "⌄",
            (true, false) => "›",
            (false, _) => "",
        };
        let row = div()
            .id(SharedString::from(format!("tree/{}", entry.path)))
            .tab_index(1)
            .cursor_pointer()
            .flex()
            .items_center()
            .gap_1()
            .rounded_md()
            .pl(px(6. + 14. * depth as f32))
            .pr_1()
            .py_1()
            .hover(move |style| style.bg(rgb(palette.selection)))
            .focus(move |style| style.text_color(rgb(palette.focus)))
            .when(
                selection.file.as_deref() == Some(entry.path.as_str()),
                |row| row.bg(rgb(palette.selection)),
            )
            .child(div().w(px(12.)).flex_none().child(indicator))
            .child(div().min_w(px(0.)).truncate().child(entry.name.clone()))
            .on_click(cx.listener(move |library, _, window, cx| {
                if is_folder {
                    library.toggle_folder(&path, cx);
                } else {
                    library.open_file(path.clone(), window, cx);
                }
            }))
            .on_key_down(
                cx.listener(move |library, event: &gpui::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                        if is_folder {
                            library.toggle_folder(&keyboard_path, cx);
                        } else {
                            library.open_file(keyboard_path.clone(), window, cx);
                        }
                    }
                }),
            );
        let mut node = div().child(row);
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
                                library.close_file(&close, window, cx)
                            }))
                            .on_key_down(cx.listener(
                                move |library, event: &gpui::KeyDownEvent, window, cx| {
                                    if event.keystroke.key == "enter"
                                        || event.keystroke.key == "space"
                                    {
                                        library.close_file(&close_by_key, window, cx);
                                    }
                                },
                            )),
                    )
            }))
    }

    fn source_pane(
        &self,
        selection: &Selection,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let pane = div()
            .flex_1()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .p_4()
            .gap_2()
            .child(
                div()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(selection.project.name.clone()),
            );
        let Some(path) = &selection.file else {
            return pane.child(
                div()
                    .text_color(rgb(palette.muted))
                    .child("Choose a file from the sidebar."),
            );
        };
        let pane = pane.child(self.file_tabs(selection, palette, cx));
        let file = div()
            .truncate()
            .text_color(rgb(palette.muted))
            .child(format!("{path} · Temporary draft"));
        let editor = selection
            .tabs
            .iter()
            .find(|tab| &tab.path == path)
            .and_then(|tab| tab.editor.as_ref());
        let preview = match editor {
            None => div().child("Loading source…").into_any_element(),
            Some(Err(error)) => div()
                .child(format!("Couldn’t open source: {error}"))
                .into_any_element(),
            Some(Ok(editor)) => div()
                .flex_1()
                .min_h(px(0.))
                .child(
                    Input::new(editor)
                        .h_full()
                        .tab_index(3)
                        .appearance(false)
                        .font_family(monospace_font())
                        .text_size(px(14.)),
                )
                .into_any_element(),
        };
        pane.child(file).child(preview)
    }
}

fn monospace_font() -> &'static str {
    if cfg!(target_os = "windows") {
        "Consolas"
    } else if cfg!(target_os = "macos") {
        "Menlo"
    } else {
        "monospace"
    }
}

fn projects_dir() -> Option<PathBuf> {
    if let Some(profile) = env::var_os("REFRACT_DATA_DIR").filter(|path| !path.is_empty()) {
        return Some(PathBuf::from(profile).join("projects"));
    }
    let config_dir = if cfg!(target_os = "windows") {
        env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    };
    config_dir.map(|dir| dir.join("Refract").join("projects"))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let projects_dir = projects_dir().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "Couldn’t locate the Refract profile. Set REFRACT_DATA_DIR to its folder.",
        )
    })?;
    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1000.), px(680.)), cx);
        let window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                window.set_window_title("Refract · Native Preview");
                let library = cx.new(|cx| Library::new(projects_dir, window, cx));
                cx.new(|cx| Root::new(library, window, cx))
            },
        );
        match window {
            Ok(_) => cx.activate(true),
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
    use gpui::TestAppContext;
    use std::fs;

    #[gpui::test]
    fn keeps_source_drafts_in_their_tabs_and_restores_library_focus(cx: &mut TestAppContext) {
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

        cx.update(gpui_component::init);
        let mut library = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| Library::new(projects, window, cx));
            library = Some(view.clone());
            Root::new(view, window, cx)
        });
        let library = library.unwrap();
        cx.refresh().unwrap();
        cx.simulate_keystrokes("tab tab enter");
        cx.update(|_, app| assert!(library.read(app).grid_view));
        cx.simulate_keystrokes("tab enter");
        cx.update(|_, app| {
            let selection = library.read(app).selection.as_ref().expect("open project");
            assert_eq!(selection.project.id, id);
            let editor = selection.tabs[0].editor.as_ref().unwrap().as_ref().unwrap();
            assert_eq!(editor.read(app).value().as_ref(), "A line of LaTeX\n");
        });
        cx.simulate_keystrokes("tab enter");
        cx.update(|_, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            assert!(selection.expanded_folders.contains("sections"));
        });
        cx.simulate_keystrokes("tab enter");
        let tree_focus = cx.update(|window, app| {
            let selection = library.read(app).selection.as_ref().unwrap();
            assert_eq!(selection.file.as_deref(), Some("sections/abstract.tex"));
            assert_eq!(selection.tabs.len(), 2);
            let editor = selection.tabs[1]
                .editor
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .clone();
            assert_eq!(editor.read(app).value().as_ref(), "An abstract\n");
            let tree_focus = window.focused(app).unwrap();
            editor.update(app, |editor, cx| editor.focus(window, cx));
            tree_focus
        });
        cx.simulate_keystrokes("end enter");
        cx.simulate_input("A draft");
        cx.update(|window, _| window.focus(&tree_focus));
        cx.simulate_keystrokes("tab tab enter");
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
            assert_eq!(editor.read(app).value().as_ref(), "An abstract\nA draft\n");
        });
        cx.simulate_keystrokes("tab enter");
        cx.update(|window, app| {
            let library = library.read(app);
            let selection = library.selection.as_ref().unwrap();
            assert!(selection.file.is_none());
            assert!(selection.tabs.is_empty());
            assert!(library.home_focus.is_focused(window));
        });
        cx.simulate_keystrokes("enter");
        cx.update(|window, app| {
            let library = library.read(app);
            assert!(library.selection.is_none());
            assert!(library.grid_view);
            assert!(library.project_focus[id].is_focused(window));
        });
        assert_eq!(
            fs::read_to_string(project.join("sections/abstract.tex")).unwrap(),
            "An abstract\n"
        );
    }
}
