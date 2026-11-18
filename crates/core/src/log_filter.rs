use crate::types::{ConsoleLine, ConsoleLineType};

pub fn filter_logs(
    lines: &[ConsoleLine],
    query: &str,
    kinds: &[ConsoleLineType],
) -> Vec<ConsoleLine> {
    let query = query.trim().to_lowercase();
    lines
        .iter()
        .filter(|l| kinds.is_empty() || kinds.contains(&l.kind))
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
            line("Steve joined the game", ConsoleLineType::Stdout),
            line("Server overloaded", ConsoleLineType::Stderr),
            line("Starting server...", ConsoleLineType::System),
        ];
        assert_eq!(filter_logs(&lines, "", &[]).len(), 3);
        assert_eq!(filter_logs(&lines, "STEVE", &[]).len(), 1);
        assert_eq!(filter_logs(&lines, "", &[ConsoleLineType::Stderr]).len(), 1);
        assert!(filter_logs(&lines, "steve", &[ConsoleLineType::System]).is_empty());
    }
}
