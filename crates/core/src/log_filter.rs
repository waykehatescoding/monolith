use crate::types::{ConsoleLine, ConsoleLineType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

pub fn line_level(text: &str) -> LogLevel {
    let head = text.get(..text.len().min(64)).unwrap_or(text);
    if head.contains("ERROR]") || head.contains("FATAL]") || head.contains("SEVERE]") {
        LogLevel::Error
    } else if head.contains("WARN]") || head.contains("WARNING]") {
        LogLevel::Warn
    } else {
        LogLevel::Info
    }
}

pub fn level_of(line: &ConsoleLine) -> LogLevel {
    match line.kind {
        ConsoleLineType::Stderr => LogLevel::Error,
        _ => line_level(&line.text),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogView {
    #[default]
    All,
    Warnings,
    Errors,
}

impl LogView {
    pub fn allows(self, line: &ConsoleLine) -> bool {
        match self {
            LogView::All => true,
            LogView::Warnings => level_of(line) >= LogLevel::Warn,
            LogView::Errors => level_of(line) == LogLevel::Error,
        }
    }
}

pub fn filter_logs(lines: &[ConsoleLine], query: &str, view: LogView) -> Vec<ConsoleLine> {
    let query = query.trim().to_lowercase();
    lines
        .iter()
        .filter(|l| view.allows(l))
        .filter(|l| query.is_empty() || l.text.to_lowercase().contains(&query))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, kind: ConsoleLineType) -> ConsoleLine {
        ConsoleLine {
            timestamp: String::new(),
            text: text.into(),
            kind,
        }
    }

    #[test]
    fn filters_by_text_and_level() {
        let lines = vec![
            line(
                "[12:00:00 INFO]: Steve joined the game",
                ConsoleLineType::Stdout,
            ),
            line("Server overloaded", ConsoleLineType::Stderr),
            line("[12:00:00 WARN]: Can't keep up!", ConsoleLineType::Stdout),
            line("Starting server...", ConsoleLineType::System),
        ];
        assert_eq!(filter_logs(&lines, "", LogView::All).len(), 4);
        assert_eq!(filter_logs(&lines, "STEVE", LogView::All).len(), 1);
        assert_eq!(filter_logs(&lines, "", LogView::Warnings).len(), 2);
        assert_eq!(filter_logs(&lines, "", LogView::Errors).len(), 1);
        assert!(filter_logs(&lines, "steve", LogView::Errors).is_empty());
    }

    #[test]
    fn detects_levels() {
        assert_eq!(
            line_level("[12:00:00 WARN]: Can't keep up!"),
            LogLevel::Warn
        );
        assert_eq!(
            line_level("[12:00:00] [Server thread/ERROR]: boom"),
            LogLevel::Error
        );
        assert_eq!(line_level("[12:00:00 INFO]: Done"), LogLevel::Info);
        assert_eq!(line_level(&"é".repeat(40)), LogLevel::Info);
        assert_eq!(
            level_of(&line("oops", ConsoleLineType::Stderr)),
            LogLevel::Error
        );
    }
}
