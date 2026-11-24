use std::ops::Range;

use iced::advanced::text::highlighter::{self, Format};
use iced::{Font, Theme};
use monolith_theme::token::{AMBER, CORAL_HOVER, INK_TERTIARY, MOSS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Syntax {
    Plain,
    Yaml,
    Json,
    Toml,
    Properties,
    Mcfunction,
}

impl Syntax {
    pub fn of(name: &str) -> Syntax {
        let ext = name
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_ascii_lowercase());
        match ext.as_deref() {
            Some("yml" | "yaml") => Syntax::Yaml,
            Some("json" | "json5" | "mcmeta") => Syntax::Json,
            Some("toml") => Syntax::Toml,
            Some("properties" | "cfg" | "conf" | "ini") => Syntax::Properties,
            Some("mcfunction") => Syntax::Mcfunction,
            _ => Syntax::Plain,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Comment,
    Key,
    Text,
    Number,
}

pub struct Highlighter {
    syntax: Syntax,
    line: usize,
}

impl highlighter::Highlighter for Highlighter {
    type Settings = Syntax;
    type Highlight = Token;
    type Iterator<'a> = std::vec::IntoIter<(Range<usize>, Token)>;

    fn new(syntax: &Syntax) -> Self {
        Highlighter {
            syntax: *syntax,
            line: 0,
        }
    }

    fn update(&mut self, syntax: &Syntax) {
        self.syntax = *syntax;
        self.line = 0;
    }

    fn change_line(&mut self, line: usize) {
        self.line = self.line.min(line);
    }

    fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_> {
        self.line += 1;
        tokens(self.syntax, line).into_iter()
    }

    fn current_line(&self) -> usize {
        self.line
    }
}

pub fn format(token: &Token, _theme: &Theme) -> Format<Font> {
    Format {
        color: Some(match token {
            Token::Comment => INK_TERTIARY,
            Token::Key => MOSS,
            Token::Text => AMBER,
            Token::Number => CORAL_HOVER,
        }),
        font: None,
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '+')
}

fn word_token(word: &str) -> Option<Token> {
    let literal = matches!(
        word.to_ascii_lowercase().as_str(),
        "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "~"
    );
    (literal || word.parse::<f64>().is_ok()).then_some(Token::Number)
}

fn values(line: &str, from: usize, hash_comments: bool, out: &mut Vec<(Range<usize>, Token)>) {
    let mut chars = line[from..]
        .char_indices()
        .map(|(i, c)| (i + from, c))
        .peekable();
    let mut previous = ' ';
    while let Some((start, c)) = chars.next() {
        if c == '#' && hash_comments && previous.is_whitespace() {
            out.push((start..line.len(), Token::Comment));
            return;
        }
        if c == '"' || c == '\'' {
            let mut end = line.len();
            let mut escaped = false;
            for (i, next) in chars.by_ref() {
                if next == c && !escaped {
                    end = i + 1;
                    break;
                }
                escaped = next == '\\' && !escaped;
            }
            out.push((start..end, Token::Text));
            previous = c;
            continue;
        }
        if is_word(c) {
            let mut end = start + c.len_utf8();
            while let Some(&(i, next)) = chars.peek() {
                if !is_word(next) {
                    break;
                }
                end = i + next.len_utf8();
                chars.next();
            }
            if let Some(token) = word_token(&line[start..end]) {
                out.push((start..end, token));
            }
        }
        previous = c;
    }
}

fn leading(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

pub fn tokens(syntax: Syntax, line: &str) -> Vec<(Range<usize>, Token)> {
    let mut out = Vec::new();
    let start = leading(line);
    let body = &line[start..];
    match syntax {
        Syntax::Plain => {}
        _ if body.starts_with('#')
            || (syntax == Syntax::Properties && body.starts_with([';', '!'])) =>
        {
            out.push((start..line.len(), Token::Comment));
        }
        Syntax::Yaml => {
            let mut at = start;
            if let Some(item) = body.strip_prefix("- ") {
                at += 2 + leading(item);
            }
            let rest = &line[at..];
            let colon = match rest.chars().next() {
                Some(quote @ ('"' | '\'')) => rest[1..]
                    .find(quote)
                    .map(|i| i + 2)
                    .filter(|&i| rest[i..] == *":" || rest[i..].starts_with(": ")),
                _ => rest
                    .find(": ")
                    .or_else(|| rest.ends_with(':').then(|| rest.len() - 1)),
            };
            if let Some(colon) = colon {
                out.push((at..at + colon, Token::Key));
                at += colon + 1;
            }
            values(line, at, true, &mut out);
        }
        Syntax::Toml | Syntax::Properties => {
            if body.starts_with('[') {
                out.push((start..start + body.trim_end().len(), Token::Key));
                return out;
            }
            let separator = body.find(['=', ':']);
            if let Some(i) = separator {
                out.push((start..start + body[..i].trim_end().len(), Token::Key));
                let value = start + i + 1;
                if syntax == Syntax::Properties {
                    let text = line[value..].trim();
                    if !text.is_empty() {
                        let from = value + leading(&line[value..]);
                        let token = word_token(text).unwrap_or(Token::Text);
                        out.push((from..from + text.len(), token));
                    }
                } else {
                    values(line, value, true, &mut out);
                }
            }
        }
        Syntax::Json => {
            values(line, 0, false, &mut out);
            for (range, token) in out.iter_mut() {
                if *token == Token::Text && line[range.end..].trim_start().starts_with(':') {
                    *token = Token::Key;
                }
            }
        }
        Syntax::Mcfunction => {
            let command = body.find(char::is_whitespace).unwrap_or(body.len());
            out.push((start..start + command, Token::Key));
            values(line, start + command, false, &mut out);
            let mut at = start + command;
            while let Some(i) = line[at..].find('@') {
                let from = at + i;
                let end = (from + 2).min(line.len());
                if line.is_char_boundary(end) {
                    out.push((from..end, Token::Text));
                }
                at = from + 1;
            }
            out.sort_by_key(|(range, _)| range.start);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(syntax: Syntax, line: &str) -> Vec<(&str, Token)> {
        tokens(syntax, line)
            .into_iter()
            .map(|(range, token)| (&line[range], token))
            .collect()
    }

    #[test]
    fn picks_syntax_by_extension() {
        assert_eq!(Syntax::of("paper-global.YML"), Syntax::Yaml);
        assert_eq!(Syntax::of("server.properties"), Syntax::Properties);
        assert_eq!(Syntax::of("load.mcfunction"), Syntax::Mcfunction);
        assert_eq!(Syntax::of("README"), Syntax::Plain);
    }

    #[test]
    fn highlights_yaml() {
        use Token::*;
        assert_eq!(
            spans(Syntax::Yaml, "  enabled: true # note"),
            [("enabled", Key), ("true", Number), ("# note", Comment)]
        );
        assert_eq!(spans(Syntax::Yaml, "  - 'a: b'"), [("'a: b'", Text)]);
        assert_eq!(
            spans(Syntax::Yaml, "secret: 'x#y'"),
            [("secret", Key), ("'x#y'", Text)]
        );
        assert_eq!(spans(Syntax::Yaml, "  # comment"), [("# comment", Comment)]);
    }

    #[test]
    fn highlights_json_toml_properties_and_functions() {
        use Token::*;
        assert_eq!(
            spans(Syntax::Json, r#"  "name": "Steve", "level": 4,"#),
            [
                (r#""name""#, Key),
                (r#""Steve""#, Text),
                (r#""level""#, Key),
                ("4", Number)
            ]
        );
        assert_eq!(
            spans(Syntax::Toml, "port = 25565 # default"),
            [("port", Key), ("25565", Number), ("# default", Comment)]
        );
        assert_eq!(spans(Syntax::Toml, "[server]"), [("[server]", Key)]);
        assert_eq!(
            spans(Syntax::Properties, "motd=A Minecraft Server"),
            [("motd", Key), ("A Minecraft Server", Text)]
        );
        assert_eq!(
            spans(Syntax::Mcfunction, "give @a diamond 64"),
            [("give", Key), ("@a", Text), ("64", Number)]
        );
    }
}
