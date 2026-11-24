use std::cell::{Cell, RefCell};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, DrawingArea, EventControllerKey, Orientation, PolicyType, PropagationPhase,
    ScrolledWindow, TextBuffer, TextIter, TextTag, TextView, WrapMode,
};

use crate::formatter::{format_code, FormatOptions, FormatOutcome};
use crate::language::Language;
use crate::syntax::{highlight_buffer, install_syntax_tags};

pub struct EditorTab {
    pub root: GtkBox,
    pub text_view: TextView,
    pub buffer: TextBuffer,
    pub scrolled: ScrolledWindow,
    pub gutter: DrawingArea,
    pub file_path: RefCell<Option<PathBuf>>,
    pub title: RefCell<String>,
    pub language: Cell<Language>,
    pub indent_size: Cell<usize>,
    pub dirty: Cell<bool>,
    suppress_dirty: Cell<bool>,
    highlight_scheduled: Cell<bool>,
}

impl EditorTab {
    pub fn new(
        initial_path: Option<PathBuf>,
        initial_title: String,
        initial_content: &str,
        on_state_change: Rc<dyn Fn()>,
    ) -> Rc<Self> {
        let lang = Language::from_path_and_content(initial_path.as_deref(), initial_content);
        let default_indent = lang.default_indent_size();

        let buffer = TextBuffer::new(None);
        install_syntax_tags(&buffer);

        // Search highlight tag
        let table = buffer.tag_table();
        if table.lookup("search_match").is_none() {
            let match_tag = TextTag::builder()
                .name("search_match")
                .background("#613214")
                .foreground("#ffffff")
                .build();
            table.add(&match_tag);
        }

        let text_view = TextView::builder()
            .buffer(&buffer)
            .monospace(true)
            .wrap_mode(WrapMode::None)
            .left_margin(12)
            .right_margin(24)
            .top_margin(8)
            .bottom_margin(32)
            . pixels_above_lines(1)
            .pixels_below_lines(1)
            .build();
        text_view.add_css_class("code-view");
        text_view.set_hexpand(true);
        text_view.set_vexpand(true);

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vscrollbar_policy(PolicyType::Automatic)
            .hexpand(true)
            .vexpand(true)
            .child(&text_view)
            .build();

        let gutter = DrawingArea::builder()
            .content_width(52)
            .vexpand(true)
            .build();
        gutter.add_css_class("line-gutter");

        let root = GtkBox::new(Orientation::Horizontal, 0);
        root.set_hexpand(true);
        root.set_vexpand(true);
        root.append(&gutter);
        root.append(&scrolled);

        let tab = Rc::new(Self {
            root,
            text_view,
            buffer,
            scrolled,
            gutter,
            file_path: RefCell::new(initial_path),
            title: RefCell::new(initial_title),
            language: Cell::new(lang),
            indent_size: Cell::new(default_indent),
            dirty: Cell::new(false),
            suppress_dirty: Cell::new(true),
            highlight_scheduled: Cell::new(false),
        });

        tab.buffer.set_text(initial_content);
        tab.buffer.place_cursor(&tab.buffer.start_iter());
        highlight_buffer(&tab.buffer, lang);
        tab.suppress_dirty.set(false);

        Self::setup_gutter_drawing(&tab);
        Self::setup_signals(&tab, on_state_change);
        Self::setup_smart_keys(&tab);

        tab
    }

    fn setup_gutter_drawing(tab: &Rc<Self>) {
        let tab_weak = Rc::downgrade(tab);
        tab.gutter.set_draw_func(move |area, cr, width, height| {
            let Some(tab) = tab_weak.upgrade() else {
                return;
            };

            // Gutter background (#1e1e1e matching VS Code editor canvas)
            cr.set_source_rgb(0.118, 0.118, 0.118);
            let _ = cr.paint();

            // Subtle right border separator (#2b2b2b)
            cr.set_source_rgb(0.169, 0.169, 0.169);
            cr.set_line_width(1.0);
            cr.move_to(width as f64 - 0.5, 0.0);
            cr.line_to(width as f64 - 0.5, height as f64);
            let _ = cr.stroke();

            let total_lines = tab.buffer.line_count().max(1);
            let digits = total_lines.to_string().len().max(3);
            let desired_width = (digits as i32 * 9) + 26;
            if (area.content_width() - desired_width).abs() >= 6 {
                area.set_content_width(desired_width);
            }

            let cursor_iter = tab.cursor_iter();
            let active_line = cursor_iter.line();

            let vis = tab.text_view.visible_rect();
            let top_buf_y = vis.y();
            let bottom_buf_y = vis.y() + vis.height();

            let (mut iter, _) = tab.text_view.line_at_y(top_buf_y);

            cr.select_font_face(
                "monospace",
                gtk4::cairo::FontSlant::Normal,
                gtk4::cairo::FontWeight::Normal,
            );
            cr.set_font_size(13.0);

            loop {
                let line_idx = iter.line();
                let (buf_y, line_h) = tab.text_view.line_yrange(&iter);
                if buf_y > bottom_buf_y {
                    break;
                }

                let (_, win_y) = tab.text_view.buffer_to_window_coords(
                    gtk4::TextWindowType::Widget,
                    0,
                    buf_y,
                );

                if line_idx == active_line {
                    cr.set_source_rgb(0.157, 0.157, 0.157);
                    cr.rectangle(0.0, win_y as f64, (width - 1) as f64, line_h as f64);
                    let _ = cr.fill();
                    // Active line number (#c6c6c6)
                    cr.set_source_rgb(0.776, 0.776, 0.776);
                } else {
                    // Inactive line number (#858585)
                    cr.set_source_rgb(0.522, 0.522, 0.522);
                }

                let label = format!("{}", line_idx + 1);
                if let Ok(extents) = cr.text_extents(&label) {
                    let x = (width as f64 - 12.0 - extents.width()).max(6.0);
                    let y = win_y as f64 + (line_h as f64 * 0.74);
                    cr.move_to(x, y);
                    let _ = cr.show_text(&label);
                }

                if !iter.forward_line() {
                    break;
                }
            }
        });
    }

    fn setup_signals(tab: &Rc<Self>, on_state_change: Rc<dyn Fn()>) {
        // Redraw gutter on vertical scroll
        {
            let gutter = tab.gutter.clone();
            let vadj = tab.scrolled.vadjustment();
            vadj.connect_value_changed(move |_| {
                gutter.queue_draw();
            });
            let gutter2 = tab.gutter.clone();
            vadj.connect_changed(move |_| {
                gutter2.queue_draw();
            });
        }

        // Buffer text edits: mark dirty + schedule syntax highlight + refresh status
        {
            let tab_weak = Rc::downgrade(tab);
            let cb = on_state_change.clone();
            tab.buffer.connect_changed(move |_| {
                let Some(tab) = tab_weak.upgrade() else {
                    return;
                };
                if !tab.suppress_dirty.get() {
                    tab.dirty.set(true);
                    // If buffer was PlainText & untitled, re-evaluate language heuristics
                    if tab.file_path.borrow().is_none() && tab.language.get() == Language::PlainText
                    {
                        let sample = tab.text();
                        let detected = Language::from_path_and_content(None, &sample);
                        if detected != Language::PlainText {
                            tab.language.set(detected);
                            tab.indent_size.set(detected.default_indent_size());
                        }
                    }
                }
                tab.schedule_highlight();
                tab.gutter.queue_draw();
                cb();
            });
        }

        // Cursor position changes: update Ln/Col status & active line in gutter
        {
            let tab_weak = Rc::downgrade(tab);
            let cb = on_state_change;
            tab.buffer.connect_mark_set(move |buf, _iter, mark| {
                if mark.name().as_deref() == Some("insert") && buf.has_selection()
                    || mark.name().as_deref() == Some("insert")
                {
                    if let Some(tab) = tab_weak.upgrade() {
                        tab.gutter.queue_draw();
                        cb();
                    }
                }
            });
        }
    }

    fn setup_smart_keys(tab: &Rc<Self>) {
        let key_ctrl = EventControllerKey::new();
        key_ctrl.set_propagation_phase(PropagationPhase::Capture);

        let tab_weak = Rc::downgrade(tab);
        key_ctrl.connect_key_pressed(move |_, key, _code, state| {
            let Some(tab) = tab_weak.upgrade() else {
                return glib::Propagation::Proceed;
            };

            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
            let alt = state.contains(gdk::ModifierType::ALT_MASK);

            // Ctrl+/ -> Toggle line comment
            if ctrl && !alt && (key == gdk::Key::slash || key == gdk::Key::KP_Divide) {
                tab.toggle_comment();
                return glib::Propagation::Stop;
            }

            // Tab / Shift+Tab -> Smart spaces indent / block indent & outdent
            if !ctrl && !alt && (key == gdk::Key::Tab || key == gdk::Key::ISO_Left_Tab) {
                if shift || key == gdk::Key::ISO_Left_Tab {
                    tab.outdent_selection_or_line();
                } else {
                    tab.indent_selection_or_insert_spaces();
                }
                return glib::Propagation::Stop;
            }

            // Enter / Return -> Smart auto-indentation
            if !ctrl
                && !alt
                && !shift
                && (key == gdk::Key::Return || key == gdk::Key::KP_Enter)
            {
                tab.smart_newline();
                return glib::Propagation::Stop;
            }

            // '}' -> Auto-dedent if typed on whitespace-only indented line
            if !ctrl && !alt && key == gdk::Key::braceright {
                if tab.try_smart_closing_brace() {
                    return glib::Propagation::Stop;
                }
            }

            glib::Propagation::Proceed
        });

        tab.text_view.add_controller(key_ctrl);
    }

    pub fn schedule_highlight(self: &Rc<Self>) {
        if self.highlight_scheduled.get() {
            return;
        }
        self.highlight_scheduled.set(true);
        let tab_weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(35), move || {
            if let Some(tab) = tab_weak.upgrade() {
                tab.highlight_scheduled.set(false);
                highlight_buffer(&tab.buffer, tab.language.get());
            }
        });
    }

    pub fn text(&self) -> String {
        let start = self.buffer.start_iter();
        let end = self.buffer.end_iter();
        self.buffer.text(&start, &end, false).to_string()
    }

    pub fn cursor_iter(&self) -> TextIter {
        let mark = self.buffer.get_insert();
        self.buffer.iter_at_mark(&mark)
    }

    pub fn cursor_line_col(&self) -> (usize, usize) {
        let iter = self.cursor_iter();
        ((iter.line() + 1) as usize, (iter.line_offset() + 1) as usize)
    }

    pub fn format_document(self: &Rc<Self>, use_system_formatters: bool) -> FormatOutcome {
        let current = self.text();
        let path_borrow = self.file_path.borrow().clone();
        let outcome = format_code(
            &current,
            self.language.get(),
            path_borrow.as_deref(),
            FormatOptions {
                indent_size: self.indent_size.get(),
                use_system_formatters,
            },
        );

        if outcome.changed {
            let (line, col) = self.cursor_line_col();
            let scroll_val = self.scrolled.vadjustment().value();

            self.buffer.begin_user_action();
            let mut start = self.buffer.start_iter();
            let mut end = self.buffer.end_iter();
            self.buffer.delete(&mut start, &mut end);
            let mut insert_pos = self.buffer.start_iter();
            self.buffer.insert(&mut insert_pos, &outcome.formatted);
            self.buffer.end_user_action();

            // Restore cursor smoothly
            let target_line = ((line - 1) as i32).min(self.buffer.line_count().saturating_sub(1));
            if let Some(mut restored) = self.buffer.iter_at_line(target_line) {
                let max_chars = restored.chars_in_line().saturating_sub(1).max(0);
                let target_col = ((col - 1) as i32).min(max_chars);
                restored.set_line_offset(target_col);
                self.buffer.place_cursor(&restored);
            }

            highlight_buffer(&self.buffer, self.language.get());

            let vadj = self.scrolled.vadjustment();
            glib::idle_add_local_once(move || {
                vadj.set_value(scroll_val);
            });
        }

        outcome
    }

    pub fn save_to_path(self: &Rc<Self>, path: &Path) -> std::io::Result<()> {
        let content = self.text();
        fs::write(path, &content)?;
        *self.file_path.borrow_mut() = Some(path.to_path_buf());
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            *self.title.borrow_mut() = name.to_string();
        }
        let new_lang = Language::from_path_and_content(Some(path), &content);
        if new_lang != self.language.get() {
            self.language.set(new_lang);
            highlight_buffer(&self.buffer, new_lang);
        }
        self.dirty.set(false);
        Ok(())
    }

    pub fn set_language(self: &Rc<Self>, lang: Language) {
        self.language.set(lang);
        self.indent_size.set(lang.default_indent_size());
        highlight_buffer(&self.buffer, lang);
    }

    fn indent_selection_or_insert_spaces(&self) {
        let indent_w = self.indent_size.get().max(1);
        if let Some((sel_start, sel_end)) = self.buffer.selection_bounds() {
            if sel_start.line() != sel_end.line() {
                let start_line = sel_start.line();
                let mut end_line = sel_end.line();
                if sel_end.starts_line() && end_line > start_line {
                    end_line -= 1;
                }
                let pad = " ".repeat(indent_w);
                self.buffer.begin_user_action();
                for ln in start_line..=end_line {
                    if let Some(mut line_iter) = self.buffer.iter_at_line(ln) {
                        self.buffer.insert(&mut line_iter, &pad);
                    }
                }
                self.buffer.end_user_action();
                return;
            }
        }

        let iter = self.cursor_iter();
        let col = iter.line_offset().max(0) as usize;
        let spaces_needed = indent_w - (col % indent_w);
        self.buffer.begin_user_action();
        self.buffer.delete_selection(true, true);
        self.buffer.insert_at_cursor(&" ".repeat(spaces_needed));
        self.buffer.end_user_action();
    }

    fn outdent_selection_or_line(&self) {
        let indent_w = self.indent_size.get().max(1);
        let (start_line, end_line) =
            if let Some((sel_start, sel_end)) = self.buffer.selection_bounds() {
                let s = sel_start.line();
                let mut e = sel_end.line();
                if sel_end.starts_line() && e > s {
                    e -= 1;
                }
                (s, e)
            } else {
                let ln = self.cursor_iter().line();
                (ln, ln)
            };

        self.buffer.begin_user_action();
        for ln in start_line..=end_line {
            if let Some(mut line_start) = self.buffer.iter_at_line(ln) {
                let mut line_end = line_start;
                if !line_end.ends_line() {
                    line_end.forward_to_line_end();
                }
                let line_txt = self.buffer.text(&line_start, &line_end, false);
                let mut remove_chars = 0i32;
                for ch in line_txt.chars().take(indent_w) {
                    if ch == ' ' {
                        remove_chars += 1;
                    } else if ch == '\t' && remove_chars == 0 {
                        remove_chars = 1;
                        break;
                    } else {
                        break;
                    }
                }
                if remove_chars > 0 {
                    let mut del_end = line_start;
                    del_end.forward_chars(remove_chars);
                    self.buffer.delete(&mut line_start, &mut del_end);
                }
            }
        }
        self.buffer.end_user_action();
    }

    fn smart_newline(&self) {
        let indent_w = self.indent_size.get().max(1);
        self.buffer.begin_user_action();
        self.buffer.delete_selection(true, true);

        let cursor = self.cursor_iter();
        let line_idx = cursor.line();
        let line_start = self
            .buffer
            .iter_at_line(line_idx)
            .unwrap_or_else(|| self.buffer.start_iter());

        let before_cursor = self.buffer.text(&line_start, &cursor, false).to_string();
        let leading_ws: String = before_cursor
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .map(|c| if c == '\t' { " ".repeat(indent_w) } else { " ".to_string() })
            .collect();

        let trimmed_before = before_cursor.trim_end();
        let last_char = trimmed_before.chars().last();

        let mut next_after = cursor;
        let char_after = if next_after.forward_char() {
            let s = self.buffer.text(&cursor, &next_after, false);
            s.chars().next()
        } else {
            None
        };

        let opens_block = matches!(last_char, Some('{') | Some('[') | Some('('))
            || (last_char == Some(':')
                && matches!(
                    self.language.get(),
                    Language::Python | Language::Yaml
                ));

        let matching_pair = matches!(
            (last_char, char_after),
            (Some('{'), Some('}')) | (Some('['), Some(']')) | (Some('('), Some(')'))
        );

        if matching_pair {
            let inner = format!("{}{}", leading_ws, " ".repeat(indent_w));
            let text_to_insert = format!("\n{inner}\n{leading_ws}");
            self.buffer.insert_at_cursor(&text_to_insert);
            // Move cursor back onto the middle indented line
            let mut new_pos = self.cursor_iter();
            new_pos.backward_chars((leading_ws.chars().count() + 1) as i32);
            self.buffer.place_cursor(&new_pos);
        } else if opens_block {
            let inner = format!("{}{}", leading_ws, " ".repeat(indent_w));
            self.buffer.insert_at_cursor(&format!("\n{inner}"));
        } else {
            self.buffer.insert_at_cursor(&format!("\n{leading_ws}"));
        }

        self.buffer.end_user_action();
        self.text_view
            .scroll_mark_onscreen(&self.buffer.get_insert());
    }

    fn try_smart_closing_brace(&self) -> bool {
        if self.buffer.has_selection() {
            return false;
        }
        let indent_w = self.indent_size.get().max(1);
        let cursor = self.cursor_iter();
        let Some(mut line_start) = self.buffer.iter_at_line(cursor.line()) else {
            return false;
        };
        let before = self.buffer.text(&line_start, &cursor, false).to_string();
        if before.chars().all(|c| c == ' ') && before.len() >= indent_w {
            self.buffer.begin_user_action();
            let mut del_end = line_start;
            del_end.forward_chars(indent_w as i32);
            self.buffer.delete(&mut line_start, &mut del_end);
            self.buffer.insert_at_cursor("}");
            self.buffer.end_user_action();
            return true;
        }
        false
    }

    pub fn toggle_comment(&self) {
        let Some(prefix) = self.language.get().line_comment_prefix() else {
            return;
        };
        let (start_line, end_line) =
            if let Some((sel_start, sel_end)) = self.buffer.selection_bounds() {
                let s = sel_start.line();
                let mut e = sel_end.line();
                if sel_end.starts_line() && e > s {
                    e -= 1;
                }
                (s, e)
            } else {
                let ln = self.cursor_iter().line();
                (ln, ln)
            };

        let mut all_commented = true;
        for ln in start_line..=end_line {
            if let Some(ls) = self.buffer.iter_at_line(ln) {
                let mut le = ls;
                if !le.ends_line() {
                    le.forward_to_line_end();
                }
                let txt = self.buffer.text(&ls, &le, false);
                let trimmed = txt.trim_start();
                if !trimmed.is_empty() && !trimmed.starts_with(prefix) {
                    all_commented = false;
                    break;
                }
            }
        }

        self.buffer.begin_user_action();
        for ln in start_line..=end_line {
            if let Some(ls) = self.buffer.iter_at_line(ln) {
                let mut le = ls;
                if !le.ends_line() {
                    le.forward_to_line_end();
                }
                let txt = self.buffer.text(&ls, &le, false).to_string();
                if txt.trim().is_empty() {
                    continue;
                }
                let ws_chars = txt.chars().take_while(|c| c.is_whitespace()).count() as i32;
                let mut target = ls;
                target.forward_chars(ws_chars);

                if all_commented {
                    let rest = &txt[txt.len() - txt.trim_start().len()..];
                    let remove_len = if rest.starts_with(&format!("{prefix} ")) {
                        prefix.chars().count() + 1
                    } else if rest.starts_with(prefix) {
                        prefix.chars().count()
                    } else {
                        0
                    };
                    if remove_len > 0 {
                        let mut del_end = target;
                        del_end.forward_chars(remove_len as i32);
                        self.buffer.delete(&mut target, &mut del_end);
                    }
                } else {
                    self.buffer.insert(&mut target, &format!("{prefix} "));
                }
            }
        }
        self.buffer.end_user_action();
    }

    pub fn highlight_search(&self, query: &str) -> usize {
        let start = self.buffer.start_iter();
        let end = self.buffer.end_iter();
        self.buffer.remove_tag_by_name("search_match", &start, &end);
        if query.is_empty() {
            return 0;
        }

        let mut count = 0usize;
        let mut cursor = self.buffer.start_iter();
        while let Some((m_start, m_end)) =
            cursor.forward_search(query, gtk4::TextSearchFlags::CASE_INSENSITIVE, None)
        {
            self.buffer
                .apply_tag_by_name("search_match", &m_start, &m_end);
            count += 1;
            cursor = m_end;
        }
        count
    }

    pub fn find_next(&self, query: &str, backwards: bool) -> bool {
        if query.is_empty() {
            return false;
        }
        let flags = gtk4::TextSearchFlags::CASE_INSENSITIVE;
        let current_bounds = self.buffer.selection_bounds();

        if backwards {
            let search_from = current_bounds
                .map(|(s, _)| s)
                .unwrap_or_else(|| self.cursor_iter());
            let found = search_from
                .backward_search(query, flags, None)
                .or_else(|| self.buffer.end_iter().backward_search(query, flags, None));
            if let Some((mut m_start, m_end)) = found {
                self.buffer.select_range(&m_start, &m_end);
                self.text_view
                    .scroll_to_iter(&mut m_start, 0.15, false, 0.0, 0.5);
                return true;
            }
        } else {
            let search_from = current_bounds
                .map(|(_, e)| e)
                .unwrap_or_else(|| self.cursor_iter());
            let found = search_from
                .forward_search(query, flags, None)
                .or_else(|| self.buffer.start_iter().forward_search(query, flags, None));
            if let Some((mut m_start, m_end)) = found {
                self.buffer.select_range(&m_start, &m_end);
                self.text_view
                    .scroll_to_iter(&mut m_start, 0.15, false, 0.0, 0.5);
                return true;
            }
        }
        false
    }
}
