use gpui::{App, rgba};
use gpui_component::{
    Theme, ThemeConfig,
    highlighter::{HighlightTheme, LanguageConfig, LanguageRegistry, ThemeStyle},
};
use refract_native::latex_syntax;
use serde_json::json;
use std::rc::Rc;

pub(super) fn init(cx: &mut App) {
    let config = LanguageConfig::new(
        latex_syntax::NAME,
        latex_syntax::language(),
        Vec::new(),
        latex_syntax::HIGHLIGHTS,
        "",
        "",
    );
    LanguageRegistry::singleton().register(latex_syntax::NAME, &config);

    #[cfg(not(target_os = "macos"))]
    let inter_loaded = {
        let data: &[u8] = include_bytes!("../fonts/Inter-Variable.ttf");
        match cx
            .text_system()
            .add_fonts(vec![std::borrow::Cow::Borrowed(data)])
        {
            Ok(()) => true,
            Err(error) => {
                eprintln!("Couldn’t load the bundled Inter font. {error}");
                false
            }
        }
    };
    let theme = Theme::global_mut(cx);
    #[cfg(not(target_os = "macos"))]
    if inter_loaded {
        Rc::make_mut(&mut theme.light_theme).font_family = Some("Inter".into());
        Rc::make_mut(&mut theme.dark_theme).font_family = Some("Inter".into());
    }
    configure(
        Rc::make_mut(&mut theme.light_theme),
        &HighlightTheme::default_light(),
        ["#1b4ed8", "#507447", "#996018", "#956434"],
        0x27272a0a,
    );
    configure(
        Rc::make_mut(&mut theme.dark_theme),
        &HighlightTheme::default_dark(),
        ["#79a8ff", "#8fb58a", "#e5b567", "#d8ba8e"],
        0xf5f5f50a,
    );
    let mode = theme.mode;
    Theme::change(mode, None, cx);
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use gpui::{TestAppContext, rgb};
    use gpui_component::{ThemeMode, highlighter::SyntaxHighlighter, input::Rope};

    #[gpui::test]
    fn uses_the_refract_source_colors_in_light_and_dark_modes(cx: &mut TestAppContext) {
        cx.update(|app| {
            crate::initialize_app(app);
            let source = "\\textbf{a} $x$ % comment";
            let mut highlighter = SyntaxHighlighter::new(latex_syntax::NAME);
            highlighter.update(None, &Rope::from(source));
            for (mode, expected) in [
                (ThemeMode::Light, [0x1b4ed8, 0x956434, 0x996018, 0x507447]),
                (ThemeMode::Dark, [0x79a8ff, 0xd8ba8e, 0xe5b567, 0x8fb58a]),
            ] {
                Theme::change(mode, None, app);
                let styles =
                    highlighter.styles(&(0..source.len()), &Theme::global(app).highlight_theme);
                for (offset, color) in [0, 7, 11, 15].into_iter().zip(expected) {
                    let style = styles
                        .iter()
                        .find(|(range, _)| range.contains(&offset))
                        .unwrap();
                    assert_eq!(style.1.color, Some(rgb(color).into()));
                }
            }
        });
    }
}

fn configure(
    config: &mut ThemeConfig,
    defaults: &HighlightTheme,
    colors: [&str; 4],
    active_line: u32,
) {
    let style = config
        .highlight
        .get_or_insert_with(|| defaults.style.clone());
    let [command, comment, math, bracket] = colors.map(|color| {
        serde_json::from_value::<ThemeStyle>(json!({"color": color})).expect("valid source color")
    });
    style.syntax.keyword = Some(command);
    style.syntax.comment = Some(comment);
    style.syntax.punctuation_special = Some(math);
    style.syntax.punctuation_bracket = Some(bracket);
    style.editor_active_line = Some(rgba(active_line).into());
}
