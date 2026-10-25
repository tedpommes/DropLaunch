//! The server's console commands, for the console's autocomplete. Rust lists every command and
//! setting in generated code (ConsoleGen): their names and descriptions are string literals in
//! Assembly-CSharp.dll, so they're read from the installed server itself (always that build's
//! list, for every branch, nothing to keep up to date). Carbon's come from its command
//! attributes, Oxide's from its literals plus a few well-known ones.

use crate::t;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Command {
    /// What's typed: `status`, `server.seed`, `c.reload`.
    pub name: String,
    pub help: String,
    /// rust | carbon | oxide
    pub source: &'static str,
}

/// Read once per server build (the DLL's time and size), not on every keystroke.
type Cached = (Vec<(PathBuf, Option<SystemTime>, u64)>, Vec<Command>);
static CACHE: Mutex<Option<Cached>> = Mutex::new(None);

/// Every command the server in `server_dir` knows, sorted by name.
pub fn list(server_dir: &Path) -> Vec<Command> {
    let managed = server_dir.join("RustDedicated_Data").join("Managed");
    let carbon = server_dir.join("carbon").join("managed");
    let mut files = vec![managed.join("Assembly-CSharp.dll")];
    files.extend(dlls(&managed).into_iter().filter(|p| file_name(p).starts_with("Oxide.")));
    files.push(carbon.join("Carbon.Common.dll"));
    files.extend(dlls(&carbon.join("modules")));
    let key: Vec<_> = files
        .iter()
        .filter_map(|f| std::fs::metadata(f).ok().map(|m| (f.clone(), m.modified().ok(), m.len())))
        .collect();
    if let Some((cached_key, commands)) = CACHE.lock().unwrap().as_ref() {
        if *cached_key == key {
            return commands.clone();
        }
    }

    let mut commands = Vec::new();
    for (file, _, _) in &key {
        let Ok(data) = std::fs::read(file) else { continue };
        let name = file_name(file);
        if name == "Assembly-CSharp.dll" {
            commands.extend(rust_commands(&user_strings(&data)));
        } else if name.starts_with("Oxide.") {
            commands.extend(oxide_commands(&user_strings(&data)));
        } else {
            commands.extend(carbon_commands(&data));
        }
    }
    if key.iter().any(|(f, _, _)| file_name(f).starts_with("Oxide.")) {
        commands.extend(OXIDE.iter().map(|&(name, help)| Command { name: name.into(), help: oxide_help(help, name), source: "oxide" }));
    }
    commands.sort_by(|a, b| a.name.cmp(&b.name));
    commands.dedup_by(|a, b| {
        if a.name == b.name && b.help.is_empty() {
            b.help = std::mem::take(&mut a.help);
        }
        a.name == b.name
    });
    *CACHE.lock().unwrap() = Some((key, commands.clone()));
    commands
}

fn dlls(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|e| e.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dll"))).collect())
        .unwrap_or_default();
    out.sort();
    out
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// `parent.name` (commands are lowercase: `AdventCalendar.OnRpcMessage` is code, not one).
fn is_full(s: &str) -> bool {
    let mut parts = s.splitn(2, '.');
    let (Some(parent), Some(name)) = (parts.next(), parts.next()) else { return false };
    let word = |w: &str| !w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    word(parent) && word(name) && !parent.starts_with(|c: char| c.is_ascii_digit())
}

fn is_word(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn tidy_help(text: &str) -> String {
    text.trim().trim_start_matches("(Generated)").trim().to_string()
}

/// ConsoleGen's literals come in a block, in order: the name, `parent.name`, the description.
/// (A name or description used earlier in the DLL isn't repeated: string literals are stored
/// once.) The block is where `name` + `parent.name` pairs are.
fn rust_commands(strings: &[String]) -> Vec<Command> {
    let pairs: Vec<usize> = (1..strings.len())
        .filter(|&k| is_full(&strings[k]) && strings[k].strip_suffix(strings[k - 1].as_str()).is_some_and(|p| p.ends_with('.')))
        .collect();
    let (Some(&first), Some(&last)) = (pairs.first(), pairs.last()) else { return Vec::new() };
    if pairs.len() < 50 {
        return Vec::new();
    }
    // A few entries before the first and after the last pair (their names came earlier).
    let reach = |start: usize, step: isize| {
        let (mut k, mut edge, mut quiet) = (start as isize, start, 0);
        while k >= 0 && (k as usize) < strings.len() && quiet < 6 {
            if is_full(&strings[k as usize]) {
                edge = k as usize;
                quiet = 0;
            } else {
                quiet += 1;
            }
            k += step;
        }
        edge
    };
    let (first, last) = (reach(first, -1), reach(last, 1));
    let mut out = Vec::new();
    for k in first..=last {
        let full = &strings[k];
        if !is_full(full) {
            continue;
        }
        let help = strings.get(k + 1).filter(|d| !is_full(d) && !is_word(d)).map(|d| tidy_help(d)).unwrap_or_default();
        // Global commands are typed without "global.".
        let name = full.strip_prefix("global.").unwrap_or(full).to_string();
        out.push(Command { name, help, source: "rust" });
    }
    out
}

/// Carbon's commands are attributes: `[CommandVar("reload", "help")]` on its core (typed as
/// `c.reload`), `[ConsoleCommand("module.command", "help")]` in modules.
fn carbon_commands(data: &[u8]) -> Vec<Command> {
    let mut out = Vec::new();
    if let Some(meta) = Metadata::read(data) {
        if let Some(heap) = meta.heap("#Blob") {
            for blob in entries(heap) {
                let args = attribute_strings(blob);
                let (Some(name), Some(help)) = (args.first(), args.get(1)) else { continue };
                if !help.contains(' ') {
                    continue;
                }
                if is_word(name) && name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                    out.push(Command { name: format!("c.{name}"), help: help.trim().to_string(), source: "carbon" });
                } else if is_full(name) {
                    out.push(Command { name: name.clone(), help: help.trim().to_string(), source: "carbon" });
                }
            }
        }
    }
    // The few core ones registered in code.
    out.extend(
        user_strings(data)
            .into_iter()
            .filter(|s| s.starts_with("c.") && is_full(s))
            .map(|name| Command { name, help: String::new(), source: "carbon" }),
    );
    out
}

fn oxide_commands(strings: &[String]) -> Vec<Command> {
    strings
        .iter()
        .filter(|s| is_full(s) && ["o.", "oxide.", "umod.", "u."].iter().any(|p| s.starts_with(p)))
        .map(|name| Command { name: name.clone(), help: String::new(), source: "oxide" })
        .collect()
}

/// Oxide's permission and plugin commands (also as o.*), with what they take (the help is in
/// the language file; {cmd} is the command).
const OXIDE: &[(&str, &str)] = &[
    ("oxide.plugins", "plugins"),
    ("oxide.load", "load"),
    ("oxide.unload", "unload"),
    ("oxide.reload", "reload"),
    ("oxide.grant", "grant"),
    ("oxide.revoke", "revoke"),
    ("oxide.group", "group"),
    ("oxide.usergroup", "usergroup"),
    ("oxide.show", "show"),
    ("oxide.version", "version"),
    ("oxide.lang", "lang"),
    ("o.plugins", "plugins"),
    ("o.load", "load"),
    ("o.unload", "unload"),
    ("o.reload", "reload"),
    ("o.grant", "grant"),
    ("o.revoke", "revoke"),
    ("o.group", "group"),
    ("o.usergroup", "usergroup"),
    ("o.show", "show"),
    ("o.version", "version"),
];

fn oxide_help(which: &str, cmd: &str) -> String {
    match which {
        "plugins" => t!("concmd.oxide.plugins"),
        "load" => t!("concmd.oxide.load", cmd),
        "unload" => t!("concmd.oxide.unload", cmd),
        "reload" => t!("concmd.oxide.reload", cmd),
        "grant" => t!("concmd.oxide.grant", cmd),
        "revoke" => t!("concmd.oxide.revoke", cmd),
        "group" => t!("concmd.oxide.group", cmd),
        "usergroup" => t!("concmd.oxide.usergroup", cmd),
        "show" => t!("concmd.oxide.show", cmd),
        "version" => t!("concmd.oxide.version"),
        _ => t!("concmd.oxide.lang", cmd),
    }
}

// ---------------------------------------------------------------- .NET metadata

/// A .NET assembly's metadata streams (#US, #Blob, ...).
struct Metadata<'a> {
    data: &'a [u8],
    streams: Vec<(String, usize, usize)>,
}

fn u16_at(data: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?) as usize)
}

fn u32_at(data: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?) as usize)
}

impl<'a> Metadata<'a> {
    fn read(data: &'a [u8]) -> Option<Self> {
        let pe = u32_at(data, 0x3C)?;
        if data.get(pe..pe + 4)? != b"PE\0\0" {
            return None;
        }
        let coff = pe + 4;
        let sections = u16_at(data, coff + 2)?;
        let optional_size = u16_at(data, coff + 16)?;
        let optional = coff + 20;
        let directories = optional + if u16_at(data, optional)? == 0x10B { 96 } else { 112 };
        let cli_rva = u32_at(data, directories + 14 * 8)?;
        let table = optional + optional_size;
        let offset = |rva: usize| -> Option<usize> {
            (0..sections).find_map(|i| {
                let at = table + i * 40;
                let (vsize, va, rsize, raw) = (u32_at(data, at + 8)?, u32_at(data, at + 12)?, u32_at(data, at + 16)?, u32_at(data, at + 20)?);
                (va <= rva && rva < va + vsize.max(rsize)).then(|| rva - va + raw)
            })
        };
        let cli = offset(cli_rva)?;
        let root = offset(u32_at(data, cli + 8)?)?;
        if u32_at(data, root)? != 0x424A_5342 {
            return None;
        }
        let mut at = root + 16 + u32_at(data, root + 12)?;
        let count = u16_at(data, at + 2)?;
        at += 4;
        let mut streams = Vec::new();
        for _ in 0..count {
            let (start, size) = (u32_at(data, at)?, u32_at(data, at + 4)?);
            at += 8;
            let end = at + data.get(at..)?.iter().position(|&b| b == 0)?;
            let name = String::from_utf8_lossy(&data[at..end]).into_owned();
            at = (end + 4) & !3;
            streams.push((name, root + start, size));
        }
        Some(Self { data, streams })
    }

    fn heap(&self, name: &str) -> Option<&'a [u8]> {
        let (_, start, size) = self.streams.iter().find(|(n, _, _)| n == name)?;
        self.data.get(*start..start + size)
    }
}

/// A heap's entries: each starts with its length (1, 2 or 4 bytes).
fn entries(heap: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut at = 1; // the first entry is the empty one
    std::iter::from_fn(move || {
        let (len, skip) = compressed(heap.get(at..)?)?;
        let entry = heap.get(at + skip..at + skip + len)?;
        at += skip + len;
        Some(entry)
    })
}

/// ECMA-335 compressed unsigned integer: (value, bytes used).
fn compressed(data: &[u8]) -> Option<(usize, usize)> {
    let b = *data.first()? as usize;
    if b & 0x80 == 0 {
        Some((b, 1))
    } else if b & 0xC0 == 0x80 {
        Some((((b & 0x3F) << 8) | *data.get(1)? as usize, 2))
    } else {
        Some((((b & 0x1F) << 24) | (*data.get(1)? as usize) << 16 | (*data.get(2)? as usize) << 8 | *data.get(3)? as usize, 4))
    }
}

/// The string literals in the code (#US: UTF-16 plus one flag byte each), in order.
fn user_strings(data: &[u8]) -> Vec<String> {
    let Some(heap) = Metadata::read(data).and_then(|m| m.heap("#US")) else { return Vec::new() };
    entries(heap)
        .filter(|e| e.len() > 1)
        .map(|e| {
            let units: Vec<u16> = e[..e.len() - 1].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units)
        })
        .collect()
}

/// The leading string arguments of a custom attribute's value (prolog 01 00).
fn attribute_strings(blob: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    if blob.len() < 3 || blob[0] != 1 || blob[1] != 0 {
        return out;
    }
    let mut at = 2;
    while at < blob.len() {
        if blob[at] == 0xFF {
            out.push(String::new()); // null
            at += 1;
            continue;
        }
        let Some((len, skip)) = compressed(&blob[at..]).filter(|(_, skip)| *skip <= 2) else { break };
        let Some(Ok(text)) = blob.get(at + skip..at + skip + len).map(std::str::from_utf8) else { break };
        out.push(text.to_string());
        at += skip + len;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn consolegen_block_gives_names_and_help() {
        let mut list = vec!["Something", "else"];
        // 50 pairs make it the table.
        let names: Vec<String> = (0..50).map(|i| format!("cmd{i:02}")).collect();
        let fulls: Vec<String> = names.iter().map(|n| format!("server.{n}")).collect();
        for (n, f) in names.iter().zip(&fulls) {
            list.push(n);
            list.push(f);
        }
        list.extend(["status", "global.status", "Print out currently connected clients", "global.find", "(Generated) Search for a command", "seed"]);
        let commands = rust_commands(&strings(&list));
        assert_eq!(commands.len(), 52);
        let status = commands.iter().find(|c| c.name == "status").unwrap();
        assert_eq!(status.help, "Print out currently connected clients");
        // Its name was seen before: no pair, still found, "(Generated)" dropped.
        assert_eq!(commands.iter().find(|c| c.name == "find").unwrap().help, "Search for a command");
        assert_eq!(commands.iter().find(|c| c.name == "server.cmd00").unwrap().help, "");
    }

    #[test]
    fn attribute_strings_read_the_leading_strings() {
        let blob = [1u8, 0, 6, b'r', b'e', b'l', b'o', b'a', b'd', 4, b'a', b' ', b'b', b'c', 0, 0];
        assert_eq!(attribute_strings(&blob), ["reload", "a bc", "", ""]);
    }

    /// The real thing, when a server is installed here (DROPLAUNCH_TEST_SERVER=<Server_Files>).
    #[test]
    #[ignore]
    fn installed_server_lists_its_commands() {
        let Some(dir) = std::env::var_os("DROPLAUNCH_TEST_SERVER") else { return };
        let commands = list(Path::new(&dir));
        println!("{} commands", commands.len());
        for c in commands.iter().step_by(60) {
            println!("  [{}] {} - {}", c.source, c.name, c.help.chars().take(70).collect::<String>());
        }
        for c in commands.iter().filter(|c| c.source != "rust").take(12) {
            println!("  [{}] {} - {}", c.source, c.name, c.help.chars().take(70).collect::<String>());
        }
        for name in ["status", "server.seed", "teleportpos", "kick"] {
            let c = commands.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("{name} missing"));
            println!("{} - {}", c.name, c.help);
        }
        assert!(commands.len() > 1000);
    }
}
