use vte::{Params, Parser, Perform};

#[derive(Clone, Debug)]
pub struct TerminalState {
    pub rows: usize,
    pub cols: usize,
    pub lines: Vec<String>,
    pub cursor_row: usize,
    pub cursor_col: usize,
}

impl TerminalState {
    pub fn new(rows: usize, cols: usize) -> Self {
        let rows = rows.max(1);
        let cols = cols.max(1);
        Self {
            rows,
            cols,
            lines: vec![String::new(); rows],
            cursor_row: 0,
            cursor_col: 0,
        }
    }

    pub fn resize(&mut self, rows: usize, cols: usize) {
        self.rows = rows.max(1);
        self.cols = cols.max(1);
        self.lines.resize(self.rows, String::new());
        if self.cursor_row >= self.rows {
            self.cursor_row = self.rows - 1;
        }
        if self.cursor_col >= self.cols {
            self.cursor_col = self.cols - 1;
        }
    }

    pub fn diff_json(&self) -> Vec<u8> {
        // Simple full-state diff for M6; can be optimized later.
        let mut out = String::from("{\"rows\":");
        out.push_str(&self.rows.to_string());
        out.push_str(",\"cols\":");
        out.push_str(&self.cols.to_string());
        out.push_str(",\"cursor\":[");
        out.push_str(&self.cursor_row.to_string());
        out.push(',');
        out.push_str(&self.cursor_col.to_string());
        out.push_str("],\"lines\":[");
        for (i, line) in self.lines.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('"');
            for ch in line.chars() {
                match ch {
                    '\\' => out.push_str("\\\\"),
                    '"' => out.push_str("\\\""),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        out.push_str("]}");
        out.into_bytes()
    }
}

pub struct TerminalEngine {
    parser: Parser,
    pub state: TerminalState,
}

impl TerminalEngine {
    pub fn new(rows: usize, cols: usize) -> Self {
        Self {
            parser: Parser::new(),
            state: TerminalState::new(rows, cols),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        for b in bytes {
            let mut performer = TerminalPerformer {
                state: &mut self.state,
            };
            self.parser.advance(&mut performer, *b);
        }
    }

    pub fn resize(&mut self, rows: usize, cols: usize) {
        self.state.resize(rows, cols);
    }
}

struct TerminalPerformer<'a> {
    state: &'a mut TerminalState,
}

impl<'a> TerminalPerformer<'a> {
    fn put_char(&mut self, c: char) {
        let row = self.state.cursor_row.min(self.state.rows.saturating_sub(1));
        let col = self.state.cursor_col.min(self.state.cols.saturating_sub(1));
        let line = &mut self.state.lines[row];
        // ensure line length in chars
        let mut chars: Vec<char> = line.chars().collect();
        if chars.len() < self.state.cols {
            chars.resize(self.state.cols, ' ');
        }
        if col < chars.len() {
            chars[col] = c;
        }
        *line = chars
            .into_iter()
            .collect::<String>()
            .trim_end_matches(' ')
            .to_string();
        self.state.cursor_col += 1;
        if self.state.cursor_col >= self.state.cols {
            self.state.cursor_col = 0;
            self.newline();
        }
    }

    fn newline(&mut self) {
        if self.state.cursor_row + 1 < self.state.rows {
            self.state.cursor_row += 1;
        } else {
            self.state.lines.remove(0);
            self.state.lines.push(String::new());
        }
    }

    fn carriage_return(&mut self) {
        self.state.cursor_col = 0;
    }
}

impl<'a> Perform for TerminalPerformer<'a> {
    fn print(&mut self, c: char) {
        self.put_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' => {
                self.newline();
            }
            b'\r' => {
                self.carriage_return();
            }
            b'\t' => {
                let next = ((self.state.cursor_col / 8) + 1) * 8;
                self.state.cursor_col = next.min(self.state.cols.saturating_sub(1));
            }
            0x08 => {
                self.state.cursor_col = self.state.cursor_col.saturating_sub(1);
            }
            _ => {}
        }
    }

    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _c: char) {}
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}
    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {}
    fn csi_dispatch(&mut self, params: &Params, _intermediates: &[u8], _ignore: bool, c: char) {
        let p: Vec<usize> = params
            .iter()
            .map(|g| g.first().copied().unwrap_or(0) as usize)
            .collect();
        match c {
            'A' => {
                let n = p.first().copied().unwrap_or(1).max(1);
                self.state.cursor_row = self.state.cursor_row.saturating_sub(n);
            }
            'B' => {
                let n = p.first().copied().unwrap_or(1).max(1);
                self.state.cursor_row =
                    (self.state.cursor_row + n).min(self.state.rows.saturating_sub(1));
            }
            'C' => {
                let n = p.first().copied().unwrap_or(1).max(1);
                self.state.cursor_col =
                    (self.state.cursor_col + n).min(self.state.cols.saturating_sub(1));
            }
            'D' => {
                let n = p.first().copied().unwrap_or(1).max(1);
                self.state.cursor_col = self.state.cursor_col.saturating_sub(n);
            }
            'H' | 'f' => {
                let r = p.first().copied().unwrap_or(1).max(1) - 1;
                let ccol = p.get(1).copied().unwrap_or(1).max(1) - 1;
                self.state.cursor_row = r.min(self.state.rows.saturating_sub(1));
                self.state.cursor_col = ccol.min(self.state.cols.saturating_sub(1));
            }
            'J' => {
                let mode = p.first().copied().unwrap_or(0);
                if mode == 2 {
                    for l in &mut self.state.lines {
                        l.clear();
                    }
                    self.state.cursor_row = 0;
                    self.state.cursor_col = 0;
                }
            }
            'K' => {
                let mode = p.first().copied().unwrap_or(0);
                let row = self.state.cursor_row.min(self.state.rows.saturating_sub(1));
                let mut chars: Vec<char> = self.state.lines[row].chars().collect();
                if chars.len() < self.state.cols {
                    chars.resize(self.state.cols, ' ');
                }
                match mode {
                    0 => {
                        for i in self.state.cursor_col..chars.len() {
                            chars[i] = ' ';
                        }
                    }
                    1 => {
                        for i in 0..=self.state.cursor_col.min(chars.len().saturating_sub(1)) {
                            chars[i] = ' ';
                        }
                    }
                    2 => {
                        for c in &mut chars {
                            *c = ' ';
                        }
                    }
                    _ => {}
                }
                self.state.lines[row] = chars
                    .into_iter()
                    .collect::<String>()
                    .trim_end_matches(' ')
                    .to_string();
            }
            _ => {}
        }
    }
    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_text_and_newlines() {
        let mut eng = TerminalEngine::new(4, 10);
        eng.feed(b"hi\r\nthere");
        let json = String::from_utf8(eng.state.diff_json()).unwrap();
        assert!(json.contains("hi"));
        assert!(json.contains("there"));
        assert_eq!(eng.state.cursor_row, 1);
    }

    #[test]
    fn handles_csi_cursor_and_clear() {
        let mut eng = TerminalEngine::new(3, 5);
        eng.feed(b"abc\x1b[2J\x1b[Hxyz");
        assert!(eng.state.lines[0].starts_with("xyz"));
    }
}
