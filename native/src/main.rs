use gpui::{
    App, Application, Bounds, Context, FocusHandle, KeyBinding, ListHorizontalSizingBehavior,
    SharedString, Task, Window, WindowAppearance, WindowBounds, WindowOptions, actions, div,
    prelude::*, px, rgb, size, uniform_list,
};
use refract_native::{Project, list_projects, read_source};
use std::{env, io, path::PathBuf, rc::Rc};

actions!(library, [NextProject, PreviousProject]);

struct Library {
    projects_dir: PathBuf,
    projects: Option<io::Result<Vec<Project>>>,
    selection: Option<Selection>,
    preview_task: Option<Task<()>>,
    focus_handle: FocusHandle,
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
        }
    }

    fn open_project(&mut self, project: Project, cx: &mut Context<Self>) {
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
        cx.notify();
    }
}

#[derive(Clone, Copy)]
struct Palette {
    canvas: u32,
    sidebar: u32,
    text: u32,
    muted: u32,
    border: u32,
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
                text: 0xf5f5f5,
                muted: 0xa3a3a3,
                border: 0x191919,
                selection: 0x1a1b1b,
                focus: 0x79a8ff,
            },
            _ => Self {
                canvas: 0xfcfcfc,
                sidebar: 0xfafafa,
                text: 0x27272a,
                muted: 0x71717b,
                border: 0xe4e4e7,
                selection: 0xf0f0f2,
                focus: 0x1b4ed8,
            },
        }
    }
}

impl Render for Library {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_window(window);
        let mut projects = div()
            .id("projects")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll();
        match &self.projects {
            Some(Ok(library)) if !library.is_empty() => {
                projects = projects.children(library.iter().map(|project| {
                    let selected = self
                        .selection
                        .as_ref()
                        .is_some_and(|item| item.project.id == project.id);
                    let keyboard_project = project.clone();
                    let click_project = project.clone();
                    div()
                        .id(SharedString::from(project.id.clone()))
                        .tab_index(0)
                        .cursor_pointer()
                        .border_1()
                        .border_color(rgb(palette.sidebar))
                        .focus(move |style| style.border_color(rgb(palette.focus)))
                        .hover(move |style| style.bg(rgb(palette.selection)))
                        .when(selected, |row| row.bg(rgb(palette.selection)))
                        .rounded_md()
                        .px_2()
                        .py_1()
                        .child(div().truncate().child(project.name.clone()))
                        .on_click(cx.listener(move |library, _, _, cx| {
                            library.open_project(click_project.clone(), cx);
                        }))
                        .on_key_down(cx.listener(
                            move |library, event: &gpui::KeyDownEvent, _, cx| {
                                if event.keystroke.key == "enter" {
                                    library.open_project(keyboard_project.clone(), cx);
                                }
                            },
                        ))
                }));
            }
            state => {
                let message = match state {
                    None => "Loading projects…".into(),
                    Some(Ok(_)) => "No projects found".into(),
                    Some(Err(error)) => format!("Couldn’t read projects: {error}"),
                };
                projects = projects.child(div().text_color(rgb(palette.muted)).child(message));
            }
        }

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
                    .child(div().font_semibold().child("Projects"))
                    .child(projects),
            )
            .child(self.source_pane(palette))
    }
}

impl Library {
    fn source_pane(&self, palette: Palette) -> impl IntoElement {
        let pane = div().flex_1().min_w(px(0.)).flex().flex_col().p_4().gap_2();
        let Some(selection) = &self.selection else {
            return pane.child(
                div()
                    .text_color(rgb(palette.muted))
                    .child("Open a project to view its LaTeX source."),
            );
        };
        let title = div()
            .truncate()
            .font_semibold()
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let projects_dir = projects_dir().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "Couldn’t locate the Refract profile. Set REFRACT_DATA_DIR to its folder.",
        )
    })?;
    Application::new().run(move |cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("tab", NextProject, None),
            KeyBinding::new("shift-tab", PreviousProject, None),
        ]);
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
