//! Tiny reader for Valve's text KeyValues format (`appmanifest_*.acf`, `app_info_print`).
//! Anything outside quotes/braces is skipped, so console noise around a dump is harmless.

#[derive(Debug, Clone, PartialEq)]
pub enum Vdf {
    Str(String),
    Obj(Vec<(String, Vdf)>),
}

impl Vdf {
    /// Case-insensitive child lookup (Steam isn't consistent about key casing).
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Obj(items) => items.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v),
            Vdf::Str(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            Vdf::Obj(_) => None,
        }
    }

    pub fn entries(&self) -> &[(String, Vdf)] {
        match self {
            Vdf::Obj(items) => items,
            Vdf::Str(_) => &[],
        }
    }
}

enum Token {
    Str(String),
    Open,
    Close,
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '{' => tokens.push(Token::Open),
            '}' => tokens.push(Token::Close),
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(other) => s.push(other),
                            None => break,
                        },
                        other => s.push(other),
                    }
                }
                tokens.push(Token::Str(s));
            }
            _ => {}
        }
    }
    tokens
}

/// Parse into a root object holding the top-level key/value pairs.
pub fn parse(text: &str) -> Vdf {
    let tokens = tokenize(text);
    let mut iter = tokens.into_iter();
    Vdf::Obj(parse_object(&mut iter))
}

fn parse_object(iter: &mut impl Iterator<Item = Token>) -> Vec<(String, Vdf)> {
    let mut items = Vec::new();
    while let Some(token) = iter.next() {
        match token {
            Token::Close => break,
            Token::Open => {
                parse_object(iter); // stray block without a key
            }
            Token::Str(key) => match iter.next() {
                Some(Token::Str(value)) => items.push((key, Vdf::Str(value))),
                Some(Token::Open) => items.push((key, Vdf::Obj(parse_object(iter)))),
                Some(Token::Close) | None => break,
            },
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_and_noise() {
        let text = "AppID : 1, change number : 5\n\"AppState\"\n{\n\t\"buildid\"\t\t\"42\"\n\t\"UserConfig\"\n\t{\n\t\t\"BetaKey\"\t\"staging\"\n\t}\n}\n";
        let root = parse(text);
        let app = root.get("appstate").unwrap();
        assert_eq!(app.get("BuildID").and_then(Vdf::as_str), Some("42"));
        assert_eq!(app.get("UserConfig").and_then(|u| u.get("BetaKey")).and_then(Vdf::as_str), Some("staging"));
    }

    #[test]
    fn escapes() {
        let root = parse(r#""k" "a \"quoted\" \\ value""#);
        assert_eq!(root.get("k").and_then(Vdf::as_str), Some(r#"a "quoted" \ value"#));
    }
}
