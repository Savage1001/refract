use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::Path,
};

const COMPACT_WIDTH: f32 = 1200.;
const MIN_EDITOR_WIDTH: f32 = 300.;
const PDF_CLEARANCE: f32 = 292.;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pane {
    Editor,
    Preview,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Divider {
    Sidebar,
    Editor,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct PaneVisibility {
    pub editor: bool,
    pub preview: bool,
}

impl Default for PaneVisibility {
    fn default() -> Self {
        Self {
            editor: true,
            preview: true,
        }
    }
}

impl PaneVisibility {
    pub fn read(path: &Path) -> Self {
        let mut panes: Self = File::open(path)
            .ok()
            .and_then(|file| serde_json::from_reader(file.take(1024)).ok())
            .unwrap_or_default();
        if !panes.editor && !panes.preview {
            panes.editor = true;
        }
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
        if Self::compact(width) {
            let preview = self.compact_preview || (self.panes.preview && !self.panes.editor);
            PaneVisibility {
                editor: !preview,
                preview,
            }
        } else {
            self.panes
        }
    }

    pub fn toggle(&mut self, pane: Pane, width: f32, displayed: PaneVisibility) {
        if Self::compact(width)
            && (self.compact_preview || (pane == Pane::Preview && self.panes.editor))
        {
            self.compact_preview = pane == Pane::Preview && !displayed.preview;
            if pane == Pane::Editor && !self.compact_preview {
                self.panes.editor = true;
            }
        } else {
            match pane {
                Pane::Editor => self.panes.editor = !displayed.editor,
                Pane::Preview => self.panes.preview = !displayed.preview,
            }
            if !self.panes.editor && !self.panes.preview {
                self.panes.editor = true;
            }
        }
    }

    pub fn reveal(&mut self, pane: Pane, width: f32) {
        match pane {
            Pane::Editor => {
                self.compact_preview = false;
                self.panes.editor = true;
            }
            Pane::Preview if Self::compact(width) => self.compact_preview = true,
            Pane::Preview => self.panes.preview = true,
        }
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

    pub fn resize(&mut self, divider: Divider, width: f32, viewport: f32) {
        match divider {
            Divider::Sidebar => {
                let maximum = (viewport * 0.4).floor().max(150.);
                self.sidebar_width = Some(width.clamp(150., maximum));
            }
            Divider::Editor => {
                let maximum =
                    (viewport - self.sidebar_width(viewport) - PDF_CLEARANCE).max(MIN_EDITOR_WIDTH);
                self.editor_width = Some(width.clamp(MIN_EDITOR_WIDTH, maximum));
            }
        }
    }
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
    fn compact_preview_switches_without_overwriting_the_desktop_choices() {
        let mut layout = WorkspaceLayout::default();
        let editor = PaneVisibility {
            editor: true,
            preview: false,
        };
        let preview = PaneVisibility {
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
        layout.toggle(Pane::Editor, 1440., editor);
        assert_eq!(layout.visible(1440.), editor);
        layout.reveal(Pane::Preview, 900.);
        layout.reveal(Pane::Editor, 900.);
        assert_eq!(layout.visible(900.), editor);
        assert_eq!(layout.panes, editor);
    }
}
