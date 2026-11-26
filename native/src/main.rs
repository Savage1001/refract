use gpui::{
    App, Application, Bounds, Context, FocusHandle, FontWeight, Image, ImageFormat, KeyBinding,
    ListHorizontalSizingBehavior, SharedString, Task, Window, WindowAppearance, WindowBounds,
    WindowOptions, actions, div, img, prelude::*, px, rgb, size, uniform_list,
};
use refract_native::{Project, list_projects, read_source};
use std::{collections::HashMap, env, io, path::PathBuf, rc::Rc, sync::Arc};

actions!(library, [NextProject, PreviousProject]);

struct Library {
    projects_dir: PathBuf,
    projects: Option<io::Result<Vec<Project>>>,
    selection: Option<Selection>,
    preview_task: Option<Task<()>>,
    focus_handle: FocusHandle,
    project_focus: HashMap<String, FocusHandle>,
    home_focus: FocusHandle,
    logo: Arc<Image>,
    grid_view: bool,
}

struct Selection {
    project: Project,
    preview: Option<io::Result<SourcePreview>>,
}

struct SourcePreview {
    lines: Rc<[SharedString]>,
    widest_line: usize,
}

impl SourcePreview {
    fn new(source: &str) -> Self {
        let lines: Rc<[SharedString]> = source
            .lines()
            .map(|line| SharedString::from(line.replace('\t', "    ")))
            .collect();
        let widest_line = lines
            .iter()
            .enumerate()
            .max_by_key(|(_, line)| line.len())
            .map_or(0, |(index, _)| index);
        Self { lines, widest_line }
    }
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
        self.selection = Some(Selection {
            project: project.clone(),
            preview: None,
        });
        let path = self.projects_dir.clone();
        let load = cx
            .background_executor()
            .spawn(async move { read_source(&path, &project) });
        // Replacing the task cancels the previous UI update if projects are opened in quick succession.
        self.preview_task = Some(cx.spawn(async move |library, cx| {
            let preview = load.await.map(|source| SourcePreview::new(&source));
            let _ = library.update(cx, |library, cx| {
                if let Some(selection) = &mut library.selection {
                    selection.preview = Some(preview);
                    cx.notify();
                }
            });
        }));
        window.focus(&self.home_focus);
        cx.notify();
    }

    fn return_to_projects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.preview_task = None;
        if let Some(previous) = self.selection.take()
            && let Some(focus) = self.project_focus.get(&previous.project.id)
        {
            window.focus(focus);
        }
        cx.notify();
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
            .on_action(|_: &NextProject, window, _| window.focus_next())
            .on_action(|_: &PreviousProject, window, _| window.focus_prev())
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
                    .child(
                        div()
                            .truncate()
                            .text_color(rgb(palette.muted))
                            .child(selection.project.root_file().to_owned()),
                    ),
            )
            .child(self.source_pane(selection, palette))
    }

    fn source_pane(&self, selection: &Selection, palette: Palette) -> impl IntoElement {
        let pane = div().flex_1().min_w(px(0.)).flex().flex_col().p_4().gap_2();
        let title = div()
            .truncate()
            .font_weight(FontWeight::SEMIBOLD)
            .child(selection.project.name.clone());
        let file = div()
            .truncate()
            .text_color(rgb(palette.muted))
            .child(format!("{} · Read only", selection.project.root_file()));
        let preview = match &selection.preview {
            None => div().child("Loading source…").into_any_element(),
            Some(Err(error)) => div()
                .child(format!("Couldn’t open source: {error}"))
                .into_any_element(),
            Some(Ok(source)) if source.lines.is_empty() => {
                div().child("This file is empty.").into_any_element()
            }
            Some(Ok(source)) => {
                let lines = source.lines.clone();
                uniform_list(
                    SharedString::from(format!("source-{}", selection.project.id)),
                    lines.len(),
                    move |range, _, _| {
                        range
                            .map(|index| {
                                div()
                                    .h(px(22.))
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .whitespace_nowrap()
                                    .child(
                                        div()
                                            .w(px(44.))
                                            .text_right()
                                            .text_color(rgb(palette.muted))
                                            .child((index + 1).to_string()),
                                    )
                                    .child(lines[index].clone())
                            })
                            .collect::<Vec<_>>()
                    },
                )
                .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
                .with_width_from_item(Some(source.widest_line))
                .flex_1()
                .min_h(px(0.))
                .font_family(monospace_font())
                .into_any_element()
            }
        };
        pane.child(title).child(file).child(preview)
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

fn bind_library_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", NextProject, None),
        KeyBinding::new("shift-tab", PreviousProject, None),
    ]);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let projects_dir = projects_dir().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "Couldn’t locate the Refract profile. Set REFRACT_DATA_DIR to its folder.",
        )
    })?;
    Application::new().run(move |cx: &mut App| {
        bind_library_keys(cx);
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
                cx.new(|cx| Library::new(projects_dir, window, cx))
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
    fn opens_an_existing_project_and_restores_library_focus(cx: &mut TestAppContext) {
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

        cx.update(bind_library_keys);
        let (library, cx) = cx.add_window_view(|window, cx| Library::new(projects, window, cx));
        cx.refresh().unwrap();
        cx.simulate_keystrokes("tab tab enter");
        cx.update(|_, app| assert!(library.read(app).grid_view));
        cx.simulate_keystrokes("tab enter");
        cx.update(|_, app| {
            let selection = library.read(app).selection.as_ref().expect("open project");
            assert_eq!(selection.project.id, id);
            let preview = selection.preview.as_ref().unwrap().as_ref().unwrap();
            assert_eq!(preview.lines[0].as_ref(), "A line of LaTeX");
        });
        cx.simulate_keystrokes("enter");
        cx.update(|window, app| {
            let library = library.read(app);
            assert!(library.selection.is_none());
            assert!(library.grid_view);
            assert!(library.project_focus[id].is_focused(window));
        });
    }
}
