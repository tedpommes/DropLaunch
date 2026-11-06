//! DropLaunch's colours. Four themes come with it (`ui/themes`: Default, Light, Grey, Vibrant); a
//! theme file dropped on the exe or the window is added (copied into the data folder's Themes
//! folder) and used. Settings › Visuals switches between them, and saves any of them as a file to
//! make your own from.
//!
//! A theme file: `{"theme": {"name": "Midnight", "mode": "dark"}, "colors": {"accent": "#7a95e3"}}`,
//! colours as #rrggbb (#rrggbbaa: see-through), named as in `ui/themes/default.json`. It only needs
//! the colours it changes: the rest come from Default (from Light, for a "light" theme).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Themes added by dropping a file on DropLaunch (in the data folder).
pub const FOLDER: &str = "Themes";
pub const DEFAULT: &str = "default";
const LIGHT: &str = "light";

/// The themes that come with DropLaunch, in the order Settings shows them.
const BUILT_IN: [(&str, &str); 4] = [
    (DEFAULT, include_str!("../../ui/themes/default.json")),
    (LIGHT, include_str!("../../ui/themes/light.json")),
    ("grey", include_str!("../../ui/themes/grey.json")),
    ("vibrant", include_str!("../../ui/themes/vibrant.json")),
];

/// Every colour a theme has, in the order a theme file lists them (styles.css has each as a
/// `--name` variable).
pub const KEYS: [&str; 36] = [
    "bg", "bg-deep", "card", "inset", "console", "popup", "tip", "line", "line-strong", "overlay", "shadow", "text", "text-strong", "muted", "faint",
    "accent", "accent-hover", "on-accent", "on-bright", "ok", "warn", "danger", "violet", "orange", "cyan", "pink", "mint", "coral", "src-server",
    "src-map", "src-carbon", "src-oxide", "src-rustedit", "src-rcon", "src-droplaunch", "src-steamcmd",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Dark,
    Light,
}

/// Where a theme comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    BuiltIn,
    /// Dropped on DropLaunch (kept in the data folder's Themes folder).
    Added,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub mode: Mode,
    pub source: Source,
    pub file: Option<PathBuf>,
    /// Every colour (the ones its file leaves out from Default or Light).
    pub colors: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Meta {
    name: String,
    #[serde(default)]
    mode: Mode,
}

/// A theme file's text: has a "theme" part with a name, and its colours. Its mistakes are found
/// later (`read`), so a theme file with one says what's wrong instead of being something else.
fn looks_like_theme(text: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    (value["theme"]["name"].is_string() && value["colors"].is_object()).then_some(value)
}

/// `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa` -> `#rrggbb(aa)` in small letters.
fn color(text: &str) -> Option<String> {
    let hex = text.trim().strip_prefix('#')?;
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let hex = hex.to_ascii_lowercase();
    match hex.len() {
        3 | 4 => Some(format!("#{}", hex.chars().flat_map(|c| [c, c]).collect::<String>())),
        6 | 8 => Some(format!("#{hex}")),
        _ => None,
    }
}

/// A theme's name, mode and the colours it has (known ones only; others are left out).
fn parse(text: &str, file: &str) -> Result<(String, Mode, BTreeMap<String, String>), String> {
    let value = looks_like_theme(text).ok_or_else(|| crate::t!("theme.error.notTheme", file))?;
    let meta: Meta = serde_json::from_value(value["theme"].clone()).map_err(|_| crate::t!("theme.error.meta", file))?;
    let name = meta.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 40 {
        return Err(crate::t!("theme.error.meta", file));
    }
    let mut colors = BTreeMap::new();
    for (key, value) in value["colors"].as_object().into_iter().flatten() {
        if !KEYS.contains(&key.as_str()) {
            continue;
        }
        let hex = value.as_str().and_then(color).ok_or_else(|| crate::t!("theme.error.color", file, key, value))?;
        colors.insert(key.clone(), hex);
    }
    Ok((name, meta.mode, colors))
}

fn read(path: &Path) -> Result<(String, Mode, BTreeMap<String, String>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| crate::t!("theme.error.read", file = path.display(), error = e))?;
    parse(&text, &file_name(path))
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn builtin() -> &'static [Theme] {
    static THEMES: OnceLock<Vec<Theme>> = OnceLock::new();
    THEMES.get_or_init(|| {
        BUILT_IN
            .iter()
            .map(|(id, text)| {
                let (name, mode, colors) = parse(text, id).expect("ui/themes holds theme files");
                Theme { id: id.to_string(), name, mode, source: Source::BuiltIn, file: None, colors }
            })
            .collect()
    })
}

/// A theme's own colours over the ones it leaves out (Default's, or Light's for a light theme).
fn complete(mode: Mode, own: BTreeMap<String, String>) -> BTreeMap<String, String> {
    let base = if mode == Mode::Light { LIGHT } else { DEFAULT };
    let mut colors = builtin().iter().find(|t| t.id == base).map(|t| t.colors.clone()).unwrap_or_default();
    colors.extend(own);
    colors
}

/// An added theme's id: its file's name (one that's a built-in theme's gets "-file").
fn added_id(path: &Path) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    if builtin().iter().any(|t| t.id == stem) {
        format!("{stem}-file")
    } else {
        stem
    }
}

fn added(path: PathBuf) -> Option<Theme> {
    let (name, mode, own) = read(&path).ok()?;
    Some(Theme { id: added_id(&path), name, mode, source: Source::Added, colors: complete(mode, own), file: Some(path) })
}

/// Every theme there is: the built-in ones, then the added ones (by name).
pub fn list(data: &Path) -> Vec<Theme> {
    let mut themes = builtin().to_vec();
    let mut more: Vec<Theme> = std::fs::read_dir(data.join(FOLDER))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) && p.is_file())
        .filter_map(added)
        .collect();
    more.sort_by_key(|t| t.name.to_lowercase());
    themes.extend(more);
    themes
}

/// The theme to use: the one picked in Settings, else Default.
pub fn choose<'a>(picked: Option<&str>, themes: &'a [Theme]) -> &'a Theme {
    picked.and_then(|id| themes.iter().find(|t| t.id == id)).unwrap_or(&themes[0])
}

/// A theme file's name: the theme's name in plain letters (`Night Owl` -> `night-owl.json`).
fn file_stem(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-');
    match out {
        "" => "theme".into(),
        // (Not a built-in one's name: those stay as they are.)
        _ if builtin().iter().any(|t| t.id == out) => format!("{out}-theme"),
        _ => out.into(),
    }
}

/// A theme file dropped on DropLaunch: checked, then kept in the data folder's Themes folder (a newer
/// file with the same name replaces the older one).
pub fn import(path: &Path, data: &Path) -> Result<Theme, String> {
    let (name, _, _) = read(path)?;
    let dir = data.join(FOLDER);
    std::fs::create_dir_all(&dir).map_err(|e| crate::t!("theme.error.save", error = e))?;
    let target = dir.join(format!("{}.json", file_stem(&name)));
    if !crate::startfile::same_path(path, &target) {
        std::fs::copy(path, &target).map_err(|e| crate::t!("theme.error.save", error = e))?;
    }
    added(target.clone()).ok_or_else(|| crate::t!("theme.error.read", file = target.display(), error = "?"))
}

/// A theme file, by its looks (a .json file with a "theme" part and colours).
pub fn is_theme_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) && std::fs::read_to_string(path).is_ok_and(|t| looks_like_theme(&t).is_some())
}

/// A theme as a file, with every colour (what Save as a file writes: the built-in ones exactly as
/// they are in ui/themes).
pub fn file_text(theme: &Theme) -> String {
    let mode = if theme.mode == Mode::Light { "light" } else { "dark" };
    let colors: Vec<String> = KEYS.iter().filter_map(|k| theme.colors.get(*k).map(|v| format!("    \"{k}\": \"{v}\""))).collect();
    format!(
        "{{\n  \"theme\": {{ \"name\": {}, \"mode\": \"{mode}\" }},\n  \"colors\": {{\n{}\n  }}\n}}\n",
        serde_json::to_string(&theme.name).unwrap_or_default(),
        colors.join(",\n")
    )
}

/// The window around the page in the theme's colours too: its title bar light or dark, and what
/// shows before the page does in the theme's background.
pub fn dress(window: &tauri::WebviewWindow, theme: &Theme) {
    let _ = window.set_theme(Some(if theme.mode == Mode::Light { tauri::Theme::Light } else { tauri::Theme::Dark }));
    let bg = theme.colors.get("bg").and_then(|c| u32::from_str_radix(c.get(1..7)?, 16).ok());
    if let Some(n) = bg {
        let _ = window.set_background_color(Some(tauri::window::Color((n >> 16) as u8, (n >> 8) as u8, n as u8, 255)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("droplaunch-theme-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Each built-in theme has every colour, and its file is what Save as a file writes.
    #[test]
    fn the_built_in_themes_are_complete() {
        let ids: Vec<&str> = builtin().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["default", "light", "grey", "vibrant"]);
        for (theme, (_, text)) in builtin().iter().zip(BUILT_IN) {
            assert_eq!(theme.colors.len(), KEYS.len(), "{}", theme.id);
            assert_eq!(file_text(theme), text, "{}", theme.id);
        }
        assert_eq!(builtin()[1].mode, Mode::Light);
    }

    /// styles.css starts in Default's colours: the same values, and the triplets of the ones it
    /// uses see-through (rgb(var(--accent-rgb) / 0.2)).
    #[test]
    fn the_stylesheet_starts_in_the_default_theme() {
        let css = include_str!("../../ui/styles.css");
        let root = &css[css.find(":root {").unwrap()..];
        let root = &root[..root.find('}').unwrap()];
        let var = |name: &str| root.lines().find_map(|l| l.trim().strip_prefix(&format!("--{name}: "))?.strip_suffix(';').map(str::to_string));
        for (key, value) in &builtin()[0].colors {
            assert_eq!(var(key).as_deref(), Some(value.as_str()), "--{key}");
        }
        for key in css.split("var(--").skip(1).filter_map(|s| s.split(')').next()?.strip_suffix("-rgb")).collect::<std::collections::BTreeSet<_>>() {
            let n = u32::from_str_radix(&builtin()[0].colors[key][1..7], 16).unwrap();
            assert_eq!(var(&format!("{key}-rgb")), Some(format!("{} {} {}", n >> 16, (n >> 8) & 255, n & 255)), "--{key}-rgb");
        }
    }

    #[test]
    fn a_theme_file_needs_only_the_colours_it_changes() {
        let (name, mode, colors) = parse(r##"{"theme":{"name":" Night "},"colors":{"accent":"#ABC","line":"#ffffff20","unknown":"#000"}}"##, "x").unwrap();
        assert_eq!((name.as_str(), mode), ("Night", Mode::Dark));
        assert_eq!(colors.len(), 2);
        assert_eq!(colors["accent"], "#aabbcc");
        let full = complete(mode, colors);
        assert_eq!(full.len(), KEYS.len());
        assert_eq!(full["bg"], builtin()[0].colors["bg"]);
        // A light one fills in from Light.
        let (_, mode, colors) = parse(r#"{"theme":{"name":"Day","mode":"light"},"colors":{}}"#, "x").unwrap();
        assert_eq!(complete(mode, colors)["bg"], builtin()[1].colors["bg"]);

        assert!(parse(r#"{"theme":{"name":"Bad"},"colors":{"bg":"red"}}"#, "x").unwrap_err().contains("bg"));
        assert!(parse(r#"{"theme":{"name":"Bad"},"colors":{"bg":"url(x)"}}"#, "x").is_err());
        assert!(parse(r#"{"theme":{"name":"Bad","mode":"dim"},"colors":{}}"#, "x").is_err());
        assert!(parse(r#"{"theme":{"name":""},"colors":{}}"#, "x").is_err());
        assert!(looks_like_theme(r#"{"language":{"code":"nl","name":"Nederlands"}}"#).is_none());
        assert!(looks_like_theme("\u{feff}{\"theme\":{\"name\":\"A\"},\"colors\":{}}").is_some());
    }

    #[test]
    fn dropped_themes_are_kept_and_replaced_by_name() {
        let base = temp_dir("base");
        let data = temp_dir("data");
        let dropped = base.join("my theme.json");
        std::fs::write(&dropped, r##"{"theme":{"name":"Night Owl"},"colors":{"accent":"#ff00aa"}}"##).unwrap();
        assert!(is_theme_file(&dropped));
        let theme = import(&dropped, &data).unwrap();
        assert_eq!((theme.id.as_str(), theme.source), ("night-owl", Source::Added));
        assert_eq!(theme.file.as_deref(), Some(data.join(FOLDER).join("night-owl.json").as_path()));
        assert_eq!(theme.colors["accent"], "#ff00aa");
        // Again, newer: replaces it. A theme named like a built-in one doesn't take its place.
        std::fs::write(&dropped, r##"{"theme":{"name":"Night Owl"},"colors":{"accent":"#00ff00"}}"##).unwrap();
        import(&dropped, &data).unwrap();
        std::fs::write(&dropped, r#"{"theme":{"name":"Light","mode":"light"},"colors":{}}"#).unwrap();
        assert_eq!(import(&dropped, &data).unwrap().id, "light-theme");
        let themes = list(&data);
        let ids: Vec<&str> = themes.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["default", "light", "grey", "vibrant", "light-theme", "night-owl"]);
        assert_eq!(themes[5].colors["accent"], "#00ff00");
        assert_eq!(choose(Some("night-owl"), &themes).name, "Night Owl");
        assert_eq!(choose(Some("gone"), &themes).id, DEFAULT);
        assert_eq!(choose(None, &themes).id, DEFAULT);

        // Not theme files.
        std::fs::write(base.join("nl.json"), r#"{"language":{"code":"nl","name":"Nederlands"}}"#).unwrap();
        assert!(!is_theme_file(&base.join("nl.json")));
        assert!(import(&base.join("nl.json"), &data).is_err());
        for dir in [base, data] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
