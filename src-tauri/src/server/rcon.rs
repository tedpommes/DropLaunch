//! A small Rust WebRCON client on its own thread: ws://127.0.0.1:<port>/<password>, JSON
//! `{Identifier, Message, Name}` out, `{Identifier, Message, Type}` back. It reconnects until
//! dropped. Broadcast log lines (Identifier <= 0) are ignored: the log file already has them.
//! Except chat (Type "Chat"): the server sends it as data (who, which channel, what), which a
//! player can't fake by typing something that looks like a log line.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::ErrorKind;
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tungstenite::Message;

pub enum Event {
    Connected,
    Disconnected(String),
    Reply { id: i32, text: String },
    Chat(Chat),
}

/// A chat message, as the server sent it over RCON.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Chat {
    pub name: String,
    pub steam_id: String,
    pub text: String,
    /// global | team | server | cards | local | clan | other
    pub channel: &'static str,
    /// Milliseconds since 1970.
    pub time: i64,
}

/// Rust's chat entry (`{Channel, Message, UserId, Username, Color, Time}`), checked: no control
/// characters, a Steam ID of digits, a known channel, limited lengths. None: not one.
pub fn parse_chat(message: &str) -> Option<Chat> {
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Entry {
        #[serde(default)]
        channel: Value,
        #[serde(default)]
        message: String,
        #[serde(default)]
        user_id: Value,
        #[serde(default)]
        username: String,
        #[serde(default)]
        time: i64,
    }
    let entry: Entry = serde_json::from_str(message.trim()).ok()?;
    let channel = match &entry.channel {
        Value::Number(n) => match n.as_i64() {
            Some(0) => "global",
            Some(1) => "team",
            Some(2) => "server",
            Some(3) => "cards",
            Some(4) => "local",
            Some(5) => "clan",
            _ => "other",
        },
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "global" => "global",
            "team" => "team",
            "server" => "server",
            "cards" => "cards",
            "local" => "local",
            "clan" => "clan",
            _ => "other",
        },
        _ => "other",
    };
    let steam_id: String = match &entry.user_id {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    };
    let steam_id = if !steam_id.is_empty() && steam_id.len() <= 20 && steam_id.bytes().all(|b| b.is_ascii_digit()) { steam_id } else { String::new() };
    let plain = |text: &str, max: usize| -> String { text.chars().map(|c| if c.is_control() { ' ' } else { c }).take(max).collect::<String>().trim().to_string() };
    let text = plain(&entry.message, 500);
    if text.is_empty() {
        return None;
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    // Seconds since 1970 (a time from before 2015 or in the future: now).
    let time = Some(entry.time.saturating_mul(1000)).filter(|&t| t > 1_420_000_000_000 && t < now + 86_400_000).unwrap_or(now);
    Some(Chat { name: plain(&entry.username, 64), steam_id, text, channel, time })
}

pub struct Client {
    commands: Sender<(i32, String)>,
    events: Receiver<Event>,
    stop: Arc<AtomicBool>,
    /// Closes when the thread has finished (connection closed properly).
    done: Receiver<()>,
}

impl Client {
    pub fn start(port: u16, password: String) -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let (done_tx, done) = mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("rcon".into())
            .spawn(move || {
                run(port, &password, &command_rx, &event_tx, &flag);
                drop(done_tx);
            })
            .expect("spawn rcon thread");
        Self { commands, events, stop, done }
    }

    /// Disconnect and wait (up to `wait`) until the server has acknowledged it.
    pub fn close(self, wait: Duration) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.done.recv_timeout(wait);
    }

    pub fn send(&self, id: i32, text: &str) {
        let _ = self.commands.send((id, text.to_string()));
    }

    pub fn poll(&self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(ev) = self.events.try_recv() {
            out.push(ev);
        }
        out
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

#[derive(Deserialize)]
struct Incoming {
    #[serde(rename = "Identifier", default)]
    identifier: i32,
    #[serde(rename = "Message", default)]
    message: String,
    #[serde(rename = "Type", default)]
    kind: String,
}

fn run(port: u16, password: &str, commands: &Receiver<(i32, String)>, events: &Sender<Event>, stop: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        match connect(port, password) {
            Ok(mut ws) => {
                let _ = events.send(Event::Connected);
                let reason = session(&mut ws, commands, events, stop);
                let _ = events.send(Event::Disconnected(reason));
            }
            Err(_) => {
                // Not up yet, or busy loading: commands sent meanwhile are dropped.
                while commands.try_recv().is_ok() {}
            }
        }
        for _ in 0..20 {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn connect(port: u16, password: &str) -> anyhow::Result<tungstenite::WebSocket<TcpStream>> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let url = format!("ws://127.0.0.1:{port}/{}", encode(password));
    let (ws, _) = tungstenite::client(url.as_str(), stream).map_err(|e| anyhow::anyhow!("{e}"))?;
    ws.get_ref().set_nonblocking(true)?;
    Ok(ws)
}

fn session(
    ws: &mut tungstenite::WebSocket<TcpStream>,
    commands: &Receiver<(i32, String)>,
    events: &Sender<Event>,
    stop: &AtomicBool,
) -> String {
    loop {
        if stop.load(Ordering::SeqCst) {
            close_politely(ws);
            return "closed".into();
        }
        loop {
            match commands.try_recv() {
                Ok((id, text)) => {
                    let json = serde_json::json!({ "Identifier": id, "Message": text, "Name": "DropLaunch" }).to_string();
                    match ws.send(Message::text(json)) {
                        Ok(()) => {}
                        Err(tungstenite::Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => {}
                        Err(e) => return e.to_string(),
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return "closed".into(),
            }
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                if let Ok(msg) = serde_json::from_str::<Incoming>(text.as_str()) {
                    if msg.identifier > 0 {
                        let _ = events.send(Event::Reply { id: msg.identifier, text: msg.message });
                    } else if msg.kind == "Chat" {
                        if let Some(chat) = parse_chat(&msg.message) {
                            let _ = events.send(Event::Chat(chat));
                        }
                    }
                }
            }
            Ok(Message::Close(_)) => return "the server closed the connection".into(),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(40)),
            Err(e) => return e.to_string(),
        }
    }
}

/// Say goodbye and wait briefly for the server's answer. Dropping a socket that still has unread
/// data (the server keeps broadcasting its log) makes Windows reset the connection, which the
/// server then logs as an exception.
fn close_politely(ws: &mut tungstenite::WebSocket<TcpStream>) {
    let _ = ws.close(None);
    let deadline = Instant::now() + Duration::from_millis(800);
    while Instant::now() < deadline {
        match ws.read() {
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => {
                let _ = ws.flush();
                std::thread::sleep(Duration::from_millis(10));
            }
            // ConnectionClosed: the close handshake is done.
            Err(_) => return,
        }
    }
}

/// Percent-encode a password for the URL path.
fn encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn passwords_are_url_safe() {
        assert_eq!(super::encode("ab C/d%"), "ab%20C%2Fd%25");
    }

    /// Rust's chat entry becomes a message with a known channel, a Steam ID of digits and no
    /// control characters; anything else isn't one.
    #[test]
    fn chat_comes_from_the_servers_own_data() {
        let chat = super::parse_chat(r##"{"Channel":1,"Message":"hi\nthere","UserId":"76561198000000001","Username":"Ann","Color":"#5af","Time":1763380000}"##).unwrap();
        assert_eq!(chat.channel, "team");
        assert_eq!(chat.steam_id, "76561198000000001");
        assert_eq!(chat.name, "Ann");
        assert_eq!(chat.text, "hi there");
        assert_eq!(chat.time, 1_763_380_000_000);
        // A number for the id, a name for the channel, no time: still read.
        let chat = super::parse_chat(r#"{"Channel":"Global","Message":"gg","UserId":76561198000000002,"Username":"Bob"}"#).unwrap();
        assert_eq!((chat.channel, chat.steam_id.as_str()), ("global", "76561198000000002"));
        // An id that isn't one is left out; an empty message or a log line isn't chat.
        assert_eq!(super::parse_chat(r#"{"Channel":0,"Message":"x","UserId":"7656 OR 1","Username":"C"}"#).unwrap().steam_id, "");
        assert!(super::parse_chat(r#"{"Channel":0,"Message":"  ","UserId":"1"}"#).is_none());
        assert!(super::parse_chat("[CHAT] Bob[76561198000000002] : hi").is_none());
    }
}
