use super::synctex::SourcePosition;
use crate::valid_project_component;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticLevel {
    Error,
    Warning,
}

pub fn diagnostic_source(line: &str) -> Option<SourcePosition> {
    let local = line
        .strip_prefix("./")
        .or_else(|| line.strip_prefix(".\\"))?;
    let mut parts = local.splitn(3, ':');
    let file = parts.next()?.replace('\\', "/");
    let number = parts.next()?.parse().ok()?;
    if number == 0
        || parts.next()?.trim().is_empty()
        || !file.split('/').all(valid_project_component)
    {
        return None;
    }
    Some(SourcePosition { file, line: number })
}

pub fn diagnostic_level(line: &str) -> Option<DiagnosticLevel> {
    let lower = line.to_ascii_lowercase();
    if line.starts_with('!')
        || diagnostic_source(line).is_some()
        || lower.split(':').any(|part| {
            part.trim_start()
                .strip_prefix("error")
                .is_some_and(|suffix| {
                    !suffix.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
                })
        })
    {
        Some(DiagnosticLevel::Error)
    } else if lower
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|word| word == "warning")
    {
        Some(DiagnosticLevel::Warning)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_tex_errors_and_warnings_and_limits_source_links_to_project_paths() {
        let line = ".\\chapters\\résumé.tex:24: Undefined control sequence.";
        assert_eq!(diagnostic_level(line), Some(DiagnosticLevel::Error));
        assert_eq!(
            diagnostic_source(line),
            Some(SourcePosition {
                file: "chapters/résumé.tex".into(),
                line: 24
            })
        );
        assert!(diagnostic_source("./../outside.tex:12: Invalid input").is_none());
        assert!(diagnostic_source("./sections/.internal.tex:1: Error").is_none());
        assert_eq!(
            diagnostic_level("! LaTeX Error: File not found."),
            Some(DiagnosticLevel::Error)
        );
        assert_eq!(
            diagnostic_level("driver: error"),
            Some(DiagnosticLevel::Error)
        );
        assert_eq!(
            diagnostic_level("Package foo Warning: Re-run."),
            Some(DiagnosticLevel::Warning)
        );
        assert_eq!(diagnostic_level("Output written on main.pdf."), None);
    }
}
