use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::ffi::CString;
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::ptr;
use std::rc::Rc;
use std::time::Duration;

use gtk4::gdk;
use gtk4::glib;
use gtk4::pango;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, EventControllerKey, GestureClick, Label, Orientation, PolicyType,
    PropagationPhase, ScrolledWindow, Stack, TextBuffer, TextTag, TextView, WrapMode,
};

const MAX_SCROLLBACK_LINES: usize = 1500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CellStyle {
    pub fg: Option<(u8, u8, u8)>,
    pub bg: Option<(u8, u8, u8)>,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermCell {
    pub ch: char,
    pub style: CellStyle,
}

impl Default for TermCell {
    fn default() -> Self {
        Self {
            ch: ' ',
            style: CellStyle::default(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TermLine {
    pub cells: Vec<TermCell>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VtState {
    Ground,
    Esc,
    EscCharset,
    Csi,
    Osc,
    OscEsc,
    StrIgnore,
    StrIgnoreEsc,
}

pub struct VtScreen {
    pub lines: VecDeque<TermLine>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub style: CellStyle,
    state: VtState,
    csi_buf: String,
    utf8_carry: Vec<u8>,
    viewport_base_row: usize,
}

impl Default for VtScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl VtScreen {
    pub fn new() -> Self {
        let mut lines = VecDeque::with_capacity(256);
        lines.push_back(TermLine::default());
        Self {
            lines,
            cursor_row: 0,
            cursor_col: 0,
            style: CellStyle::default(),
            state: VtState::Ground,
            csi_buf: String::with_capacity(32),
            utf8_carry: Vec::with_capacity(4),
            viewport_base_row: 0,
        }
    }

    pub fn clear_all(&mut self) {
        self.lines.clear();
        self.lines.push_back(TermLine::default());
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.viewport_base_row = 0;
    }

    /// Processes incoming PTY bytes and returns any terminal response bytes (e.g. DSR / DA1 replies)
    /// that must be written back to `master_fd`.
    pub fn feed_bytes(&mut self, incoming: &[u8]) -> Vec<u8> {
        let mut replies = Vec::new();

        let combined_storage: Vec<u8>;
        let data: &[u8] = if self.utf8_carry.is_empty() {
            incoming
        } else {
            let mut tmp = std::mem::take(&mut self.utf8_carry);
            tmp.extend_from_slice(incoming);
            combined_storage = tmp;
            &combined_storage
        };

        let mut offset = 0usize;
        while offset < data.len() {
            match std::str::from_utf8(&data[offset..]) {
                Ok(valid_str) => {
                    for ch in valid_str.chars() {
                        self.feed_char(ch, &mut replies);
                    }
                    break;
                }
                Err(err) => {
                    let valid_up_to = err.valid_up_to();
                    if valid_up_to > 0 {
                        if let Ok(valid_prefix) =
                            std::str::from_utf8(&data[offset..offset + valid_up_to])
                        {
                            for ch in valid_prefix.chars() {
                                self.feed_char(ch, &mut replies);
                            }
                        }
                        offset += valid_up_to;
                    }

                    if let Some(invalid_len) = err.error_len() {
                        self.feed_char('\u{FFFD}', &mut replies);
                        offset += invalid_len;
                    } else {
                        // Incomplete trailing UTF-8 sequence at end of read chunk
                        self.utf8_carry.extend_from_slice(&data[offset..]);
                        break;
                    }
                }
            }
        }

        self.enforce_scrollback_limit();
        replies
    }

    fn ensure_row_exists(&mut self, row: usize) {
        while self.lines.len() <= row {
            self.lines.push_back(TermLine::default());
        }
        if self.lines.len() > 28 {
            self.viewport_base_row = self.viewport_base_row.max(self.lines.len() - 28);
        }
    }

    fn enforce_scrollback_limit(&mut self) {
        while self.lines.len() > MAX_SCROLLBACK_LINES {
            self.lines.pop_front();
            self.cursor_row = self.cursor_row.saturating_sub(1);
            self.viewport_base_row = self.viewport_base_row.saturating_sub(1);
        }
    }

    fn put_char(&mut self, ch: char) {
        self.ensure_row_exists(self.cursor_row);
        let line = &mut self.lines[self.cursor_row];
        if self.cursor_col > line.cells.len() {
            line.cells.resize(self.cursor_col, TermCell::default());
        }
        let cell = TermCell {
            ch,
            style: self.style,
        };
        if self.cursor_col == line.cells.len() {
            line.cells.push(cell);
        } else {
            line.cells[self.cursor_col] = cell;
        }
        self.cursor_col += 1;
    }

    fn feed_char(&mut self, ch: char, replies: &mut Vec<u8>) {
        match self.state {
            VtState::Ground => match ch {
                '\x1b' => {
                    self.state = VtState::Esc;
                }
                '\r' => {
                    self.cursor_col = 0;
                }
                '\n' | '\x0b' | '\x0c' => {
                    self.cursor_row += 1;
                    self.ensure_row_exists(self.cursor_row);
                }
                '\x08' => {
                    self.cursor_col = self.cursor_col.saturating_sub(1);
                }
                '\t' => {
                    let spaces = 8 - (self.cursor_col % 8);
                    for _ in 0..spaces {
                        self.put_char(' ');
                    }
                }
                '\x07' | '\x00' => {}
                c if c.is_control() => {}
                c => {
                    self.put_char(c);
                }
            },

            VtState::Esc => match ch {
                '[' => {
                    self.csi_buf.clear();
                    self.state = VtState::Csi;
                }
                ']' => {
                    self.state = VtState::Osc;
                }
                'P' | 'X' | '^' | '_' => {
                    self.state = VtState::StrIgnore;
                }
                '(' | ')' | '*' | '+' => {
                    self.state = VtState::EscCharset;
                }
                'c' => {
                    self.clear_all();
                    self.style = CellStyle::default();
                    self.state = VtState::Ground;
                }
                'M' => {
                    self.cursor_row = self.cursor_row.saturating_sub(1);
                    self.state = VtState::Ground;
                }
                _ => {
                    // Includes ESC =, ESC >, ESC 7, ESC 8, etc.
                    self.state = VtState::Ground;
                }
            },

            VtState::EscCharset => {
                self.state = VtState::Ground;
            }

            VtState::Osc => match ch {
                '\x07' => self.state = VtState::Ground,
                '\x1b' => self.state = VtState::OscEsc,
                _ => {}
            },

            VtState::OscEsc => {
                if ch == '\\' {
                    self.state = VtState::Ground;
                } else {
                    self.state = VtState::Ground;
                    self.feed_char(ch, replies);
                }
            }

            VtState::StrIgnore => match ch {
                '\x07' => self.state = VtState::Ground,
                '\x1b' => self.state = VtState::StrIgnoreEsc,
                _ => {}
            },

            VtState::StrIgnoreEsc => {
                self.state = VtState::Ground;
                if ch != '\\' {
                    self.feed_char(ch, replies);
                }
            }

            VtState::Csi => {
                let code = ch as u32;
                if (0x20..=0x3F).contains(&code) {
                    if self.csi_buf.len() < 128 {
                        self.csi_buf.push(ch);
                    }
                } else if (0x40..=0x7E).contains(&code) {
                    let params = self.csi_buf.clone();
                    self.state = VtState::Ground;
                    self.handle_csi(&params, ch, replies);
                } else if ch == '\x1b' {
                    self.state = VtState::Esc;
                } else {
                    self.state = VtState::Ground;
                }
            }
        }
    }

    fn handle_csi(&mut self, raw_params: &str, final_ch: char, replies: &mut Vec<u8>) {
        let is_private = raw_params.starts_with(['?', '>', '=']);
        if is_private {
            // Private DEC / Kitty / xterm mode sequences (?2004h, ?1049h/l, ?u, >0q, >4;1m)
            return;
        }

        match final_ch {
            'm' => self.apply_sgr(raw_params),

            'A' => {
                let n = parse_first_num(raw_params, 1);
                self.cursor_row = self.cursor_row.saturating_sub(n);
            }
            'B' => {
                let n = parse_first_num(raw_params, 1);
                self.cursor_row += n;
                self.ensure_row_exists(self.cursor_row);
            }
            'C' => {
                let n = parse_first_num(raw_params, 1);
                self.cursor_col += n;
            }
            'D' => {
                let n = parse_first_num(raw_params, 1);
                self.cursor_col = self.cursor_col.saturating_sub(n);
            }
            'E' => {
                let n = parse_first_num(raw_params, 1);
                self.cursor_row += n;
                self.cursor_col = 0;
                self.ensure_row_exists(self.cursor_row);
            }
            'F' => {
                let n = parse_first_num(raw_params, 1);
                self.cursor_row = self.cursor_row.saturating_sub(n);
                self.cursor_col = 0;
            }
            'G' => {
                let col = parse_first_num(raw_params, 1);
                self.cursor_col = col.saturating_sub(1);
            }
            'H' | 'f' => {
                let mut parts = raw_params.split(';');
                let r = parts
                    .next()
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(1)
                    .max(1);
                let c = parts
                    .next()
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(1)
                    .max(1);
                self.cursor_row = self.viewport_base_row + (r - 1);
                self.cursor_col = c - 1;
                self.ensure_row_exists(self.cursor_row);
            }
            'K' => {
                let mode = parse_first_num(raw_params, 0);
                self.ensure_row_exists(self.cursor_row);
                let line = &mut self.lines[self.cursor_row];
                match mode {
                    0 => {
                        if self.cursor_col < line.cells.len() {
                            line.cells.truncate(self.cursor_col);
                        }
                    }
                    1 => {
                        let end = self.cursor_col.min(line.cells.len().saturating_sub(1));
                        for cell in line.cells.iter_mut().take(end + 1) {
                            *cell = TermCell::default();
                        }
                    }
                    2 => {
                        line.cells.clear();
                    }
                    _ => {}
                }
            }
            'J' => {
                let mode = parse_first_num(raw_params, 0);
                match mode {
                    0 => {
                        self.ensure_row_exists(self.cursor_row);
                        if self.cursor_col < self.lines[self.cursor_row].cells.len() {
                            self.lines[self.cursor_row].cells.truncate(self.cursor_col);
                        }
                        self.lines.truncate(self.cursor_row + 1);
                    }
                    2 | 3 => {
                        self.clear_all();
                    }
                    _ => {}
                }
            }
            'P' => {
                // Delete N characters at cursor
                let n = parse_first_num(raw_params, 1);
                self.ensure_row_exists(self.cursor_row);
                let line = &mut self.lines[self.cursor_row];
                if self.cursor_col < line.cells.len() {
                    let end = (self.cursor_col + n).min(line.cells.len());
                    line.cells.drain(self.cursor_col..end);
                }
            }
            '@' => {
                // Insert N blank characters at cursor
                let n = parse_first_num(raw_params, 1);
                self.ensure_row_exists(self.cursor_row);
                let line = &mut self.lines[self.cursor_row];
                if self.cursor_col <= line.cells.len() {
                    line.cells.splice(
                        self.cursor_col..self.cursor_col,
                        std::iter::repeat_n(TermCell::default(), n),
                    );
                }
            }
            'X' => {
                // Erase N characters at cursor
                let n = parse_first_num(raw_params, 1);
                self.ensure_row_exists(self.cursor_row);
                let line = &mut self.lines[self.cursor_row];
                for col in self.cursor_col..(self.cursor_col + n) {
                    if col < line.cells.len() {
                        line.cells[col] = TermCell::default();
                    }
                }
            }
            'n' => {
                let code = parse_first_num(raw_params, 0);
                if code == 6 {
                    let rel_row = self.cursor_row.saturating_sub(self.viewport_base_row) + 1;
                    let col = self.cursor_col + 1;
                    replies.extend_from_slice(format!("\x1b[{rel_row};{col}R").as_bytes());
                } else if code == 5 {
                    replies.extend_from_slice(b"\x1b[0n");
                }
            }
            'c' => {
                // Primary Device Attributes (DA1) response — unlocks instant fish 4.x startup!
                replies.extend_from_slice(b"\x1b[?62;22c");
            }
            _ => {}
        }
    }

    fn apply_sgr(&mut self, raw_params: &str) {
        if raw_params.is_empty() {
            self.style = CellStyle::default();
            return;
        }
        // Normalize ':' subparameters (e.g. 38:2:R:G:B) to ';'
        let normalized = raw_params.replace(':', ";");
        let nums: Vec<u16> = normalized
            .split(';')
            .map(|s| s.parse::<u16>().unwrap_or(0))
            .collect();

        let mut i = 0usize;
        while i < nums.len() {
            match nums[i] {
                0 => self.style = CellStyle::default(),
                1 => self.style.bold = true,
                3 => self.style.italic = true,
                22 => self.style.bold = false,
                23 => self.style.italic = false,
                30..=37 => self.style.fg = Some(ansi16_rgb((nums[i] - 30) as u8)),
                39 => self.style.fg = None,
                40..=47 => self.style.bg = Some(ansi16_rgb((nums[i] - 40) as u8)),
                49 => self.style.bg = None,
                90..=97 => self.style.fg = Some(ansi16_rgb((nums[i] - 90 + 8) as u8)),
                100..=107 => self.style.bg = Some(ansi16_rgb((nums[i] - 100 + 8) as u8)),
                38 | 48 => {
                    let is_fg = nums[i] == 38;
                    if i + 2 < nums.len() && nums[i + 1] == 5 {
                        let rgb = ansi256_rgb(nums[i + 2] as u8);
                        if is_fg {
                            self.style.fg = Some(rgb);
                        } else {
                            self.style.bg = Some(rgb);
                        }
                        i += 3;
                        continue;
                    } else if i + 4 < nums.len() && nums[i + 1] == 2 {
                        let rgb = (nums[i + 2] as u8, nums[i + 3] as u8, nums[i + 4] as u8);
                        if is_fg {
                            self.style.fg = Some(rgb);
                        } else {
                            self.style.bg = Some(rgb);
                        }
                        i += 5;
                        continue;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
}

fn parse_first_num(raw: &str, default: usize) -> usize {
    raw.split(';')
        .next()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .map(|v| if v == 0 && default > 0 { default } else { v })
        .unwrap_or(default)
}

/// VS Code Dark Modern 16-color terminal palette
fn ansi16_rgb(idx: u8) -> (u8, u8, u8) {
    match idx {
        0 => (30, 30, 30),      // Black
        1 => (205, 49, 49),     // Red
        2 => (13, 188, 121),    // Green
        3 => (229, 229, 16),    // Yellow
        4 => (36, 114, 200),    // Blue
        5 => (188, 63, 188),    // Magenta
        6 => (17, 168, 205),    // Cyan
        7 => (229, 229, 229),   // White
        8 => (102, 102, 102),   // Bright Black
        9 => (241, 76, 76),     // Bright Red
        10 => (35, 209, 139),   // Bright Green
        11 => (245, 245, 67),   // Bright Yellow
        12 => (59, 142, 234),   // Bright Blue
        13 => (214, 112, 214),  // Bright Magenta
        14 => (41, 184, 219),   // Bright Cyan
        _ => (255, 255, 255),   // Bright White
    }
}

fn ansi256_rgb(code: u8) -> (u8, u8, u8) {
    if code < 16 {
        return ansi16_rgb(code);
    }
    if code < 232 {
        let idx = code - 16;
        let r = idx / 36;
        let g = (idx % 36) / 6;
        let b = idx % 6;
        let scale = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
        return (scale(r), scale(g), scale(b));
    }
    let gray = 8 + (code - 232) * 10;
    (gray, gray, gray)
}

struct PtyProcess {
    master_fd: RawFd,
    child: Child,
    fd_source: Option<glib::SourceId>,
}

impl Drop for PtyProcess {
    fn drop(&mut self) {
        if let Some(src) = self.fd_source.take() {
            src.remove();
        }
        unsafe {
            libc::close(self.master_fd);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct TerminalSession {
    root: ScrolledWindow,
    text_view: TextView,
    buffer: TextBuffer,
    pty: RefCell<Option<PtyProcess>>,
    screen: RefCell<VtScreen>,
    style_tags: RefCell<HashMap<CellStyle, String>>,
    working_dir: RefCell<Option<PathBuf>>,
    shell_state: RefCell<String>,
    render_scheduled: Cell<bool>,
    last_cols_rows: Cell<(u16, u16)>,
    on_state_change: Rc<dyn Fn()>,
    on_cycle_tab: Rc<dyn Fn(bool)>,
}

impl TerminalSession {
    fn new(
        working_dir: Option<PathBuf>,
        on_state_change: Rc<dyn Fn()>,
        on_cycle_tab: Rc<dyn Fn(bool)>,
    ) -> Rc<Self> {
        let buffer = TextBuffer::new(None);
        let text_view = TextView::builder()
            .buffer(&buffer)
            .editable(false)
            .cursor_visible(true)
            .monospace(true)
            .wrap_mode(WrapMode::Char)
            .left_margin(8)
            .right_margin(8)
            .top_margin(6)
            .bottom_margin(10)
            .build();
        text_view.add_css_class("terminal-view");
        text_view.set_hexpand(true);
        text_view.set_vexpand(true);

        let root = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vscrollbar_policy(PolicyType::Automatic)
            .hexpand(true)
            .vexpand(true)
            .child(&text_view)
            .build();

        let session = Rc::new(Self {
            root,
            text_view,
            buffer,
            pty: RefCell::new(None),
            screen: RefCell::new(VtScreen::new()),
            style_tags: RefCell::new(HashMap::new()),
            working_dir: RefCell::new(working_dir),
            shell_state: RefCell::new("idle".to_string()),
            render_scheduled: Cell::new(false),
            last_cols_rows: Cell::new((80, 32)),
            on_state_change,
            on_cycle_tab,
        });

        Self::wire_input(&session);
        Self::wire_resize_tracking(&session);
        session.spawn_shell();

        session
    }

    fn clear(&self) {
        self.screen.borrow_mut().clear_all();
        self.render_now();
        self.write_pty(b"\x0c");
    }

    fn restart(self: &Rc<Self>, dir: Option<PathBuf>) {
        self.kill();
        if let Some(d) = dir {
            *self.working_dir.borrow_mut() = Some(d);
        }
        self.screen.borrow_mut().clear_all();
        self.render_now();
        self.spawn_shell();
    }

    fn kill(&self) {
        *self.pty.borrow_mut() = None;
        self.screen.borrow_mut().clear_all();
        self.buffer.set_text("");
        *self.shell_state.borrow_mut() = "stopped".to_string();
    }

    fn spawn_shell(self: &Rc<Self>) {
        let shell_path = std::env::var("SHELL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "/bin/bash".to_string());

        let shell_name = Path::new(&shell_path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("sh")
            .to_string();

        let (cols, rows) = self.estimate_cols_rows();
        let ws = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };

        let mut master_fd: RawFd = -1;
        let mut slave_fd: RawFd = -1;
        let rc = unsafe {
            libc::openpty(
                &mut master_fd,
                &mut slave_fd,
                ptr::null_mut(),
                ptr::null_mut(),
                &ws,
            )
        };
        if rc != 0 || master_fd < 0 || slave_fd < 0 {
            self.buffer.set_text("Failed to allocate Linux PTY.");
            *self.shell_state.borrow_mut() = "error".to_string();
            (self.on_state_change)();
            return;
        }

        unsafe {
            let flags = libc::fcntl(master_fd, libc::F_GETFL);
            if flags >= 0 {
                libc::fcntl(master_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
            libc::fcntl(master_fd, libc::F_SETFD, libc::FD_CLOEXEC);
        }

        let slave_in = unsafe { libc::dup(slave_fd) };
        let slave_out = unsafe { libc::dup(slave_fd) };
        let slave_err = slave_fd;

        let mut cmd = Command::new(&shell_path);
        cmd.arg("-i");
        if let Some(dir) = self.working_dir.borrow().clone() {
            if dir.is_dir() {
                cmd.current_dir(dir);
            }
        }
        cmd.env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .stdin(unsafe { Stdio::from_raw_fd(slave_in) })
            .stdout(unsafe { Stdio::from_raw_fd(slave_out) })
            .stderr(unsafe { Stdio::from_raw_fd(slave_err) });

        unsafe {
            cmd.pre_exec(move || {
                libc::setsid();
                libc::ioctl(0, libc::TIOCSCTTY, 0);
                // Ensure UTF-8 input handling on the slave PTY
                let mut tio: libc::termios = std::mem::zeroed();
                if libc::tcgetattr(0, &mut tio) == 0 {
                    tio.c_iflag |= libc::IUTF8;
                    libc::tcsetattr(0, libc::TCSANOW, &tio);
                }
                Ok(())
            });
        }

        // Write CString drop safety note: std::process::Command handles execvp safely
        let _ = CString::new(shell_path.as_str());

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(err) => {
                unsafe {
                    libc::close(master_fd);
                }
                self.buffer
                    .set_text(&format!("Failed to launch shell '{shell_path}': {err}"));
                *self.shell_state.borrow_mut() = "error".to_string();
                (self.on_state_change)();
                return;
            }
        };

        let term_weak = Rc::downgrade(self);
        let source_id = glib::unix_fd_add_local(
            master_fd,
            glib::IOCondition::IN | glib::IOCondition::HUP | glib::IOCondition::ERR,
            move |fd, cond| {
                let Some(term) = term_weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };

                let mut buf = [0u8; 8192];
                let mut read_any = false;
                let mut eof = false;

                loop {
                    let n = unsafe {
                        libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len())
                    };
                    if n > 0 {
                        read_any = true;
                        let replies = term.screen.borrow_mut().feed_bytes(&buf[..n as usize]);
                        if !replies.is_empty() {
                            term.write_pty(&replies);
                        }
                    } else if n == 0 {
                        eof = true;
                        break;
                    } else {
                        let err = std::io::Error::last_os_error();
                        if err.raw_os_error() == Some(libc::EAGAIN)
                            || err.raw_os_error() == Some(libc::EWOULDBLOCK)
                        {
                            break;
                        }
                        // EIO on Linux PTY master indicates slave closed (shell exited)
                        eof = true;
                        break;
                    }
                }

                if read_any {
                    term.schedule_render();
                }

                if eof
                    || cond.intersects(glib::IOCondition::HUP | glib::IOCondition::ERR)
                        && !read_any
                {
                    let _ = term.screen.borrow_mut().feed_bytes(
                        b"\r\n\x1b[90m[Shell exited - press Enter to restart]\x1b[0m\r\n",
                    );
                    term.render_now();
                    *term.shell_state.borrow_mut() = "exited".to_string();
                    if let Some(mut p) = term.pty.borrow_mut().take() {
                        p.fd_source = None;
                        glib::idle_add_local_once(move || drop(p));
                    }
                    (term.on_state_change)();
                    return glib::ControlFlow::Break;
                }

                glib::ControlFlow::Continue
            },
        );

        *self.pty.borrow_mut() = Some(PtyProcess {
            master_fd,
            child,
            fd_source: Some(source_id),
        });
        *self.shell_state.borrow_mut() = shell_name;
        (self.on_state_change)();
    }

    fn write_pty(&self, bytes: &[u8]) {
        if let Some(pty) = self.pty.borrow().as_ref() {
            let mut written = 0usize;
            while written < bytes.len() {
                let n = unsafe {
                    libc::write(
                        pty.master_fd,
                        bytes[written..].as_ptr() as *const libc::c_void,
                        bytes.len() - written,
                    )
                };
                if n > 0 {
                    written += n as usize;
                } else {
                    break;
                }
            }
        }
    }

    fn schedule_render(self: &Rc<Self>) {
        if self.render_scheduled.get() {
            return;
        }
        self.render_scheduled.set(true);
        let t_weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(12), move || {
            if let Some(t) = t_weak.upgrade() {
                t.render_scheduled.set(false);
                t.render_now();
            }
        });
    }

    fn render_now(&self) {
        let screen = self.screen.borrow();

        // Build plain text + style spans in character coordinates
        let mut full_text = String::with_capacity(screen.lines.len() * 48);
        let mut spans: Vec<(usize, usize, CellStyle)> = Vec::new();
        let mut char_pos = 0usize;
        let mut cursor_char_offset: Option<usize> = None;

        let default_style = CellStyle::default();

        for (row_idx, line) in screen.lines.iter().enumerate() {
            if row_idx > 0 {
                full_text.push('\n');
                char_pos += 1;
            }

            let line_start_char = char_pos;
            let mut run_style = default_style;
            let mut run_start = char_pos;

            let max_col = if row_idx == screen.cursor_row {
                line.cells.len().max(screen.cursor_col)
            } else {
                line.cells.len()
            };

            for col_idx in 0..max_col {
                if row_idx == screen.cursor_row && col_idx == screen.cursor_col {
                    cursor_char_offset = Some(char_pos);
                }
                let cell = line.cells.get(col_idx).copied().unwrap_or_default();
                if cell.style != run_style {
                    if run_style != default_style && run_start < char_pos {
                        spans.push((run_start, char_pos, run_style));
                    }
                    run_style = cell.style;
                    run_start = char_pos;
                }
                full_text.push(cell.ch);
                char_pos += 1;
            }

            if row_idx == screen.cursor_row && cursor_char_offset.is_none() {
                cursor_char_offset = Some(line_start_char + screen.cursor_col.min(max_col));
            }

            if run_style != default_style && run_start < char_pos {
                spans.push((run_start, char_pos, run_style));
            }
        }

        let final_cursor = cursor_char_offset.unwrap_or(char_pos);
        drop(screen);

        self.buffer.set_text(&full_text);

        for (start, end, style) in spans {
            let tag_name = self.ensure_style_tag(style);
            let s_iter = self.buffer.iter_at_offset(start as i32);
            let e_iter = self.buffer.iter_at_offset(end as i32);
            self.buffer.apply_tag_by_name(&tag_name, &s_iter, &e_iter);
        }

        let cur_iter = self.buffer.iter_at_offset(final_cursor as i32);
        self.buffer.place_cursor(&cur_iter);

        // Scroll to bottom smoothly
        let mut end_iter = self.buffer.end_iter();
        self.text_view
            .scroll_to_iter(&mut end_iter, 0.0, false, 0.0, 1.0);
    }

    fn ensure_style_tag(&self, style: CellStyle) -> String {
        if let Some(existing) = self.style_tags.borrow().get(&style) {
            return existing.clone();
        }

        // Prevent unbounded tag table growth in extreme rainbow programs
        if self.style_tags.borrow().len() > 512 {
            self.style_tags.borrow_mut().clear();
        }

        let name = format!(
            "vt_{}_{}_{}_{}",
            style
                .fg
                .map(|(r, g, b)| format!("{r:02x}{g:02x}{b:02x}"))
                .unwrap_or_else(|| "def".into()),
            style
                .bg
                .map(|(r, g, b)| format!("{r:02x}{g:02x}{b:02x}"))
                .unwrap_or_else(|| "def".into()),
            u8::from(style.bold),
            u8::from(style.italic),
        );

        let table = self.buffer.tag_table();
        if table.lookup(&name).is_none() {
            let mut builder = TextTag::builder().name(&name);
            if let Some((r, g, b)) = style.fg {
                builder = builder.foreground(&format!("#{r:02x}{g:02x}{b:02x}"));
            }
            if let Some((r, g, b)) = style.bg {
                builder = builder.background(&format!("#{r:02x}{g:02x}{b:02x}"));
            }
            if style.bold {
                builder = builder.weight(700);
            }
            if style.italic {
                builder = builder.style(pango::Style::Italic);
            }
            table.add(&builder.build());
        }

        self.style_tags.borrow_mut().insert(style, name.clone());
        name
    }

    fn estimate_cols_rows(&self) -> (u16, u16) {
        let w = self.text_view.width().max(320);
        let h = self.text_view.height().max(480);
        let cols = ((w - 18) / 8).clamp(32, 220) as u16;
        let rows = ((h - 16) / 18).clamp(12, 120) as u16;
        (cols, rows)
    }

    fn wire_resize_tracking(term: &Rc<Self>) {
        let t_weak = Rc::downgrade(term);
        glib::timeout_add_local(Duration::from_millis(400), move || {
            let Some(t) = t_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if let Some(pty) = t.pty.borrow().as_ref() {
                let (cols, rows) = t.estimate_cols_rows();
                if (cols, rows) != t.last_cols_rows.get() {
                    t.last_cols_rows.set((cols, rows));
                    let ws = libc::winsize {
                        ws_row: rows,
                        ws_col: cols,
                        ws_xpixel: 0,
                        ws_ypixel: 0,
                    };
                    unsafe {
                        libc::ioctl(pty.master_fd, libc::TIOCSWINSZ, &ws);
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    fn wire_input(term: &Rc<Self>) {
        let key_ctrl = EventControllerKey::new();
        key_ctrl.set_propagation_phase(PropagationPhase::Capture);

        let t_weak = Rc::downgrade(term);
        key_ctrl.connect_key_pressed(move |_, key, _code, state| {
            let Some(t) = t_weak.upgrade() else {
                return glib::Propagation::Proceed;
            };

            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
            let alt = state.contains(gdk::ModifierType::ALT_MASK);

            // Allow global app shortcuts (Ctrl+`, Ctrl+Shift+`, Ctrl+B, Ctrl+S, Ctrl+Shift+I) to bubble up
            if ctrl && !alt {
                if matches!(
                    key,
                    gdk::Key::grave
                        | gdk::Key::asciitilde
                        | gdk::Key::b
                        | gdk::Key::B
                        | gdk::Key::s
                        | gdk::Key::S
                ) || (shift && matches!(key, gdk::Key::i | gdk::Key::I))
                {
                    return glib::Propagation::Proceed;
                }

                // Cycle between multiple terminal tabs via Ctrl+PageUp / Ctrl+PageDown
                if key == gdk::Key::Page_Up {
                    (t.on_cycle_tab)(true);
                    return glib::Propagation::Stop;
                }
                if key == gdk::Key::Page_Down {
                    (t.on_cycle_tab)(false);
                    return glib::Propagation::Stop;
                }
            }

            // Ctrl+Shift+C -> Copy selection to clipboard
            if ctrl && shift && matches!(key, gdk::Key::c | gdk::Key::C) {
                if let Some(display) = gdk::Display::default() {
                    t.buffer.copy_clipboard(&display.clipboard());
                }
                return glib::Propagation::Stop;
            }

            // Ctrl+Shift+V -> Paste clipboard text into PTY
            if ctrl && shift && matches!(key, gdk::Key::v | gdk::Key::V) {
                if let Some(display) = gdk::Display::default() {
                    let t_weak2 = Rc::downgrade(&t);
                    display.clipboard().read_text_async(
                        gtk4::gio::Cancellable::NONE,
                        move |res| {
                            if let (Ok(Some(txt)), Some(term2)) = (res, t_weak2.upgrade()) {
                                term2.write_pty(txt.as_bytes());
                            }
                        },
                    );
                }
                return glib::Propagation::Stop;
            }

            // If shell exited and user presses Enter, restart shell automatically!
            if t.pty.borrow().is_none()
                && (key == gdk::Key::Return || key == gdk::Key::KP_Enter)
            {
                t.restart(None);
                return glib::Propagation::Stop;
            }

            // Ctrl+<letter> terminal control characters (Ctrl+C, Ctrl+D, Ctrl+L, Ctrl+Z, Ctrl+R, Ctrl+U, Ctrl+W, Ctrl+A, Ctrl+E)
            if ctrl && !shift && !alt {
                if let Some(ch) = key.to_unicode() {
                    let lower = ch.to_ascii_lowercase();
                    if lower.is_ascii_lowercase() {
                        // Ctrl+C when text is selected copies; otherwise sends SIGINT (0x03)
                        if lower == 'c' && t.buffer.has_selection() {
                            if let Some(display) = gdk::Display::default() {
                                t.buffer.copy_clipboard(&display.clipboard());
                            }
                            return glib::Propagation::Stop;
                        }
                        let ctrl_byte = (lower as u8) - b'a' + 1;
                        t.write_pty(&[ctrl_byte]);
                        return glib::Propagation::Stop;
                    }
                }
            }

            // Special keys -> VT100/xterm sequences
            let seq: Option<&[u8]> = match key {
                gdk::Key::Return | gdk::Key::KP_Enter => Some(b"\r"),
                gdk::Key::BackSpace => Some(b"\x7f"),
                gdk::Key::Tab => Some(b"\t"),
                gdk::Key::ISO_Left_Tab => Some(b"\x1b[Z"),
                gdk::Key::Escape => Some(b"\x1b"),
                gdk::Key::Up => Some(b"\x1b[A"),
                gdk::Key::Down => Some(b"\x1b[B"),
                gdk::Key::Right => Some(b"\x1b[C"),
                gdk::Key::Left => Some(b"\x1b[D"),
                gdk::Key::Home => Some(b"\x1b[H"),
                gdk::Key::End => Some(b"\x1b[F"),
                gdk::Key::Delete => Some(b"\x1b[3~"),
                gdk::Key::Page_Up => Some(b"\x1b[5~"),
                gdk::Key::Page_Down => Some(b"\x1b[6~"),
                _ => None,
            };

            if let Some(bytes) = seq {
                t.write_pty(bytes);
                return glib::Propagation::Stop;
            }

            // Regular Unicode characters (including Alt+<key> ESC prefix)
            if let Some(ch) = key.to_unicode() {
                if !ch.is_control() {
                    let mut encoded = [0u8; 4];
                    let s = ch.encode_utf8(&mut encoded);
                    if alt {
                        let mut with_esc = Vec::with_capacity(1 + s.len());
                        with_esc.push(0x1b);
                        with_esc.extend_from_slice(s.as_bytes());
                        t.write_pty(&with_esc);
                    } else {
                        t.write_pty(s.as_bytes());
                    }
                    return glib::Propagation::Stop;
                }
            }

            glib::Propagation::Proceed
        });

        term.text_view.add_controller(key_ctrl);
    }
}

struct TermTabEntry {
    id: String,
    num: usize,
    session: Rc<TerminalSession>,
    tab_widget: GtkBox,
    title_label: Label,
}

pub struct TerminalSidebar {
    pub root: GtkBox,
    shell_label: Label,
    tab_strip: GtkBox,
    stack: Stack,
    tabs: RefCell<Vec<TermTabEntry>>,
    active_id: RefCell<Option<String>>,
    next_uid: Cell<usize>,
    next_display_num: Cell<usize>,
    working_dir: RefCell<Option<PathBuf>>,
    on_request_hide: Rc<dyn Fn()>,
}

impl TerminalSidebar {
    pub fn new(on_request_hide: Rc<dyn Fn()>) -> Rc<Self> {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("terminal-sidebar");
        root.set_width_request(340);
        root.set_vexpand(true);

        // Top Terminal Header Bar
        let header = GtkBox::new(Orientation::Horizontal, 5);
        header.add_css_class("terminal-header");

        let title = Label::new(Some("TERMINAL"));
        title.add_css_class("sidebar-title");
        title.set_halign(Align::Start);

        let shell_label = Label::new(Some("idle"));
        shell_label.add_css_class("terminal-shell-badge");
        shell_label.set_halign(Align::Start);
        shell_label.set_hexpand(true);

        let new_btn = Button::with_label("+ New");
        new_btn.add_css_class("terminal-new-btn");
        new_btn.set_tooltip_text(Some("Open New Terminal (Ctrl+Shift+`)"));

        let clear_btn = Button::with_label("Clear");
        clear_btn.add_css_class("sidebar-btn");
        clear_btn.set_tooltip_text(Some("Clear Active Terminal Output (Ctrl+L)"));

        let restart_btn = Button::with_label("↻");
        restart_btn.add_css_class("sidebar-btn");
        restart_btn.set_tooltip_text(Some("Restart Active Shell in Workspace Folder"));

        let kill_btn = Button::with_label("Kill");
        kill_btn.add_css_class("sidebar-btn");
        kill_btn.set_tooltip_text(Some("Kill Active Terminal & Close Tab"));

        let hide_btn = Button::with_label("✕");
        hide_btn.add_css_class("tab-close-btn");
        hide_btn.set_tooltip_text(Some("Hide Terminal Sidebar (Ctrl+`)"));

        header.append(&title);
        header.append(&shell_label);
        header.append(&new_btn);
        header.append(&clear_btn);
        header.append(&restart_btn);
        header.append(&kill_btn);
        header.append(&hide_btn);

        let tab_strip = GtkBox::new(Orientation::Horizontal, 0);
        tab_strip.add_css_class("term-tab-strip");

        let tab_strip_scroll = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vscrollbar_policy(PolicyType::Never)
            .hexpand(true)
            .child(&tab_strip)
            .build();
        tab_strip_scroll.add_css_class("term-tab-strip-scroll");

        let stack = Stack::new();
        stack.set_hexpand(true);
        stack.set_vexpand(true);

        root.append(&header);
        root.append(&tab_strip_scroll);
        root.append(&stack);

        let sidebar = Rc::new(Self {
            root,
            shell_label,
            tab_strip,
            stack,
            tabs: RefCell::new(Vec::new()),
            active_id: RefCell::new(None),
            next_uid: Cell::new(1),
            next_display_num: Cell::new(1),
            working_dir: RefCell::new(None),
            on_request_hide: on_request_hide.clone(),
        });

        {
            let sb_weak = Rc::downgrade(&sidebar);
            new_btn.connect_clicked(move |_| {
                if let Some(sb) = sb_weak.upgrade() {
                    sb.new_terminal();
                }
            });
        }
        {
            let sb_weak = Rc::downgrade(&sidebar);
            clear_btn.connect_clicked(move |_| {
                if let Some(sb) = sb_weak.upgrade() {
                    if let Some(sess) = sb.active_session() {
                        sess.clear();
                        sess.text_view.grab_focus();
                    }
                }
            });
        }
        {
            let sb_weak = Rc::downgrade(&sidebar);
            restart_btn.connect_clicked(move |_| {
                if let Some(sb) = sb_weak.upgrade() {
                    if let Some(sess) = sb.active_session() {
                        let dir = sb.working_dir.borrow().clone();
                        sess.restart(dir);
                        sess.text_view.grab_focus();
                    }
                }
            });
        }
        {
            let sb_weak = Rc::downgrade(&sidebar);
            kill_btn.connect_clicked(move |_| {
                if let Some(sb) = sb_weak.upgrade() {
                    let active = sb.active_id.borrow().clone();
                    if let Some(id) = active {
                        sb.close_terminal(&id);
                    }
                }
            });
        }
        {
            hide_btn.connect_clicked(move |_| {
                on_request_hide();
            });
        }

        sidebar
    }

    pub fn set_working_directory(&self, dir: PathBuf) {
        *self.working_dir.borrow_mut() = Some(dir);
    }

    /// Lazy-starts the first terminal tab only when needed
    pub fn ensure_started(self: &Rc<Self>) {
        if self.tabs.borrow().is_empty() {
            self.new_terminal();
        } else {
            self.focus_active();
        }
    }

    pub fn new_terminal(self: &Rc<Self>) -> usize {
        if self.tabs.borrow().is_empty() {
            self.next_display_num.set(1);
        }

        let uid = self.next_uid.get();
        self.next_uid.set(uid + 1);
        let num = self.next_display_num.get();
        self.next_display_num.set(num + 1);
        let tab_id = format!("term_{uid}");

        let sb_weak_state = Rc::downgrade(self);
        let on_state_change = Rc::new(move || {
            if let Some(sb) = sb_weak_state.upgrade() {
                sb.refresh_tab_labels();
            }
        });

        let sb_weak_cycle = Rc::downgrade(self);
        let on_cycle_tab = Rc::new(move |backwards: bool| {
            if let Some(sb) = sb_weak_cycle.upgrade() {
                sb.cycle_tab(backwards);
            }
        });

        let dir = self.working_dir.borrow().clone();
        let session = TerminalSession::new(dir, on_state_change, on_cycle_tab);

        let tab_widget = GtkBox::new(Orientation::Horizontal, 4);
        tab_widget.add_css_class("term-tab");

        let initial_state = session.shell_state.borrow().clone();
        let title_label = Label::new(Some(&format!("{num}: {initial_state}")));
        title_label.add_css_class("term-tab-title");

        let close_btn = Button::with_label("×");
        close_btn.add_css_class("tab-close-btn");
        close_btn.set_tooltip_text(Some("Close Terminal"));

        tab_widget.append(&title_label);
        tab_widget.append(&close_btn);

        // Left click activates tab; middle click closes tab
        {
            let click = GestureClick::new();
            click.set_button(0);
            let sb_weak = Rc::downgrade(self);
            let id_copy = tab_id.clone();
            click.connect_pressed(move |gesture, _, _, _| {
                if let Some(sb) = sb_weak.upgrade() {
                    if gesture.current_button() == 2 {
                        sb.close_terminal(&id_copy);
                    } else if gesture.current_button() == 1 {
                        sb.activate_tab(&id_copy);
                    }
                }
            });
            tab_widget.add_controller(click);
        }

        {
            let sb_weak = Rc::downgrade(self);
            let id_copy = tab_id.clone();
            close_btn.connect_clicked(move |_| {
                if let Some(sb) = sb_weak.upgrade() {
                    sb.close_terminal(&id_copy);
                }
            });
        }

        self.tab_strip.append(&tab_widget);
        self.stack.add_named(&session.root, Some(&tab_id));

        self.tabs.borrow_mut().push(TermTabEntry {
            id: tab_id.clone(),
            num,
            session,
            tab_widget,
            title_label,
        });

        self.activate_tab(&tab_id);
        self.refresh_tab_labels();
        num
    }

    pub fn activate_tab(self: &Rc<Self>, tab_id: &str) {
        *self.active_id.borrow_mut() = Some(tab_id.to_string());
        self.stack.set_visible_child_name(tab_id);

        let mut focus_view = None;
        for entry in self.tabs.borrow().iter() {
            if entry.id == tab_id {
                entry.tab_widget.add_css_class("term-tab-active");
                focus_view = Some(entry.session.text_view.clone());
            } else {
                entry.tab_widget.remove_css_class("term-tab-active");
            }
        }

        if let Some(tv) = focus_view {
            tv.grab_focus();
        }
    }

    pub fn close_terminal(self: &Rc<Self>, tab_id: &str) {
        let mut tabs = self.tabs.borrow_mut();
        let Some(idx) = tabs.iter().position(|t| t.id == tab_id) else {
            return;
        };

        let removed = tabs.remove(idx);
        let was_active = self.active_id.borrow().as_deref() == Some(tab_id);
        let remaining_next_id = if tabs.is_empty() {
            None
        } else if was_active {
            let next_idx = idx.min(tabs.len() - 1);
            Some(tabs[next_idx].id.clone())
        } else {
            None
        };
        drop(tabs);

        removed.session.kill();
        self.tab_strip.remove(&removed.tab_widget);
        self.stack.remove(&removed.session.root);

        if self.tabs.borrow().is_empty() {
            *self.active_id.borrow_mut() = None;
            self.shell_label.set_text("idle");
            (self.on_request_hide)();
        } else {
            if let Some(next_id) = remaining_next_id {
                self.activate_tab(&next_id);
            }
            self.refresh_tab_labels();
        }
    }

    pub fn cycle_tab(self: &Rc<Self>, backwards: bool) {
        let next_id = {
            let tabs = self.tabs.borrow();
            if tabs.len() <= 1 {
                return;
            }
            let cur_id = self.active_id.borrow().clone();
            let cur_idx = tabs
                .iter()
                .position(|t| Some(&t.id) == cur_id.as_ref())
                .unwrap_or(0);
            let next_idx = if backwards {
                if cur_idx == 0 {
                    tabs.len() - 1
                } else {
                    cur_idx - 1
                }
            } else {
                (cur_idx + 1) % tabs.len()
            };
            tabs[next_idx].id.clone()
        };
        self.activate_tab(&next_id);
    }

    pub fn focus_active(&self) {
        if let Some(sess) = self.active_session() {
            sess.text_view.grab_focus();
        }
    }

    pub fn has_terminal_focus(&self) -> bool {
        self.tabs
            .borrow()
            .iter()
            .any(|t| t.session.text_view.has_focus())
    }

    pub fn kill_all(&self) {
        let drained: Vec<TermTabEntry> = self.tabs.borrow_mut().drain(..).collect();
        *self.active_id.borrow_mut() = None;
        for entry in drained {
            entry.session.kill();
            self.tab_strip.remove(&entry.tab_widget);
            self.stack.remove(&entry.session.root);
        }
        self.shell_label.set_text("stopped");
    }

    fn active_session(&self) -> Option<Rc<TerminalSession>> {
        let active = self.active_id.borrow().clone()?;
        self.tabs
            .borrow()
            .iter()
            .find(|t| t.id == active)
            .map(|t| t.session.clone())
    }

    fn refresh_tab_labels(&self) {
        let tabs = self.tabs.borrow();
        if tabs.is_empty() {
            self.shell_label.set_text("idle");
            return;
        }
        let mut active_count = 0usize;
        for entry in tabs.iter() {
            let state = entry.session.shell_state.borrow().clone();
            entry.title_label.set_text(&format!("{}: {state}", entry.num));
            if entry.session.pty.borrow().is_some() {
                active_count += 1;
            }
        }
        if active_count > 0 {
            self.shell_label.set_text(&format!("{active_count} active"));
        } else {
            self.shell_label.set_text("exited");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fish_startup_sequences_and_replies_da1_and_dsr() {
        let mut vt = VtScreen::new();
        let sample = b"\x1b[?u\x1b[>0q\x1b]11;?\x1b\\\x1bP+q696e646e\x1b\\\x1b[0c\x1b[38;2;137;180;250mWelcome!\x1b[0m\x1b[6n";
        let replies = vt.feed_bytes(sample);

        // Should reply with DA1 (\x1b[?62;22c) and DSR (\x1b[1;9R)
        let reply_str = String::from_utf8_lossy(&replies);
        assert!(reply_str.contains("\x1b[?62;22c"));
        assert!(reply_str.contains("\x1b[1;9R"));

        let rendered: String = vt.lines[0].cells.iter().map(|c| c.ch).collect();
        assert_eq!(rendered, "Welcome!");
        assert_eq!(vt.lines[0].cells[0].style.fg, Some((137, 180, 250)));
    }

    #[test]
    fn handles_carriage_return_and_erase_in_line() {
        let mut vt = VtScreen::new();
        vt.feed_bytes(b"Downloading 10%\rDownloading 100%\x1b[K");
        let rendered: String = vt.lines[0].cells.iter().map(|c| c.ch).collect();
        assert_eq!(rendered, "Downloading 100%");
    }
}
