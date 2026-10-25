//! DropLaunch's texts. English is built in (`ui/lang/en.json`, the same file the UI reads), so
//! nothing else is needed to start. Other languages are language files: one next to
//! DropLaunch.exe is used by itself (unless another language was picked in Settings), and one
//! dropped on the exe or the window is added (copied into the app data's Languages folder). A
//! language file only needs the lines it changes: everything else stays English.
//!
//! A language file: `{"language": {"code": "nl", "name": "Nederlands"}, "strings": {"key": "text"}}`,
//! with `{placeholders}` filled in by DropLaunch. `t!("key", name = value)` gives the text in the
//! language in use.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

/// English, the language every other one falls back to.
pub const BUILTIN: &str = include_str!("../../ui/lang/en.json");
pub const ENGLISH: &str = "en";
/// Languages added by dropping a file on DropLaunch (in the app data folder).
pub const FOLDER: &str = "Languages";

/// The text in the language in use: `t!("key")`, or `t!("key", name = value, other)` for its
/// `{name}` and `{other}` placeholders (a bare name passes the variable of that name).
#[macro_export]
macro_rules! t {
    ($key:literal $(,)?) => {
        $crate::lang::text($key, &[])
    };
    ($key:literal, $($name:ident $(= $value:expr)?),+ $(,)?) => {
        $crate::lang::text($key, &[$((stringify!($name), &$crate::__t_value!($name $(, $value)?) as &dyn ::std::fmt::Display)),+])
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __t_value {
    ($name:ident) => {
        $name
    };
    ($name:ident, $value:expr) => {
        $value
    };
}

#[derive(Deserialize)]
struct LangFile {
    language: Meta,
    #[serde(default)]
    strings: HashMap<String, String>,
}

#[derive(Deserialize)]
struct Meta {
    code: String,
    name: String,
}

/// Where a language comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    BuiltIn,
    /// Dropped on DropLaunch (kept in the app data's Languages folder).
    Added,
    /// Next to DropLaunch.exe.
    Folder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Language {
    pub code: String,
    pub name: String,
    pub source: Source,
    pub file: Option<PathBuf>,
}

impl Language {
    fn english() -> Self {
        Self { code: ENGLISH.into(), name: "English".into(), source: Source::BuiltIn, file: None }
    }
}

struct Active {
    language: Language,
    /// Only what the language file has (English fills in the rest).
    strings: HashMap<String, String>,
}

fn builtin() -> &'static HashMap<String, String> {
    static STRINGS: OnceLock<HashMap<String, String>> = OnceLock::new();
    STRINGS.get_or_init(|| serde_json::from_str::<LangFile>(BUILTIN).expect("ui/lang/en.json is a language file").strings)
}

fn active() -> &'static RwLock<Option<Arc<Active>>> {
    static ACTIVE: OnceLock<RwLock<Option<Arc<Active>>>> = OnceLock::new();
    ACTIVE.get_or_init(|| RwLock::new(None))
}

/// The text for `key` in the language in use, with its placeholders filled in (the key itself
/// when no language has it: a missing line shows up instead of failing).
pub fn text(key: &str, args: &[(&str, &dyn Display)]) -> String {
    let current = active().read().unwrap().clone();
    let template = current.as_ref().and_then(|a| a.strings.get(key)).or_else(|| builtin().get(key)).map_or(key, String::as_str);
    fill(&plain(template), args)
}

/// Texts for the UI can have a few tags (<b>, <em>, <code>, <a>, <br>); the backend's are plain.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let tag = tail.find('>').map(|end| &tail[..=end]);
        let name = tag.map(|t| t.trim_start_matches(['<', '/']).split([' ', '>', '/']).next().unwrap_or(""));
        match (tag, name) {
            (Some(t), Some("b" | "strong" | "em" | "code" | "a" | "br")) => {
                if name == Some("br") {
                    out.push('\n');
                }
                rest = &tail[t.len()..];
            }
            _ => {
                out.push('<');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `{name}` -> the value given for it; unknown ones stay as they are.
fn fill(template: &str, args: &[(&str, &dyn Display)]) -> String {
    if args.is_empty() {
        return template.to_string();
    }
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}').map(|close| (&after[..close], close)) {
            Some((name, close)) if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => {
                match args.iter().find(|(n, _)| *n == name) {
                    Some((_, value)) => out.push_str(&value.to_string()),
                    None => out.push_str(&rest[open..open + close + 2]),
                }
                rest = &after[close + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A language file's code and name, and its lines.
fn read_file(path: &Path) -> Result<(Meta, HashMap<String, String>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| crate::t!("lang.error.read", file = path.display(), error = e))?;
    let file: LangFile = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| crate::t!("lang.error.notLanguage", file = file_name(path), error = e))?;
    let code = file.language.code.trim().to_string();
    let name = file.language.name.trim().to_string();
    let code_ok = !code.is_empty() && code.len() <= 16 && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !code_ok || name.is_empty() || name.chars().count() > 40 {
        return Err(crate::t!("lang.error.meta", file = file_name(path)));
    }
    Ok((Meta { code, name }, file.strings))
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Language files in `dir` (other .json files are left alone).
fn scan(dir: &Path, source: Source) -> Vec<Language> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<Language> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) && p.is_file())
        .filter_map(|p| read_file(&p).ok().map(|(meta, _)| Language { code: meta.code, name: meta.name, source, file: Some(p) }))
        .collect();
    found.sort_by(|a, b| a.file.cmp(&b.file));
    found
}

/// Every language there is: English, the added ones, the ones next to the exe (the same code
/// further down this list wins: a file next to the exe can even change English).
pub fn list(base: &Path, data: &Path) -> Vec<Language> {
    let mut languages = vec![Language::english()];
    for language in scan(&data.join(FOLDER), Source::Added).into_iter().chain(scan(base, Source::Folder)) {
        match languages.iter_mut().find(|l| l.code.eq_ignore_ascii_case(&language.code)) {
            Some(existing) => *existing = language,
            None => languages.push(language),
        }
    }
    languages
}

/// The language to use: the one picked in Settings, else one next to the exe, else English.
pub fn choose<'a>(picked: Option<&str>, languages: &'a [Language]) -> &'a Language {
    picked
        .and_then(|code| languages.iter().find(|l| l.code.eq_ignore_ascii_case(code)))
        .or_else(|| languages.iter().find(|l| l.source == Source::Folder))
        .unwrap_or(&languages[0])
}

/// Use `language` from now on. A file that can't be read leaves English in place.
pub fn activate(language: &Language) -> Result<(), String> {
    let strings = match &language.file {
        Some(path) => read_file(path)?.1,
        None => HashMap::new(),
    };
    *active().write().unwrap() = Some(Arc::new(Active { language: language.clone(), strings }));
    Ok(())
}

/// The language in use.
pub fn current() -> Language {
    active().read().unwrap().as_ref().map_or_else(Language::english, |a| a.language.clone())
}

/// Every text of the language in use, English where it has none (for the UI).
pub fn strings() -> HashMap<String, String> {
    let mut all = builtin().clone();
    if let Some(current) = active().read().unwrap().as_ref() {
        all.extend(current.strings.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    all
}

/// A language file dropped on DropLaunch: checked, then kept in the app data's Languages folder
/// (a newer file with the same code replaces the older one).
pub fn import(path: &Path, data: &Path) -> Result<Language, String> {
    let (meta, _) = read_file(path)?;
    let dir = data.join(FOLDER);
    std::fs::create_dir_all(&dir).map_err(|e| crate::t!("lang.error.save", error = e))?;
    let target = dir.join(format!("{}.json", meta.code.to_ascii_lowercase()));
    if !crate::startfile::same_path(path, &target) {
        std::fs::copy(path, &target).map_err(|e| crate::t!("lang.error.save", error = e))?;
    }
    Ok(Language { code: meta.code, name: meta.name, source: Source::Added, file: Some(target) })
}

/// A language file, by its name (a .json file that has a "language" part).
pub fn is_language_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) && read_file(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("droplaunch-lang-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn placeholders_are_filled_and_tags_dropped() {
        let n = 3;
        assert_eq!(fill("{n} maps in {dir}", &[("n", &n), ("dir", &"C:\\Maps")]), "3 maps in C:\\Maps");
        assert_eq!(fill("{missing} {n}", &[("n", &n)]), "{missing} 3");
        assert_eq!(fill("a { b } {}", &[("n", &n)]), "a { b } {}");
        assert_eq!(plain("Drop <b>.map</b> files<br>here, a < b"), "Drop .map files\nhere, a < b");
    }

    #[test]
    fn english_is_built_in_and_every_line_is_text() {
        assert!(builtin().len() > 100);
        assert!(builtin().values().all(|v| !v.trim().is_empty()));
    }

    #[test]
    fn files_next_to_the_exe_win_and_are_used_by_themselves() {
        let base = temp_dir("base");
        let data = temp_dir("data");
        std::fs::create_dir_all(data.join(FOLDER)).unwrap();
        std::fs::write(base.join("nederlands.json"), r#"{"language":{"code":"nl","name":"Nederlands"},"strings":{"x":"y"}}"#).unwrap();
        std::fs::write(data.join(FOLDER).join("de.json"), "\u{feff}{\"language\":{\"code\":\"de\",\"name\":\"Deutsch\"}}").unwrap();
        // Not language files.
        std::fs::write(base.join("droplaunch.json"), r#"{"branch":"public"}"#).unwrap();
        std::fs::write(base.join("bad.json"), r#"{"language":{"code":"x y","name":"Bad"}}"#).unwrap();
        let languages = list(&base, &data);
        let codes: Vec<&str> = languages.iter().map(|l| l.code.as_str()).collect();
        assert_eq!(codes, ["en", "de", "nl"]);
        assert_eq!(choose(None, &languages).code, "nl");
        assert_eq!(choose(Some("de"), &languages).code, "de");
        assert_eq!(choose(Some("EN"), &languages).code, "en");
        assert_eq!(choose(Some("gone"), &languages).code, "nl");

        // Dropped on DropLaunch: copied in, the same code replaces the older file.
        let dropped = base.join("de-new.json");
        std::fs::write(&dropped, r#"{"language":{"code":"de","name":"Deutsch (neu)"}}"#).unwrap();
        let added = import(&dropped, &data).unwrap();
        assert_eq!((added.name.as_str(), added.source), ("Deutsch (neu)", Source::Added));
        assert_eq!(list(&base, &data).iter().filter(|l| l.code == "de").count(), 1);
        assert!(import(&base.join("droplaunch.json"), &data).is_err());
        for dir in [base, data] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
