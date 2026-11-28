use gpui::{
    App, FontFeatures, IntoElement, LineLayout, Pixels, RenderOnce, SharedString, Styled, Window,
    canvas, px,
};
use std::sync::{Arc, LazyLock};

// Chromium leaves optional ligatures off when letter spacing is nonzero.
static LETTER_SPACING_FEATURES: LazyLock<FontFeatures> = LazyLock::new(|| {
    FontFeatures(Arc::new(vec![
        ("liga".into(), 0),
        ("clig".into(), 0),
        ("calt".into(), 0),
    ]))
});

pub(crate) fn tracked_label(text: impl Into<SharedString>, spacing_em: f32) -> impl IntoElement {
    TrackedLabel {
        text: text.into(),
        spacing_em,
    }
}

#[derive(IntoElement)]
struct TrackedLabel {
    text: SharedString,
    spacing_em: f32,
}

impl RenderOnce for TrackedLabel {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let height = style
            .line_height
            .to_pixels(style.font_size, window.rem_size());
        let mut run = style.to_run(self.text.len());
        run.font.features = LETTER_SPACING_FEATURES.clone();
        let mut line = window
            .text_system()
            .shape_line(self.text, font_size, &[run], None);
        let layout: &mut Arc<LineLayout> = &mut line;
        *layout = tracked_layout(layout, font_size * self.spacing_em);
        let width = line.width;

        canvas(
            |_, _, _| (),
            move |bounds, (), window, cx| {
                let _ = line.paint(bounds.origin, height, window, cx);
            },
        )
        .w(width)
        .h(height)
    }
}

fn tracked_layout(layout: &LineLayout, letter_spacing: Pixels) -> Arc<LineLayout> {
    let mut runs = layout.runs.clone();
    let mut previous_cluster = None;
    let mut offset = px(0.);
    for glyph in runs.iter_mut().flat_map(|run| &mut run.glyphs) {
        if let Some(cluster) = previous_cluster
            && cluster != glyph.index
        {
            offset += letter_spacing;
        }
        glyph.position.x += offset;
        previous_cluster = Some(glyph.index);
    }
    // The last cluster contributes to the inline width without moving its ink.
    if previous_cluster.is_some() {
        offset += letter_spacing;
    }

    Arc::new(LineLayout {
        font_size: layout.font_size,
        width: (layout.width + offset).max(px(0.)),
        ascent: layout.ascent,
        descent: layout.descent,
        runs,
        len: layout.len,
    })
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use gpui::{TestAppContext, TextStyle};

    #[gpui::test]
    fn places_the_spacing_after_each_cluster(cx: &mut TestAppContext) {
        cx.add_empty_window().update(|window, _| {
            let shaped = window.text_system().shape_line(
                "ABC".into(),
                px(16.),
                &[TextStyle::default().to_run(3)],
                None,
            );
            let mut layout = tracked_layout(&shaped, px(0.));
            let glyphs = &mut Arc::get_mut(&mut layout).unwrap().runs[0].glyphs;
            // Simulate two glyphs originating from the same shaped cluster.
            glyphs[1].index = glyphs[0].index;

            let tracking = px(1.25);
            let tracked = tracked_layout(&layout, tracking);
            let before = &layout.runs[0].glyphs;
            let after = &tracked.runs[0].glyphs;
            assert_eq!(after[0].position.x, before[0].position.x);
            assert_eq!(after[1].position.x, before[1].position.x);
            assert_eq!(after[2].position.x, before[2].position.x + tracking);
            assert_eq!(tracked.width, layout.width + tracking * 2.);
            assert_ne!(
                shaped.runs[0].glyphs[0].index,
                shaped.runs[0].glyphs[1].index
            );
        });
    }
}
