use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::Path,
};

const COMPACT_WIDTH: f32 = 1200.;
const MIN_EDITOR_WIDTH: f32 = 300.;
const PDF_CLEARANCE: f32 = 292.;
pub const WORKSPACE_HEADER_HEIGHT: f32 = 42.;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pane {
    Agent,
    Editor,
    Preview,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Divider {
    Sidebar,
    Editor,
    Chat,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct PaneVisibility {
    pub agent: bool,
    pub editor: bool,
    pub preview: bool,
}

impl Default for PaneVisibility {
    fn default() -> Self {
        Self {
            agent: true,
            editor: true,
            preview: true,
        }
    }
}

impl PaneVisibility {
    fn set(&mut self, pane: Pane, visible: bool) {
        match pane {
            Pane::Agent => self.agent = visible,
            Pane::Editor => self.editor = visible,
            Pane::Preview => self.preview = visible,
        }
    }

    pub fn contains(self, pane: Pane) -> bool {
        match pane {
            Pane::Agent => self.agent,
            Pane::Editor => self.editor,
            Pane::Preview => self.preview,
        }
    }

    fn restore_editor_if_empty(&mut self) {
        if !self.agent && !self.editor && !self.preview {
            self.editor = true;
        }
    }

    pub fn read(path: &Path) -> Self {
        let mut panes: Self = File::open(path)
            .ok()
            .and_then(|file| serde_json::from_reader(file.take(1024)).ok())
            .unwrap_or_default();
        panes.restore_editor_if_empty();
        panes
    }

    pub fn save(self, path: &Path) -> io::Result<()> {
        let root = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(root)?;
        let mut stage = tempfile::NamedTempFile::new_in(root)?;
        serde_json::to_writer(stage.as_file_mut(), &self)?;
        stage.persist(path).map(|_| ()).map_err(|error| error.error)
    }
}

#[derive(Default)]
pub struct WorkspaceLayout {
    pub panes: PaneVisibility,
    pub sidebar_collapsed: bool,
    compact_preview: bool,
    sidebar_width: Option<f32>,
    editor_width: Option<f32>,
    chat_height: Option<f32>,
}

impl WorkspaceLayout {
    pub fn new(panes: PaneVisibility) -> Self {
        Self {
            panes,
            ..Self::default()
        }
    }

    pub fn compact(width: f32) -> bool {
        width <= COMPACT_WIDTH
    }

    pub fn visible(&self, width: f32) -> PaneVisibility {
        if !Self::compact(width) {
            self.panes
        } else if self.compact_preview {
            PaneVisibility {
                agent: false,
                editor: false,
                preview: true,
            }
        } else {
            PaneVisibility {
                preview: self.panes.preview && !self.panes.editor && !self.panes.agent,
                ..self.panes
            }
        }
    }

    pub fn toggle(&mut self, pane: Pane, width: f32, displayed: PaneVisibility) {
        if Self::compact(width)
            && (self.compact_preview
                || (pane == Pane::Preview && (self.panes.editor || self.panes.agent)))
        {
            self.compact_preview = pane == Pane::Preview && !displayed.preview;
            if pane != Pane::Preview {
                self.panes.set(pane, true);
            }
        } else {
            self.panes.set(pane, !displayed.contains(pane));
            self.panes.restore_editor_if_empty();
        }
    }

    pub fn reveal(&mut self, pane: Pane, width: f32) {
        match pane {
            Pane::Preview if Self::compact(width) => self.compact_preview = true,
            Pane::Preview => self.panes.preview = true,
            Pane::Agent | Pane::Editor => {
                self.compact_preview = false;
                self.panes.set(pane, true);
            }
        }
    }

    pub fn expand_chat(&mut self, expanded: bool) {
        self.panes.agent = true;
        self.panes.editor = !expanded;
    }

    pub fn sidebar_width(&self, viewport: f32) -> f32 {
        if self.sidebar_collapsed {
            return 56.;
        }
        let default = if viewport <= 800. {
            180.
        } else if Self::compact(viewport) {
            210.
        } else {
            232.
        };
        self.sidebar_width
            .unwrap_or(default)
            .clamp(150., (viewport * 0.4).floor().max(150.))
    }

    pub fn editor_width(&self, viewport: f32) -> f32 {
        let width = viewport - self.sidebar_width(viewport);
        self.editor_width.unwrap_or(width * 0.5).clamp(
            MIN_EDITOR_WIDTH,
            (width - PDF_CLEARANCE).max(MIN_EDITOR_WIDTH),
        )
    }

    pub fn chat_height(&self, width: f32, height: f32, has_messages: bool) -> f32 {
        let minimum = chat_min_height(height);
        let fraction = if Self::compact(width) { 0.55 } else { 0.65 };
        let maximum = ((height - WORKSPACE_HEADER_HEIGHT) * fraction).max(minimum);
        let default = if has_messages {
            380.
        } else if height <= 720. {
            235.
        } else if Self::compact(width) {
            260.
        } else {
            280.
        };
        self.chat_height.unwrap_or(default).clamp(minimum, maximum)
    }

    pub fn resize(&mut self, divider: Divider, length: f32, viewport: f32) {
        match divider {
            Divider::Sidebar => {
                let maximum = (viewport * 0.4).floor().max(150.);
                self.sidebar_width = Some(length.clamp(150., maximum));
            }
            Divider::Editor => {
                let maximum =
                    (viewport - self.sidebar_width(viewport) - PDF_CLEARANCE).max(MIN_EDITOR_WIDTH);
                self.editor_width = Some(length.clamp(MIN_EDITOR_WIDTH, maximum));
            }
            Divider::Chat => {
                let minimum = chat_min_height(viewport);
                let maximum = (viewport - WORKSPACE_HEADER_HEIGHT - 100.).max(minimum);
                self.chat_height = Some(length.clamp(minimum, maximum));
            }
        }
    }
}

fn chat_min_height(viewport_height: f32) -> f32 {
    if viewport_height <= 720. { 190. } else { 220. }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_wide_workspaces_and_clamps_both_dividers() {
        let mut layout = WorkspaceLayout::default();
        assert_eq!(layout.sidebar_width(1440.), 232.);
        assert_eq!(layout.editor_width(1440.), 604.);
        layout.resize(Divider::Sidebar, 900., 1440.);
        assert_eq!(layout.sidebar_width(1440.), 576.);
        layout.resize(Divider::Editor, 850., 1440.);
        assert_eq!(layout.editor_width(1440.), 572.);
        layout.resize(Divider::Editor, 1., 1440.);
        assert_eq!(layout.editor_width(1440.), 300.);
        layout.sidebar_collapsed = true;
        assert_eq!(layout.sidebar_width(1440.), 56.);
    }

    #[test]
    fn chat_height_matches_desktop_and_compact_breakpoints_until_resized() {
        let mut layout = WorkspaceLayout::default();
        assert_eq!(layout.chat_height(1440., 900., false), 280.);
        assert_eq!(layout.chat_height(1200., 900., false), 260.);
        assert_eq!(layout.chat_height(900., 650., false), 235.);
        assert_eq!(layout.chat_height(1440., 900., true), 380.);
        assert!((layout.chat_height(900., 650., true) - 334.4).abs() < 0.001);
        layout.resize(Divider::Chat, 440., 900.);
        assert_eq!(layout.chat_height(1440., 900., true), 440.);
        assert!((layout.chat_height(900., 650., true) - 334.4).abs() < 0.001);
        layout.resize(Divider::Chat, 40., 650.);
        assert_eq!(layout.chat_height(900., 650., false), 190.);
    }

    #[test]
    fn compact_preview_switches_without_overwriting_the_desktop_choices() {
        let mut layout = WorkspaceLayout::default();
        let editor = PaneVisibility {
            agent: true,
            editor: true,
            preview: false,
        };
        let preview = PaneVisibility {
            agent: false,
            editor: false,
            preview: true,
        };
        assert_eq!(layout.visible(1200.), editor);
        layout.toggle(Pane::Preview, 1200., editor);
        assert_eq!(layout.visible(1200.), preview);
        assert_eq!(layout.visible(1201.), PaneVisibility::default());
        layout.toggle(Pane::Preview, 1200., preview);
        assert_eq!(layout.visible(1200.), editor);
        layout.toggle(Pane::Preview, 1440., PaneVisibility::default());
        layout.toggle(Pane::Agent, 1440., editor);
        layout.toggle(Pane::Editor, 1440., layout.visible(1440.));
        let editor = PaneVisibility {
            agent: false,
            ..editor
        };
        assert_eq!(layout.visible(1440.), editor);
        layout.reveal(Pane::Preview, 900.);
        layout.reveal(Pane::Editor, 900.);
        assert_eq!(layout.visible(900.), editor);
        assert_eq!(layout.panes, editor);
    }

    #[test]
    fn agent_pane_preserves_legacy_preferences_and_returns_from_compact_preview() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("workspace.json");
        fs::write(&settings, br#"{"editor":true,"preview":true}"#).unwrap();
        let mut layout = WorkspaceLayout::new(PaneVisibility::read(&settings));
        assert!(layout.panes.agent);
        layout.expand_chat(true);
        assert!(!layout.visible(1000.).editor);
        assert!(layout.visible(1000.).agent);
        assert!(!layout.visible(1000.).preview);
        layout.reveal(Pane::Preview, 1000.);
        assert!(!layout.visible(1000.).agent);
        layout.toggle(Pane::Agent, 1000., layout.visible(1000.));
        assert!(layout.visible(1000.).agent);
        assert!(!layout.visible(1000.).editor);
        layout.panes.save(&settings).unwrap();
        assert_eq!(PaneVisibility::read(&settings), layout.panes);
    }
}
