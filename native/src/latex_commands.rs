use std::ops::Range;

pub struct Command {
    pub name: &'static str,
    pub snippet: &'static str,
    pub caret: usize,
}

macro_rules! command {
    ($name:literal, $suffix:literal, $into_suffix:expr) => {
        Command {
            name: $name,
            snippet: concat!("\\", $name, $suffix),
            caret: 1 + $name.len() + $into_suffix,
        }
    };
}

const COMMANDS: &[Command] = &[
    command!("begin", "{}", 1),
    command!("end", "{}", 1),
    command!("section", "{}", 1),
    command!("subsection", "{}", 1),
    command!("textbf", "{}", 1),
    command!("textit", "{}", 1),
    command!("emph", "{}", 1),
    command!("cite", "{}", 1),
    command!("ref", "{}", 1),
    command!("label", "{}", 1),
    command!("includegraphics", "{}", 1),
    command!("frac", "{}{}", 1),
    command!("sqrt", "{}", 1),
    command!("item", " ", 1),
    command!("usepackage", "{}", 1),
    command!("documentclass", "{}", 1),
    command!("subsubsection", "{}", 1),
    command!("paragraph", "{}", 1),
    command!("chapter", "{}", 1),
    command!("caption", "{}", 1),
    command!("footnote", "{}", 1),
    command!("url", "{}", 1),
    command!("href", "{}{}", 1),
    command!("input", "{}", 1),
    command!("include", "{}", 1),
    command!("bibliography", "{}", 1),
    command!("texttt", "{}", 1),
    command!("underline", "{}", 1),
    command!("mathrm", "{}", 1),
    command!("mathbf", "{}", 1),
    command!("mathit", "{}", 1),
    command!("sum", "", 0),
    command!("int", "", 0),
    command!("alpha", "", 0),
    command!("beta", "", 0),
    command!("gamma", "", 0),
    command!("delta", "", 0),
    command!("infty", "", 0),
    command!("times", "", 0),
    command!("cdot", "", 0),
    command!("left", "", 0),
    command!("right", "", 0),
];

pub struct Suggestions {
    pub range: Range<usize>,
    pub items: Vec<&'static Command>,
}

pub fn suggest(line: &str, caret: usize, allow_empty: bool) -> Option<Suggestions> {
    let before = line.get(..caret)?;
    let letters = before
        .bytes()
        .rev()
        .take_while(u8::is_ascii_alphabetic)
        .count();
    let prefix_start = caret - letters;
    let (prefix, range) = if prefix_start > 0 && line.as_bytes()[prefix_start - 1] == b'\\' {
        let suffix = line[caret..]
            .bytes()
            .take_while(u8::is_ascii_alphabetic)
            .count();
        (
            before[prefix_start..].to_ascii_lowercase(),
            prefix_start - 1..caret + suffix,
        )
    } else if allow_empty {
        (String::new(), caret..caret)
    } else {
        return None;
    };
    let items: Vec<_> = COMMANDS
        .iter()
        .filter(|command| command.name.starts_with(&prefix))
        .collect();
    (!items.is_empty()).then_some(Suggestions { range, items })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_commands_and_replaces_the_entire_token_at_a_unicode_offset() {
        let text = "λ \\TeX rest";
        let results = suggest(text, "λ \\Te".len(), false).unwrap();
        assert_eq!(&text[results.range], r"\TeX");
        assert_eq!(
            results
                .items
                .iter()
                .map(|command| command.name)
                .collect::<Vec<_>>(),
            ["textbf", "textit", "texttt"]
        );
        let item = results.items[1];
        assert_eq!(item.snippet, r"\textit{}");
        assert_eq!(item.caret, 8);
    }

    #[test]
    fn opens_the_complete_list_only_on_request_when_no_command_precedes_the_caret() {
        let all = suggest("plain", 4, true).unwrap();
        assert_eq!(all.range, 4..4);
        assert_eq!(all.items.len(), 42);
        assert_eq!(all.items[0].name, "begin");
        assert_eq!(all.items[41].name, "right");
        assert!(suggest("plain", 4, false).is_none());
        assert!(suggest(r"\unknown", 8, true).is_none());
        assert!(suggest("λ", 1, true).is_none());
    }
}
