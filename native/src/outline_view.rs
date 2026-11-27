use super::{Library, Palette, Selection};
use gpui::{
    Context, DragMoveEvent, Entity, EntityId, FontWeight, KeyDownEvent, MouseButton,
    MouseDownEvent, Pixels, Render, RenderImage, ScrollHandle, SharedString, Task, Window, div,
    img, point, prelude::*, px, rgb, svg,
};
use gpui_component::input::{InputState, RopeExt};
use image::{Frame, RgbaImage};
use katex_native::{Color, katex::KatexContext, layout, raster::rasterize};
use refract_native::outline::{TitlePart, sections, title_parts};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const EDIT_DELAY: Duration = Duration::from_millis(75);
const MIN_OUTLINE_HEIGHT: Pixels = px(32.);
const MIN_FILE_TREE_HEIGHT: Pixels = px(48.);
const MATH_FONT_SIZE: f64 = 12.0 * 1.21;
const RASTER_SCALE: f64 = 2.0;

pub(super) struct OutlinePane {
    expanded: bool,
    height: Option<Pixels>,
    preferences: PathBuf,
    editor: Option<EntityId>,
    rows: Vec<OutlineRow>,
    rows_current: bool,
    request: Option<OutlineRequest>,
    resize: Option<(Pixels, Pixels)>,
    scroll: ScrollHandle,
    pub(super) file_scroll: ScrollHandle,
}

struct OutlineRequest {
    cancel: Arc<AtomicBool>,
    _task: Task<()>,
}

impl Drop for OutlineRequest {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

struct OutlineRow {
    offset: usize,
    title: String,
    label: Arc<[OutlineLabel]>,
}

enum OutlineLabel {
    Text(SharedString),
    Formula { light: Formula, dark: Formula },
}

impl OutlineRow {
    fn images(&self) -> impl Iterator<Item = &Arc<RenderImage>> {
        self.label
            .iter()
            .filter_map(|part| match part {
                OutlineLabel::Text(_) => None,
                OutlineLabel::Formula { light, dark } => Some([&light.image, &dark.image]),
            })
            .flatten()
    }
}

struct Formula {
    image: Arc<RenderImage>,
    width: Pixels,
    height: Pixels,
    depth: Pixels,
}

struct OutlineDrag;

impl Render for OutlineDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size(px(0.))
    }
}

impl OutlinePane {
    pub(super) fn new(projects: &Path) -> Self {
        let preferences = projects.with_file_name(".refract-native-outline-height");
        Self {
            expanded: true,
            height: read_height(&preferences),
            preferences,
            editor: None,
            rows: Vec::new(),
            rows_current: false,
            request: None,
            resize: None,
            scroll: ScrollHandle::new(),
            file_scroll: ScrollHandle::new(),
        }
    }

    pub(super) fn clear(&mut self, window: &mut Window) {
        self.request = None;
        self.editor = None;
        self.replace_rows(Vec::new(), window);
        self.rows_current = false;
        self.scroll.set_offset(point(px(0.), px(0.)));
    }

    pub(super) fn reload_height(&mut self) {
        self.height = read_height(&self.preferences);
    }

    fn replace_rows(&mut self, rows: Vec<OutlineRow>, window: &mut Window) {
        let current_images: HashSet<_> = rows
            .iter()
            .flat_map(OutlineRow::images)
            .map(|image| image.id)
            .collect();
        let mut dropped = HashSet::new();
        for image in self.rows.iter().flat_map(OutlineRow::images) {
            if !current_images.contains(&image.id) && dropped.insert(image.id) {
                let _ = window.drop_image(image.clone());
            }
        }
        self.rows = rows;
    }
}

impl Library {
    pub(super) fn refresh_outline(
        &mut self,
        editor: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_editor().as_ref() != Some(editor) {
            return;
        }
        let id = editor.entity_id();
        let changed_file = self.outline.editor != Some(id);
        if changed_file {
            self.outline.clear(window);
            self.outline.editor = Some(id);
        }
        self.outline.request = None;
        self.outline.rows_current = false;
        let source = editor.read(cx).value();
        let existing = self
            .outline
            .rows
            .iter()
            .map(|row| (row.title.clone(), row.label.clone()))
            .collect();
        let executor = cx.background_executor().clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker = cancel.clone();
        let task = cx.spawn_in(window, async move |library, cx| {
            if !changed_file {
                executor.timer(EDIT_DELAY).await;
            }
            let rows = executor
                .spawn(async move { outline_rows(source.as_ref(), existing, &worker) })
                .await;
            let _ = library.update_in(cx, |library, window, cx| {
                if library.outline.editor == Some(id) {
                    library.outline.replace_rows(rows, window);
                    library.outline.rows_current = true;
                    cx.notify();
                }
            });
        });
        self.outline.request = Some(OutlineRequest {
            cancel,
            _task: task,
        });
    }

    fn select_outline_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else {
            return;
        };
        if self.outline.editor != Some(editor.entity_id()) {
            return;
        }
        let offset = if self.outline.rows_current {
            self.outline.rows.get(index).map(|row| row.offset)
        } else {
            // A heading can move while the latest edit is waiting for the outline refresh.
            let source = editor.read(cx).value();
            sections(source.as_ref())
                .get(index)
                .map(|section| section.offset)
        };
        let Some(offset) = offset else {
            return;
        };
        self.reveal_workspace_pane(refract_native::workspace_layout::Pane::Editor, window, cx);
        let position = editor.read(cx).text().offset_to_position(offset);
        editor.update(cx, |editor, cx| {
            editor.set_cursor_position(position, window, cx)
        });
        if self.pdf_is_visible(window)
            && self
                .selection
                .as_ref()
                .is_some_and(|selection| selection.build.pdf().is_some())
        {
            self.show_source_in_pdf(window, cx);
        }
    }

    fn resize_outline(&mut self, target: Pixels, cx: &mut Context<Self>) {
        let outline = &mut self.outline;
        let maximum = (outline.scroll.bounds().size.height
            + outline.file_scroll.bounds().size.height
            - MIN_FILE_TREE_HEIGHT)
            .max(MIN_OUTLINE_HEIGHT);
        outline.height = Some(target.clamp(MIN_OUTLINE_HEIGHT, maximum));
        cx.notify();
    }

    fn finish_outline_resize(&mut self) {
        if let Some(height) = self.outline.height
            && let Err(error) = write_height(&self.outline.preferences, height)
        {
            eprintln!("Couldn’t save the outline height: {error}");
        }
    }

    pub(super) fn outline_pane(
        &self,
        selection: &Selection,
        palette: Palette,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let outline = &self.outline;
        let expanded = outline.expanded;
        let mode = if palette.canvas == 0x0a0a0a {
            Theme::Dark
        } else {
            Theme::Light
        };
        div()
            .id("outline")
            .relative()
            .flex()
            .flex_col()
            .min_h(px(0.))
            .border_t_1()
            .border_color(rgb(palette.border))
            .mt_1()
            .pt_2()
            .text_color(rgb(palette.muted))
            .text_size(px(12.))
            .child(
                div()
                    .id("outline-toggle")
                    .debug_selector(|| "outline-toggle".into())
                    .tab_index(1)
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .p_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .font_weight(FontWeight::SEMIBOLD)
                    .hover(move |style| style.bg(rgb(palette.selection)))
                    .focus(move |style| style.bg(rgb(palette.selection)))
                    .child(
                        svg()
                            .path(if expanded {
                                "refract/chevron-down.svg"
                            } else {
                                "refract/chevron-right.svg"
                            })
                            .size(px(14.))
                            .text_color(rgb(palette.muted)),
                    )
                    .child("OUTLINE")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.outline.expanded = !view.outline.expanded;
                        cx.notify();
                    }))
                    .on_key_down(cx.listener(|view, event: &KeyDownEvent, _, cx| {
                        if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                            view.outline.expanded = !view.outline.expanded;
                            cx.notify();
                        }
                    })),
            )
            .when(expanded, |panel| {
                panel.child(self.outline_divider(palette, cx)).child(
                    div()
                        .id("outline-list")
                        .debug_selector(|| "outline-list".into())
                        .track_scroll(&outline.scroll)
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        .when_some(outline.height, |list, height| list.h(height))
                        .when(selection.file.is_some(), |list| {
                            list.children(
                                outline
                                    .rows
                                    .iter()
                                    .enumerate()
                                    .map(|(index, row)| outline_row(row, index, mode, palette, cx)),
                            )
                        }),
                )
            })
    }

    fn outline_divider(&self, palette: Palette, cx: &Context<Self>) -> impl IntoElement {
        div()
            .id("outline-resize")
            .debug_selector(|| "outline-resize".into())
            .absolute()
            .top(px(-5.))
            .left_0()
            .right_0()
            .h(px(9.))
            .tab_index(1)
            .cursor_row_resize()
            .border_b_1()
            .border_color(rgb(palette.border))
            .hover(move |style| style.border_color(rgb(palette.focus)))
            .focus(move |style| style.border_color(rgb(palette.focus)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &MouseDownEvent, _, _| {
                    view.outline.resize =
                        Some((event.position.y, view.outline.scroll.bounds().size.height));
                }),
            )
            .on_drag(OutlineDrag, |_, _, _, cx| cx.new(|_| OutlineDrag))
            .on_drag_move(
                cx.listener(|view, event: &DragMoveEvent<OutlineDrag>, _, cx| {
                    if let Some((y, height)) = view.outline.resize {
                        view.resize_outline(height + y - event.event.position.y, cx);
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, _| {
                    if view.outline.resize.take().is_some() {
                        view.finish_outline_resize();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _, _, _| {
                    if view.outline.resize.take().is_some() {
                        view.finish_outline_resize();
                    }
                }),
            )
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, _, cx| {
                let delta = match event.keystroke.key.as_str() {
                    "up" => px(10.),
                    "down" => px(-10.),
                    _ => return,
                };
                let height = view.outline.scroll.bounds().size.height;
                view.resize_outline(height + delta, cx);
                view.finish_outline_resize();
                cx.stop_propagation();
            }))
    }
}

#[derive(Clone, Copy)]
enum Theme {
    Light,
    Dark,
}

fn outline_row(
    row: &OutlineRow,
    index: usize,
    theme: Theme,
    palette: Palette,
    cx: &Context<Library>,
) -> impl IntoElement {
    div()
        .id(("outline-row", index))
        .debug_selector(|| format!("outline-row-{index}"))
        .tab_index(1)
        .w_full()
        .rounded_sm()
        .px_5()
        .py(px(6.))
        .cursor_pointer()
        .flex()
        .flex_wrap()
        .items_baseline()
        .hover(move |style| style.bg(rgb(palette.selection)))
        .focus(move |style| style.bg(rgb(palette.selection)))
        .children(row.label.iter().map(|part| {
            match part {
                OutlineLabel::Text(text) => div()
                    .min_w(px(0.))
                    .whitespace_normal()
                    .child(text.clone())
                    .into_any_element(),
                OutlineLabel::Formula { light, dark } => {
                    let formula = match theme {
                        Theme::Light => light,
                        Theme::Dark => dark,
                    };
                    img(formula.image.clone())
                        .w(formula.width)
                        .h(formula.height)
                        .relative()
                        .top(formula.depth)
                        .into_any_element()
                }
            }
        }))
        .on_click(
            cx.listener(move |view, _, window, cx| view.select_outline_row(index, window, cx)),
        )
        .on_key_down(cx.listener(move |view, event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                view.select_outline_row(index, window, cx);
            }
        }))
}

fn outline_rows(
    source: &str,
    mut existing: HashMap<String, Arc<[OutlineLabel]>>,
    cancel: &AtomicBool,
) -> Vec<OutlineRow> {
    let mut math = None;
    sections(source)
        .into_iter()
        .take_while(|_| !cancel.load(Ordering::Relaxed))
        .map(|section| {
            let label = existing
                .entry(section.title.to_owned())
                .or_insert_with(|| {
                    title_parts(section.title)
                        .into_iter()
                        .map(|part| match part {
                            TitlePart::Text(text) => OutlineLabel::Text(text.to_owned().into()),
                            TitlePart::Math { formula, source } => render_formula(
                                math.get_or_insert_with(KatexContext::default),
                                formula,
                            )
                            .unwrap_or_else(|| OutlineLabel::Text(source.to_owned().into())),
                        })
                        .collect::<Vec<_>>()
                        .into()
                })
                .clone();
            OutlineRow {
                offset: section.offset,
                title: section.title.to_owned(),
                label,
            }
        })
        .collect()
}

fn render_formula(context: &KatexContext, formula: &str) -> Option<OutlineLabel> {
    let layout = layout(context, formula, false).ok()?;
    let font = MATH_FONT_SIZE * RASTER_SCALE;
    let width = layout.width * font;
    let height = (layout.height + layout.depth) * font;
    if !(0.0..=4096.0).contains(&width) || !(0.0..=1024.0).contains(&height) {
        return None;
    }
    let raster = |r, g, b| {
        let mut pixels = rasterize(&layout, font, Color { r, g, b, a: 255 })?;
        for pixel in pixels.rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let buffer = RgbaImage::from_raw(pixels.width, pixels.height, pixels.rgba)?;
        Some(Formula {
            image: Arc::new(RenderImage::new([Frame::new(buffer)])),
            width: px(pixels.width as f32 / RASTER_SCALE as f32),
            height: px(pixels.height as f32 / RASTER_SCALE as f32),
            depth: px(layout.depth as f32 * MATH_FONT_SIZE as f32),
        })
    };
    Some(OutlineLabel::Formula {
        light: raster(0x71, 0x71, 0x7b)?,
        dark: raster(0xa3, 0xa3, 0xa3)?,
    })
}

fn read_height(path: &Path) -> Option<Pixels> {
    let mut value = String::new();
    File::open(path)
        .ok()?
        .take(64)
        .read_to_string(&mut value)
        .ok()?;
    let height = value.trim().parse::<f32>().ok()?;
    (height.is_finite() && height >= f32::from(MIN_OUTLINE_HEIGHT)).then(|| px(height))
}

fn write_height(path: &Path, height: Pixels) -> std::io::Result<()> {
    let directory = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(directory)?;
    let mut stage = tempfile::NamedTempFile::new_in(directory)?;
    writeln!(stage, "{}", f32::from(height))?;
    stage.persist(path).map(|_| ()).map_err(|error| error.error)
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::{Root, initialize_app};
    use gpui::{Modifiers, TestAppContext};
    use gpui_component::input::Position;
    use refract_native::create_project;

    #[gpui::test]
    fn outline_renders_math_and_follows_new_headings(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
        let source =
            "\\section{Introduction}\n🙂 \\subsection*{$J_{2}$ Truth, \\(\\Delta t=1800\\) s}\n";
        fs::write(projects.join(&project.id).join("main.tex"), source).unwrap();
        cx.update(initialize_app);
        let mut view = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let library = cx.new(|cx| Library::new(projects, window, cx));
            view = Some(library.clone());
            Root::new(library, window, cx)
        });
        let library = view.unwrap();
        cx.update(|window, app| {
            library.update(app, |view, cx| view.open_project(project, window, cx));
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        cx.update(|_, app| {
            let rows = &library.read(app).outline.rows;
            assert_eq!(rows.len(), 2);
            assert_eq!(
                rows[1]
                    .label
                    .iter()
                    .filter(|part| matches!(part, OutlineLabel::Formula { .. }))
                    .count(),
                2
            );
        });
        let row = cx.debug_bounds("outline-row-1").unwrap();
        cx.simulate_click(row.center(), Modifiers::none());
        cx.update(|window, app| {
            let editor = library.read(app).active_editor().unwrap();
            assert_eq!(editor.read(app).cursor_position(), Position::new(1, 2));
            editor.update(app, |editor, cx| {
                editor.set_value(format!("{source}\\section{{Conclusion}}\n"), window, cx);
            });
        });
        cx.executor().advance_clock(EDIT_DELAY);
        cx.run_until_parked();
        cx.refresh().unwrap();
        assert!(cx.debug_bounds("outline-row-2").is_some());
    }

    #[gpui::test]
    fn outline_divider_resizes_and_remembers_its_height(cx: &mut TestAppContext) {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
        let source = (1..=40)
            .map(|index| format!("\\section{{Section {index}}}\n"))
            .collect::<String>();
        fs::write(projects.join(&project.id).join("main.tex"), source).unwrap();
        cx.update(initialize_app);
        let mut view = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let library = cx.new(|cx| Library::new(projects.clone(), window, cx));
            view = Some(library.clone());
            Root::new(library, window, cx)
        });
        let library = view.unwrap();
        cx.update(|window, app| {
            library.update(app, |view, cx| view.open_project(project, window, cx));
        });
        cx.run_until_parked();
        cx.refresh().unwrap();
        let divider = cx.debug_bounds("outline-resize").unwrap().center();
        let initial = cx.debug_bounds("outline-list").unwrap().size.height;
        let down = |offset| point(divider.x, divider.y + px(offset));
        cx.simulate_mouse_down(divider, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(down(20.), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(down(100.), MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(down(100.), MouseButton::Left, Modifiers::none());
        cx.refresh().unwrap();
        let shorter = cx.debug_bounds("outline-list").unwrap().size.height;
        assert!(shorter <= initial - px(90.));
        assert_eq!(OutlinePane::new(&projects).height, Some(shorter));
        let divider = cx.debug_bounds("outline-resize").unwrap().center();
        cx.simulate_click(divider, Modifiers::none());
        cx.simulate_keystrokes("up");
        cx.refresh().unwrap();
        let taller = cx.debug_bounds("outline-list").unwrap().size.height;
        assert_eq!(taller, shorter + px(10.));
        let toggle = cx.debug_bounds("outline-toggle").unwrap().center();
        cx.simulate_click(toggle, Modifiers::none());
        cx.refresh().unwrap();
        cx.update(|_, app| assert!(!library.read(app).outline.expanded));
        let toggle = cx.debug_bounds("outline-toggle").unwrap().center();
        cx.simulate_click(toggle, Modifiers::none());
        cx.refresh().unwrap();
        cx.update(|_, app| assert!(library.read(app).outline.expanded));
        assert_eq!(cx.debug_bounds("outline-list").unwrap().size.height, taller);
    }
}
