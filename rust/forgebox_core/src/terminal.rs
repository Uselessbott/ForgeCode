use vte::{Params, Parser, Perform};

#[derive(Clone, Debug)]
pub struct TerminalState {
    pub rows: usize,
    pub cols: usize,
    pub lines: Vec<String>,
    pub scrollback: Vec<String>,
    pub scrollback_limit: usize,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub fg: u32,
    pub bg: u32,
    pub bold: bool,
    pub underline: bool,
    pub inverse: bool,
}

impl TerminalState {
    pub fn new(rows: usize, cols: usize) -> Self {
        Self::with_scrollback(rows, cols, 10_000)
    }

    pub fn total_lines(&self) -> usize {
        self.scrollback.len() + self.lines.len()
    }

    pub fn with_scrollback(rows: usize, cols: usize, scrollback_limit: usize) -> Self {
        let rows = rows.max(1);
        let cols = cols.max(1);
        Self {
            rows,
            cols,
            lines: vec![String::new(); rows],
            scrollback: Vec::new(),
            scrollback_limit: scrollback_limit.max(rows),
            cursor_row: 0,
            cursor_col: 0,
            fg: 0xFFDDDDDD,
            bg: 0xFF111111,
            bold: false,
            underline: false,
            inverse: false,
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

    pub fn push_scrollback(&mut self, line: String) {
        self.scrollback.push(line);
        if self.scrollback.len() > self.scrollback_limit {
            let drop_n = self.scrollback.len() - self.scrollback_limit;
            self.scrollback.drain(0..drop_n);
        }
    }

    pub fn diff_json(&self) -> Vec<u8> {
        self.snapshot_json(self.scrollback.len().saturating_sub(self.rows))
    }

    pub fn snapshot_json(&self, scroll_top: usize) -> Vec<u8> {
        let mut all: Vec<&str> = self.scrollback.iter().map(|s| s.as_str()).collect();
        for l in &self.lines {
            all.push(l.as_str());
        }
        let max_top = all.len().saturating_sub(self.rows);
        let top = scroll_top.min(max_top);
        let mut out = String::from("{\"rows\":");
        out.push_str(&self.rows.to_string());
        out.push_str(",\"cols\":");
        out.push_str(&self.cols.to_string());
        out.push_str(",\"cursor\":[");
        out.push_str(&self.cursor_row.to_string());
        out.push(',');
        out.push_str(&self.cursor_col.to_string());
        out.push_str("],\"scrollTop\":");
        out.push_str(&top.to_string());
        out.push_str(",\"totalLines\":");
        out.push_str(&all.len().to_string());
        out.push_str(",\"fg\":");
        out.push_str(&self.fg.to_string());
        out.push_str(",\"bg\":");
        out.push_str(&self.bg.to_string());
        out.push_str(",\"bold\":");
        out.push_str(if self.bold { "true" } else { "false" });
        out.push_str(",\"underline\":");
        out.push_str(if self.underline { "true" } else { "false" });
        out.push_str(",\"inverse\":");
        out.push_str(if self.inverse { "true" } else { "false" });
        out.push_str(",\"lines\":[");
        for i in 0..self.rows {
            if i > 0 {
                out.push(',');
            }
            let line = all.get(top + i).copied().unwrap_or("");
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
        Self::with_scrollback(rows, cols, 10_000)
    }

    pub fn with_scrollback(rows: usize, cols: usize, scrollback_limit: usize) -> Self {
        Self {
            parser: Parser::new(),
            state: TerminalState::with_scrollback(rows, cols, scrollback_limit),
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
            let removed = self.state.lines.remove(0);
            self.state.push_scrollback(removed);
            self.state.lines.push(String::new());
            self.state.cursor_row = self.state.rows - 1;
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
                } else if mode == 0 {
                    for row in self.state.cursor_row..self.state.rows {
                        if row == self.state.cursor_row {
                            clear_line_state(&mut self.state, 0);
                        } else {
                            self.state.lines[row].clear();
                        }
                    }
                }
            }
            'K' => {
                let mode = p.first().copied().unwrap_or(0);
                clear_line_state(&mut self.state, mode);
            }
            'm' => apply_sgr_state(&mut self.state, &p),
            _ => {}
        }
    }
    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {}
}

fn clear_line_state(state: &mut TerminalState, mode: usize) {
    let row = state.cursor_row.min(state.rows.saturating_sub(1));
    let mut chars: Vec<char> = state.lines[row].chars().collect();
    if chars.len() < state.cols {
        chars.resize(state.cols, ' ');
    }
    match mode {
        0 => {
            for i in state.cursor_col..chars.len() {
                chars[i] = ' ';
            }
        }
        1 => {
            for i in 0..=state.cursor_col.min(chars.len().saturating_sub(1)) {
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
    state.lines[row] = chars
        .into_iter()
        .collect::<String>()
        .trim_end_matches(' ')
        .to_string();
}

fn apply_sgr_state(state: &mut TerminalState, params: &[usize]) {
    let p = if params.is_empty() {
        &[0usize][..]
    } else {
        params
    };
    let mut i = 0;
    while i < p.len() {
        match p[i] {
            0 => {
                state.fg = 0xFFDDDDDD;
                state.bg = 0xFF111111;
                state.bold = false;
                state.underline = false;
                state.inverse = false;
            }
            1 => state.bold = true,
            4 => state.underline = true,
            7 => state.inverse = true,
            22 => state.bold = false,
            24 => state.underline = false,
            27 => state.inverse = false,
            30..=37 => state.fg = ansi16(p[i] - 30, state.bold),
            39 => state.fg = 0xFFDDDDDD,
            40..=47 => state.bg = ansi16(p[i] - 40, false),
            49 => state.bg = 0xFF111111,
            90..=97 => state.fg = ansi16(p[i] - 90 + 8, false),
            100..=107 => state.bg = ansi16(p[i] - 100 + 8, false),
            38 if p.get(i + 1) == Some(&5) => {
                if let Some(n) = p.get(i + 2) {
                    state.fg = xterm256(*n as u8);
                }
                i += 2;
            }
            48 if p.get(i + 1) == Some(&5) => {
                if let Some(n) = p.get(i + 2) {
                    state.bg = xterm256(*n as u8);
                }
                i += 2;
            }
            38 if p.get(i + 1) == Some(&2) => {
                let r = *p.get(i + 2).unwrap_or(&255) as u32;
                let g = *p.get(i + 3).unwrap_or(&255) as u32;
                let b = *p.get(i + 4).unwrap_or(&255) as u32;
                state.fg = 0xFF000000 | (r << 16) | (g << 8) | b;
                i += 4;
            }
            48 if p.get(i + 1) == Some(&2) => {
                let r = *p.get(i + 2).unwrap_or(&0) as u32;
                let g = *p.get(i + 3).unwrap_or(&0) as u32;
                let b = *p.get(i + 4).unwrap_or(&0) as u32;
                state.bg = 0xFF000000 | (r << 16) | (g << 8) | b;
                i += 4;
            }
            _ => {}
        }
        i += 1;
    }
}

fn ansi16(idx: usize, bright: bool) -> u32 {
    let base = match idx {
        0 => 0xFF000000,
        1 => 0xFFCD3131,
        2 => 0xFF0DBC79,
        3 => 0xFFE5E510,
        4 => 0xFF2472C8,
        5 => 0xFFBC3FBC,
        6 => 0xFF11A8CD,
        7 => 0xFFE5E5E5,
        8 => 0xFF666666,
        9 => 0xFFF14C4C,
        10 => 0xFF23D18B,
        11 => 0xFFF5F543,
        12 => 0xFF3B8EEA,
        13 => 0xFFD670D6,
        14 => 0xFF29B8DB,
        _ => 0xFFFFFFFF,
    };
    if bright && idx < 8 {
        ansi16(idx + 8, false)
    } else {
        base
    }
}

fn xterm256(n: u8) -> u32 {
    if n < 16 {
        return ansi16(n as usize, false);
    }
    if n >= 232 {
        let c = 8 + (n - 232) * 10;
        return 0xFF000000 | ((c as u32) << 16) | ((c as u32) << 8) | c as u32;
    }
    let n = n - 16;
    let r = (n / 36) % 6;
    let g = (n / 6) % 6;
    let b = n % 6;
    let map = |v: u8| -> u32 {
        if v == 0 {
            0
        } else {
            55 + 40 * v as u32
        }
    };
    0xFF000000 | (map(r) << 16) | (map(g) << 8) | map(b)
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

    #[test]
    fn scrollback_ring_is_bounded() {
        let mut eng = TerminalEngine::new(2, 5);
        eng.state.scrollback_limit = 3;
        eng.feed(b"a\r\nb\r\nc\r\nd\r\ne\r\n");
        assert!(eng.state.scrollback.len() <= 3);
        assert!(eng.state.total_lines() >= 2);
    }

    #[test]
    fn sgr_sets_basic_colors() {
        let mut eng = TerminalEngine::new(2, 10);
        eng.feed(b"\x1b[31mR\x1b[0m");
        assert_eq!(eng.state.fg, 0xFFDDDDDD);
        assert!(eng.state.lines[0].starts_with('R'));
    }

    #[test]
    fn snapshot_json_includes_scrollback_fields() {
        let mut eng = TerminalEngine::new(2, 5);
        eng.feed(b"1\r\n2\r\n3\r\n");
        let json = String::from_utf8(eng.state.diff_json()).unwrap();
        assert!(json.contains("\"scrollTop\""));
        assert!(json.contains("\"totalLines\""));
    }
}
