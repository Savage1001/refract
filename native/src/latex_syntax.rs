use tree_sitter::Language;
use tree_sitter_language::LanguageFn;

pub const NAME: &str = "refract-latex";
pub const HIGHLIGHTS: &str = include_str!("../latex-syntax/highlights.scm");

unsafe extern "C" {
    fn tree_sitter_refract_latex() -> *const ();
}

pub fn language() -> Language {
    // The generated Tree-sitter parser returns a pointer to its static language descriptor.
    let parser = unsafe { LanguageFn::from_raw(tree_sitter_refract_latex) };
    Language::new(parser)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

    #[test]
    fn colors_commands_and_delimiters_without_highlighting_comment_contents() {
        let source =
            "α \\section*{A} \\% literal % \\ignored [$]\r\n$x$ \\begin[] $$\\@name$$ \\ \\~\n";
        let language = language();
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();
        let tree = parser.parse(source, None).unwrap();
        assert!(!tree.root_node().has_error());
        let query = Query::new(&language, HIGHLIGHTS).unwrap();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.captures(&query, tree.root_node(), source.as_bytes());
        let mut captured = Vec::new();
        while let Some((matched, index)) = matches.next() {
            let item = matched.captures[*index];
            let category = query.capture_names()[item.index as usize];
            captured.push((category, &source[item.node.byte_range()]));
        }
        assert_eq!(
            captured,
            [
                ("keyword", r"\section*"),
                ("punctuation.bracket", "{"),
                ("punctuation.bracket", "}"),
                ("keyword", r"\%"),
                ("comment", "% \\ignored [$]\r"),
                ("punctuation.special", "$"),
                ("punctuation.special", "$"),
                ("keyword", r"\begin"),
                ("punctuation.bracket", "["),
                ("punctuation.bracket", "]"),
                ("punctuation.special", "$$"),
                ("keyword", r"\@name"),
                ("punctuation.special", "$$"),
                ("keyword", r"\~"),
            ]
        );
    }

    #[test]
    fn accepts_source_while_a_command_or_group_is_incomplete() {
        let language = language();
        let mut parser = Parser::new();
        parser.set_language(&language).unwrap();
        let tree = parser.parse("\\begin{ λ\n\\", None).unwrap();
        assert!(!tree.root_node().has_error());
        assert_eq!(tree.root_node().named_child_count(), 2);
    }
}
