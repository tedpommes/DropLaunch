//! Turns a pseudo-console byte stream into finished text lines.
//!
//! ConPTY does not forward the child's bytes untouched: it renders a virtual screen and sends
//! the changes. Slow output arrives as plain text + CRLF, but bursts (e.g. `app_info_print`)
//! are painted with cursor jumps (`ESC[row;colH`, `ESC[nC`) until the bottom row is reached,
//! after which it scrolls with CRLF again. Stripping the escape codes would glue those lines
//! together, so this keeps a one-row cursor model instead: a line is finished whenever the
//! cursor leaves its row.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    EscapeIntermediate,
    Csi,
    Osc,
    OscEscape,
}

pub struct LineAssembler {
    state: State,
    params: String,
    screen_rows: u16,
    row: u16,
    col: usize,
    current: Vec<char>,
    /// Bytes of a UTF-8 character that was split across two reads.
    utf8_tail: Vec<u8>,
}

impl LineAssembler {
    pub fn new(screen_rows: u16) -> Self {
        Self {
            state: State::Ground,
            params: String::new(),
            screen_rows: screen_rows.max(1),
            row: 1,
            col: 0,
            current: Vec::new(),
            utf8_tail: Vec::new(),
        }
    }

    /// Feed raw bytes; returns every non-empty line completed by this chunk.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut done = Vec::new();
        let mut data = std::mem::take(&mut self.utf8_tail);
        data.extend_from_slice(bytes);
        let mut rest = data.as_slice();
        while !rest.is_empty() {
            match std::str::from_utf8(rest) {
                Ok(text) => {
                    self.feed_str(text, &mut done);
                    break;
                }
                Err(err) => {
                    let (valid, after) = rest.split_at(err.valid_up_to());
                    self.feed_str(std::str::from_utf8(valid).unwrap_or_default(), &mut done);
                    match err.error_len() {
                        Some(bad) => {
                            self.feed_char('\u{FFFD}', &mut done);
                            rest = &after[bad..];
                        }
                        None => {
                            // Incomplete character at the end: wait for the next read.
                            self.utf8_tail = after.to_vec();
                            break;
                        }
                    }
                }
            }
        }
        done
    }

    /// Text on the row currently being written (not yet finished by a newline).
    pub fn partial(&self) -> String {
        self.current.iter().collect::<String>().trim().to_string()
    }

    /// End of stream: whatever is on the current row counts as a finished line.
    pub fn finish(&mut self) -> Option<String> {
        let line = self.take_line();
        (!line.is_empty()).then_some(line)
    }

    fn take_line(&mut self) -> String {
        let line: String = self.current.drain(..).collect();
        line.trim_end().to_string()
    }

    fn end_line(&mut self, done: &mut Vec<String>) {
        let line = self.take_line();
        if !line.is_empty() {
            done.push(line);
        }
    }

    fn feed_str(&mut self, text: &str, done: &mut Vec<String>) {
        for ch in text.chars() {
            self.feed_char(ch, done);
        }
    }

    fn feed_char(&mut self, ch: char, done: &mut Vec<String>) {
        match self.state {
            State::Ground => match ch {
                '\x1b' => self.state = State::Escape,
                '\r' => self.col = 0,
                '\n' => {
                    self.end_line(done);
                    self.row = (self.row + 1).min(self.screen_rows);
                }
                '\x08' => self.col = self.col.saturating_sub(1),
                '\t' => self.col = (self.col / 8 + 1) * 8,
                c if c.is_control() => {}
                c => self.put(c),
            },
            State::Escape => match ch {
                '[' => {
                    self.params.clear();
                    self.state = State::Csi;
                }
                ']' => self.state = State::Osc,
                ' '..='/' => self.state = State::EscapeIntermediate, // e.g. ESC ( B
                _ => self.state = State::Ground,                    // ESC 7, ESC =, ...
            },
            State::EscapeIntermediate => self.state = State::Ground,
            State::Csi => match ch {
                ' '..='?' => self.params.push(ch),
                '@'..='~' => {
                    self.csi(ch, done);
                    self.state = State::Ground;
                }
                _ => self.state = State::Ground,
            },
            State::Osc => match ch {
                '\x07' => self.state = State::Ground,
                '\x1b' => self.state = State::OscEscape,
                _ => {} // window title etc.
            },
            State::OscEscape => {
                self.state = if ch == '\\' { State::Ground } else { State::Osc };
            }
        }
    }

    fn put(&mut self, ch: char) {
        if self.col < self.current.len() {
            self.current[self.col] = ch;
        } else {
            self.current.resize(self.col, ' ');
            self.current.push(ch);
        }
        self.col += 1;
    }

    fn csi(&mut self, final_byte: char, done: &mut Vec<String>) {
        // Private modes (?25l cursor, ?9001h win32 input, ?1004h focus, ...) don't touch text.
        if self.params.starts_with(|c: char| matches!(c, '?' | '>' | '=' | '<')) {
            return;
        }
        let nums: Vec<Option<usize>> = self.params.split(';').map(|p| p.parse().ok()).collect();
        let arg = |i: usize| nums.get(i).copied().flatten();
        let count = arg(0).filter(|&n| n > 0).unwrap_or(1);
        match final_byte {
            // Cursor position: leaving the row finishes the line on it.
            'H' | 'f' => {
                let row = count.min(self.screen_rows as usize) as u16;
                let col = arg(1).filter(|&n| n > 0).unwrap_or(1) - 1;
                if row != self.row {
                    self.end_line(done);
                }
                self.row = row;
                self.col = col;
            }
            'A' => {
                self.end_line(done);
                self.row = self.row.saturating_sub(count as u16).max(1);
            }
            'B' | 'E' => {
                self.end_line(done);
                self.row = (self.row as usize + count).min(self.screen_rows as usize) as u16;
                if final_byte == 'E' {
                    self.col = 0;
                }
            }
            'C' => self.col += count,
            'D' => self.col = self.col.saturating_sub(count),
            'G' => self.col = count - 1,
            // Erase in line.
            'K' => match arg(0).unwrap_or(0) {
                0 => self.current.truncate(self.col),
                1 => self.current.iter_mut().take(self.col + 1).for_each(|c| *c = ' '),
                _ => self.current.clear(),
            },
            // Erase characters (conhost uses this instead of printing spaces).
            'X' => {
                let end = (self.col + count).min(self.current.len());
                let start = self.col.min(end);
                self.current[start..end].iter_mut().for_each(|c| *c = ' ');
            }
            'J' => {
                if matches!(arg(0), Some(2) | Some(3)) {
                    self.end_line(done);
                }
            }
            _ => {} // colours (m) and anything else that doesn't move text
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LineAssembler;

    fn lines(input: &str) -> Vec<String> {
        let mut vt = LineAssembler::new(50);
        let mut out = vt.feed(input.as_bytes());
        out.extend(vt.finish());
        out
    }

    #[test]
    fn plain_crlf_lines() {
        assert_eq!(lines("Loading Steam API...OK\r\nWaiting...\r\n"), ["Loading Steam API...OK", "Waiting..."]);
    }

    #[test]
    fn cursor_jumps_split_lines_and_cuf_becomes_spaces() {
        let painted = "\x1b[?25l\"258550\"\r\n{\x1b[15;9H\"common\"\x1b[16;9H{\x1b[17;17H\"name\"\x1b[10C\"Rust\"";
        let pad = |n: usize| " ".repeat(n);
        let expected = [
            "\"258550\"".to_string(),
            "{".to_string(),
            format!("{}\"common\"", pad(8)),
            format!("{}{{", pad(8)),
            format!("{}\"name\"{}\"Rust\"", pad(16), pad(10)),
        ];
        assert_eq!(lines(painted), expected);
    }

    #[test]
    fn titles_modes_and_colours_are_ignored() {
        let input = "\x1b[?9001h\x1b[?1004h\x1b[2J\x1b[m\x1b[HRedirecting\r\n\x1b]0;C:\\steamcmd.exe\x07Logging\r\n";
        assert_eq!(lines(input), ["Redirecting", "Logging"]);
    }

    #[test]
    fn carriage_return_overwrites_in_place() {
        let mut vt = LineAssembler::new(50);
        assert!(vt.feed(b"[  5%] Downloading").is_empty());
        vt.feed(b"\r[ 60%] Downloading");
        assert_eq!(vt.partial(), "[ 60%] Downloading");
    }

    #[test]
    fn split_escape_and_utf8_sequences() {
        let mut vt = LineAssembler::new(50);
        let bytes = "a\x1b[10Cb é\r\n".as_bytes();
        let mut out = Vec::new();
        for b in bytes {
            out.extend(vt.feed(std::slice::from_ref(b)));
        }
        assert_eq!(out, [format!("a{}b é", " ".repeat(10))]);
    }

    #[test]
    fn erase_to_end_of_line() {
        assert_eq!(lines("progress 99%\r\x1b[Kdone\r\n"), ["done"]);
    }
}
