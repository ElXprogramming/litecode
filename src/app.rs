use std::cell::{Cell, RefCell};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4::gdk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, CheckButton, Entry,
    EventControllerKey, FileDialog, GestureClick, Label, Orientation, Paned, PolicyType, Popover,
    PropagationPhase, ScrolledWindow, Stack,
};

use crate::editor::EditorTab;
use crate::language::Language;
use crate::sidebar::ExplorerSidebar;

struct TabEntry {
    id: String,
    editor: Rc<EditorTab>,
    header_widget: GtkBox,
    title_label: Label,
    badge_label: Label,
}

pub struct LiteCodeUi {
    pub window: ApplicationWindow,
    paned: Paned,
    sidebar: Rc<ExplorerSidebar>,
    sidebar_visible: Cell<bool>,
    tab_strip: GtkBox,
    editor_stack: Stack,
    tabs: RefCell<Vec<TabEntry>>,
    active_tab_id: RefCell<Option<String>>,
    next_untitled_num: Cell<usize>,
    next_tab_uid: Cell<usize>,
    format_on_save: CheckButton,
    use_sys_fmt: CheckButton,
    find_bar: GtkBox,
    find_entry: Entry,
    find_count_lbl: Label,
    status_msg_lbl: Label,
    cursor_pos_lbl: Label,
    indent_btn: Button,
    lang_btn: Button,
    lang_popover: Popover,
}

impl LiteCodeUi {
    pub fn build(app: &Application, initial_paths: &[PathBuf]) -> Rc<Self> {
        Self::load_css();

        let window = ApplicationWindow::builder()
            .application(app)
            .title("LiteCode")
            .default_width(1120)
            .default_height(740)
            .build();

        let app_root = GtkBox::new(Orientation::Vertical, 0);
        app_root.add_css_class("app-root");

        // 1. Top Command / Action Bar
        let toolbar = GtkBox::new(Orientation::Horizontal, 4);
        toolbar.add_css_class("top-toolbar");

        let sidebar_toggle_btn = Button::with_label("☰");
        sidebar_toggle_btn.add_css_class("toolbar-btn");
        sidebar_toggle_btn.set_tooltip_text(Some("Toggle Explorer Sidebar (Ctrl+B)"));

        let brand = Label::new(Some("LiteCode"));
        brand.add_css_class("brand-label");

        let new_btn = Button::with_label("New");
        new_btn.add_css_class("toolbar-btn");
        new_btn.set_tooltip_text(Some("New File (Ctrl+N)"));

        let open_file_btn = Button::with_label("Open File");
        open_file_btn.add_css_class("toolbar-btn");
        open_file_btn.set_tooltip_text(Some("Open File (Ctrl+O)"));

        let open_folder_btn = Button::with_label("Open Folder");
        open_folder_btn.add_css_class("toolbar-btn");
        open_folder_btn.set_tooltip_text(Some("Open Workspace Folder (Ctrl+Shift+O)"));

        let save_btn = Button::with_label("Save");
        save_btn.add_css_class("toolbar-btn");
        save_btn.set_tooltip_text(Some("Save File (Ctrl+S)"));

        let spacer = GtkBox::new(Orientation::Horizontal, 0);
        spacer.set_hexpand(true);

        let format_on_save = CheckButton::with_label("Format on Save");
        format_on_save.add_css_class("toggle-pill");
        format_on_save.set_active(false);
        format_on_save.set_tooltip_text(Some("Automatically format code whenever saving (Ctrl+S)"));

        let use_sys_fmt = CheckButton::with_label("System Tools");
        use_sys_fmt.add_css_class("toggle-pill");
        use_sys_fmt.set_active(true);
        use_sys_fmt.set_tooltip_text(Some(
            "Use installed CLI formatters (rustfmt, clang-format, prettier, ruff, gofmt) with automatic fallback to Built-in Smart Indenter",
        ));

        let find_btn = Button::with_label("Find");
        find_btn.add_css_class("toolbar-btn");
        find_btn.set_tooltip_text(Some("Find in File (Ctrl+F)"));

        let format_btn = Button::with_label("✦ Format Code");
        format_btn.add_css_class("format-primary-btn");
        format_btn.set_tooltip_text(Some("Auto-Format Indentation & Code (Ctrl+Shift+I / Alt+Shift+F)"));

        toolbar.append(&sidebar_toggle_btn);
        toolbar.append(&brand);
        toolbar.append(&new_btn);
        toolbar.append(&open_file_btn);
        toolbar.append(&open_folder_btn);
        toolbar.append(&save_btn);
        toolbar.append(&spacer);
        toolbar.append(&use_sys_fmt);
        toolbar.append(&format_on_save);
        toolbar.append(&find_btn);
        toolbar.append(&format_btn);

        // 2. Editor Right Pane (Tab Bar + Optional Find Bar + Stack of Editors)
        let tab_strip = GtkBox::new(Orientation::Horizontal, 0);
        tab_strip.add_css_class("tab-strip");

        let tab_strip_scroll = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Automatic)
            .vscrollbar_policy(PolicyType::Never)
            .hexpand(true)
            .child(&tab_strip)
            .build();
        tab_strip_scroll.add_css_class("tab-strip-scroll");

        // Find Bar (hidden by default)
        let find_bar = GtkBox::new(Orientation::Horizontal, 6);
        find_bar.add_css_class("find-bar");
        find_bar.set_visible(false);

        let find_lbl = Label::new(Some("Find:"));
        let find_entry = Entry::new();
        find_entry.add_css_class("find-entry");
        find_entry.set_placeholder_text(Some("Search text… (Enter = Next, Shift+Enter = Prev, Esc = Close)"));
        find_entry.set_width_chars(32);

        let find_prev_btn = Button::with_label("↑");
        find_prev_btn.add_css_class("toolbar-btn");
        let find_next_btn = Button::with_label("↓");
        find_next_btn.add_css_class("toolbar-btn");
        let find_count_lbl = Label::new(Some("0 matches"));
        find_count_lbl.add_css_class("find-count");

        let find_spacer = GtkBox::new(Orientation::Horizontal, 0);
        find_spacer.set_hexpand(true);
        let find_close_btn = Button::with_label("✕");
        find_close_btn.add_css_class("tab-close-btn");

        find_bar.append(&find_lbl);
        find_bar.append(&find_entry);
        find_bar.append(&find_prev_btn);
        find_bar.append(&find_next_btn);
        find_bar.append(&find_count_lbl);
        find_bar.append(&find_spacer);
        find_bar.append(&find_close_btn);

        let editor_stack = Stack::new();
        editor_stack.set_hexpand(true);
        editor_stack.set_vexpand(true);

        let editor_pane = GtkBox::new(Orientation::Vertical, 0);
        editor_pane.set_hexpand(true);
        editor_pane.set_vexpand(true);
        editor_pane.append(&tab_strip_scroll);
        editor_pane.append(&find_bar);
        editor_pane.append(&editor_stack);

        // 3. Bottom VS Code Status Bar
        let status_bar = GtkBox::new(Orientation::Horizontal, 6);
        status_bar.add_css_class("status-bar");

        let status_msg_lbl = Label::new(Some("Ready — Press Ctrl+Shift+I to Auto-Format"));
        status_msg_lbl.add_css_class("status-item");
        status_msg_lbl.set_halign(Align::Start);
        status_msg_lbl.set_hexpand(true);

        let cursor_pos_lbl = Label::new(Some("Ln 1, Col 1"));
        cursor_pos_lbl.add_css_class("status-item");

        let indent_btn = Button::with_label("Spaces: 4");
        indent_btn.add_css_class("status-btn");
        indent_btn.set_tooltip_text(Some("Click to toggle indentation width (2 or 4 spaces)"));

        let lang_btn = Button::with_label("Plain Text");
        lang_btn.add_css_class("status-btn");
        lang_btn.set_tooltip_text(Some("Click to select syntax & auto-formatter language"));

        let lang_popover = Popover::new();
        lang_popover.set_parent(&lang_btn);

        status_bar.append(&status_msg_lbl);
        status_bar.append(&cursor_pos_lbl);
        status_bar.append(&indent_btn);
        status_bar.append(&lang_btn);

        // Placeholder callbacks wired right after Rc<LiteCodeUi> creation
        let ui_holder: Rc<RefCell<Option<Rc<Self>>>> = Rc::new(RefCell::new(None));

        let on_sidebar_open = {
            let holder = ui_holder.clone();
            Rc::new(move |path: PathBuf| {
                if let Some(ui) = holder.borrow().as_ref() {
                    ui.open_file_from_disk(&path);
                }
            })
        };
        let on_sidebar_pick_dir = {
            let holder = ui_holder.clone();
            Rc::new(move || {
                if let Some(ui) = holder.borrow().as_ref() {
                    ui.prompt_open_folder();
                }
            })
        };

        let sidebar = ExplorerSidebar::new(on_sidebar_open, on_sidebar_pick_dir);

        let paned = Paned::new(Orientation::Horizontal);
        paned.set_start_child(Some(&sidebar.root));
        paned.set_resize_start_child(false);
        paned.set_shrink_start_child(false);
        paned.set_end_child(Some(&editor_pane));
        paned.set_position(235);
        paned.set_hexpand(true);
        paned.set_vexpand(true);

        app_root.append(&toolbar);
        app_root.append(&paned);
        app_root.append(&status_bar);

        window.set_child(Some(&app_root));

        let ui = Rc::new(Self {
            window,
            paned,
            sidebar,
            sidebar_visible: Cell::new(true),
            tab_strip,
            editor_stack,
            tabs: RefCell::new(Vec::new()),
            active_tab_id: RefCell::new(None),
            next_untitled_num: Cell::new(1),
            next_tab_uid: Cell::new(1),
            format_on_save,
            use_sys_fmt,
            find_bar,
            find_entry,
            find_count_lbl,
            status_msg_lbl,
            cursor_pos_lbl,
            indent_btn,
            lang_btn,
            lang_popover,
        });

        *ui_holder.borrow_mut() = Some(ui.clone());

        ui.setup_language_popover();
        ui.wire_toolbar_and_find(
            &sidebar_toggle_btn,
            &new_btn,
            &open_file_btn,
            &open_folder_btn,
            &save_btn,
            &find_btn,
            &format_btn,
            &find_prev_btn,
            &find_next_btn,
            &find_close_btn,
        );
        ui.wire_global_shortcuts();

        // Clean up popover parent on window close
        {
            let pop = ui.lang_popover.clone();
            ui.window.connect_close_request(move |_| {
                pop.unparent();
                glib::Propagation::Proceed
            });
        }

        // Load initial paths or default workspace
        ui.initialize_workspace(initial_paths);

        ui
    }

    fn load_css() {
        let provider = gtk4::CssProvider::new();
        provider.load_from_string(include_str!("style.css"));
        if let Some(display) = gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    }

    fn initialize_workspace(self: &Rc<Self>, initial_paths: &[PathBuf]) {
        let mut opened_any_file = false;
        let mut set_any_folder = false;

        for p in initial_paths {
            if p.is_dir() {
                self.sidebar.set_root_folder(p.clone());
                set_any_folder = true;
            } else if p.exists() || p.extension().is_some() {
                if !set_any_folder {
                    if let Some(parent) = p.parent() {
                        if !parent.as_os_str().is_empty() {
                            self.sidebar.set_root_folder(parent.to_path_buf());
                            set_any_folder = true;
                        }
                    }
                }
                self.open_or_create_path(p);
                opened_any_file = true;
            }
        }

        if !set_any_folder {
            if let Ok(cwd) = std::env::current_dir() {
                self.sidebar.set_root_folder(cwd);
            }
        }

        if !opened_any_file {
            self.new_untitled_tab("");
        }
    }

    fn setup_language_popover(self: &Rc<Self>) {
        let list_box = GtkBox::new(Orientation::Vertical, 2);
        list_box.set_margin_top(6);
        list_box.set_margin_bottom(6);
        list_box.set_margin_start(6);
        list_box.set_margin_end(6);

        for &lang in Language::all_selectable() {
            let btn = Button::with_label(lang.display_name());
            btn.add_css_class("toolbar-btn");
            let ui_weak = Rc::downgrade(self);
            let pop = self.lang_popover.clone();
            btn.connect_clicked(move |_| {
                pop.popdown();
                if let Some(ui) = ui_weak.upgrade() {
                    if let Some(tab) = ui.current_editor() {
                        tab.set_language(lang);
                        ui.refresh_ui_state();
                        ui.set_status(&format!("Language set to {}", lang.display_name()));
                    }
                }
            });
            list_box.append(&btn);
        }

        let sc = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vscrollbar_policy(PolicyType::Automatic)
            .min_content_height(260)
            .min_content_width(170)
            .child(&list_box)
            .build();
        self.lang_popover.set_child(Some(&sc));

        let pop = self.lang_popover.clone();
        self.lang_btn.connect_clicked(move |_| {
            pop.popup();
        });

        let ui_weak = Rc::downgrade(self);
        self.indent_btn.connect_clicked(move |_| {
            if let Some(ui) = ui_weak.upgrade() {
                if let Some(tab) = ui.current_editor() {
                    let next = if tab.indent_size.get() == 4 { 2 } else { 4 };
                    tab.indent_size.set(next);
                    ui.refresh_ui_state();
                    ui.set_status(&format!("Indentation width set to {next} spaces"));
                }
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn wire_toolbar_and_find(
        self: &Rc<Self>,
        sidebar_toggle_btn: &Button,
        new_btn: &Button,
        open_file_btn: &Button,
        open_folder_btn: &Button,
        save_btn: &Button,
        find_btn: &Button,
        format_btn: &Button,
        find_prev_btn: &Button,
        find_next_btn: &Button,
        find_close_btn: &Button,
    ) {
        {
            let ui = self.clone();
            sidebar_toggle_btn.connect_clicked(move |_| ui.toggle_sidebar());
        }
        {
            let ui = self.clone();
            new_btn.connect_clicked(move |_| ui.new_untitled_tab(""));
        }
        {
            let ui = self.clone();
            open_file_btn.connect_clicked(move |_| ui.prompt_open_file());
        }
        {
            let ui = self.clone();
            open_folder_btn.connect_clicked(move |_| ui.prompt_open_folder());
        }
        {
            let ui = self.clone();
            save_btn.connect_clicked(move |_| ui.save_current_tab(false));
        }
        {
            let ui = self.clone();
            find_btn.connect_clicked(move |_| ui.toggle_find_bar());
        }
        {
            let ui = self.clone();
            format_btn.connect_clicked(move |_| ui.format_active_tab());
        }

        // Find entry events
        {
            let ui = self.clone();
            self.find_entry.connect_changed(move |entry| {
                let q = entry.text();
                if let Some(tab) = ui.current_editor() {
                    let count = tab.highlight_search(&q);
                    ui.find_count_lbl.set_text(&format!("{count} matches"));
                    if count > 0 {
                        tab.find_next(&q, false);
                    }
                }
            });
        }
        {
            let ui = self.clone();
            find_next_btn.connect_clicked(move |_| {
                let q = ui.find_entry.text();
                if let Some(tab) = ui.current_editor() {
                    tab.find_next(&q, false);
                }
            });
        }
        {
            let ui = self.clone();
            find_prev_btn.connect_clicked(move |_| {
                let q = ui.find_entry.text();
                if let Some(tab) = ui.current_editor() {
                    tab.find_next(&q, true);
                }
            });
        }
        {
            let ui = self.clone();
            find_close_btn.connect_clicked(move |_| ui.close_find_bar());
        }
        {
            let key_ctrl = EventControllerKey::new();
            let ui = self.clone();
            key_ctrl.connect_key_pressed(move |_, key, _, state| {
                let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
                if key == gdk::Key::Escape {
                    ui.close_find_bar();
                    return glib::Propagation::Stop;
                }
                if key == gdk::Key::Return || key == gdk::Key::KP_Enter {
                    let q = ui.find_entry.text();
                    if let Some(tab) = ui.current_editor() {
                        tab.find_next(&q, shift);
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            self.find_entry.add_controller(key_ctrl);
        }
    }

    fn wire_global_shortcuts(self: &Rc<Self>) {
        let key_ctrl = EventControllerKey::new();
        key_ctrl.set_propagation_phase(PropagationPhase::Capture);

        let ui_weak = Rc::downgrade(self);
        key_ctrl.connect_key_pressed(move |_, key, _code, state| {
            let Some(ui) = ui_weak.upgrade() else {
                return glib::Propagation::Proceed;
            };

            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
            let alt = state.contains(gdk::ModifierType::ALT_MASK);

            // Ctrl+Shift+I or Alt+Shift+F -> Auto-Format Code
            if (ctrl && shift && !alt && (key == gdk::Key::I || key == gdk::Key::i))
                || (!ctrl && shift && alt && (key == gdk::Key::F || key == gdk::Key::f))
            {
                ui.format_active_tab();
                return glib::Propagation::Stop;
            }

            if ctrl && !alt {
                match key {
                    gdk::Key::s | gdk::Key::S => {
                        ui.save_current_tab(shift);
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::n | gdk::Key::N if !shift => {
                        ui.new_untitled_tab("");
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::o | gdk::Key::O => {
                        if shift {
                            ui.prompt_open_folder();
                        } else {
                            ui.prompt_open_file();
                        }
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::w | gdk::Key::W if !shift => {
                        if let Some(active_id) = ui.active_tab_id.borrow().clone() {
                            ui.close_tab(&active_id);
                        }
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::b | gdk::Key::B if !shift => {
                        ui.toggle_sidebar();
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::f | gdk::Key::F if !shift => {
                        ui.open_find_bar();
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::Tab | gdk::Key::ISO_Left_Tab => {
                        ui.cycle_tab(shift);
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }

            if key == gdk::Key::Escape && ui.find_bar.is_visible() {
                ui.close_find_bar();
                return glib::Propagation::Stop;
            }

            glib::Propagation::Proceed
        });

        self.window.add_controller(key_ctrl);
    }

    pub fn new_untitled_tab(self: &Rc<Self>, initial_text: &str) {
        let num = self.next_untitled_num.get();
        self.next_untitled_num.set(num + 1);
        let title = format!("Untitled-{num}");
        self.add_tab(None, title, initial_text);
    }

    pub fn open_or_create_path(self: &Rc<Self>, path: &Path) {
        if path.exists() {
            self.open_file_from_disk(path);
        } else {
            let title = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("new_file")
                .to_string();
            self.add_tab(Some(path.to_path_buf()), title, "");
        }
    }

    pub fn open_file_from_disk(self: &Rc<Self>, path: &Path) {
        let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

        // Switch if already open
        if let Some(existing_id) = self.tabs.borrow().iter().find_map(|t| {
            let tp = t.editor.file_path.borrow().clone()?;
            let tc = fs::canonicalize(&tp).unwrap_or(tp);
            (tc == canonical).then(|| t.id.clone())
        }) {
            self.activate_tab(&existing_id);
            return;
        }

        match fs::read_to_string(&canonical) {
            Ok(content) => {
                let title = canonical
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("file")
                    .to_string();
                self.add_tab(Some(canonical.clone()), title.clone(), &content);
                self.set_status(&format!("Opened {title}"));
            }
            Err(err) => {
                self.set_status(&format!("Cannot open UTF-8 text file: {err}"));
            }
        }
    }

    fn add_tab(self: &Rc<Self>, path: Option<PathBuf>, title: String, content: &str) {
        let uid = self.next_tab_uid.get();
        self.next_tab_uid.set(uid + 1);
        let tab_id = format!("tab_{uid}");

        let ui_weak = Rc::downgrade(self);
        let on_change = Rc::new(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.refresh_ui_state();
            }
        });

        let editor = EditorTab::new(path, title.clone(), content, on_change);

        let header = GtkBox::new(Orientation::Horizontal, 6);
        header.add_css_class("editor-tab");

        let (b_txt, b_cls) = editor.language.get().badge();
        let badge_label = Label::new(Some(b_txt));
        badge_label.add_css_class("file-badge");
        badge_label.add_css_class(b_cls);

        let title_label = Label::new(Some(&title));
        title_label.add_css_class("tab-title");

        let close_btn = Button::with_label("×");
        close_btn.add_css_class("tab-close-btn");
        close_btn.set_tooltip_text(Some("Close Tab (Ctrl+W)"));

        header.append(&badge_label);
        header.append(&title_label);
        header.append(&close_btn);

        // Click tab header to activate (middle-click to close)
        {
            let click = GestureClick::new();
            click.set_button(0);
            let ui_weak = Rc::downgrade(self);
            let id_copy = tab_id.clone();
            click.connect_pressed(move |gesture, _, _, _| {
                if let Some(ui) = ui_weak.upgrade() {
                    if gesture.current_button() == 2 {
                        ui.close_tab(&id_copy);
                    } else if gesture.current_button() == 1 {
                        ui.activate_tab(&id_copy);
                    }
                }
            });
            header.add_controller(click);
        }

        // Close button
        {
            let ui_weak = Rc::downgrade(self);
            let id_copy = tab_id.clone();
            close_btn.connect_clicked(move |_| {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.close_tab(&id_copy);
                }
            });
        }

        self.tab_strip.append(&header);
        self.editor_stack.add_named(&editor.root, Some(&tab_id));

        self.tabs.borrow_mut().push(TabEntry {
            id: tab_id.clone(),
            editor,
            header_widget: header,
            title_label,
            badge_label,
        });

        self.activate_tab(&tab_id);
    }

    pub fn activate_tab(self: &Rc<Self>, tab_id: &str) {
        *self.active_tab_id.borrow_mut() = Some(tab_id.to_string());
        self.editor_stack.set_visible_child_name(tab_id);

        let mut active_path: Option<PathBuf> = None;
        let mut focus_view = None;

        for entry in self.tabs.borrow().iter() {
            if entry.id == tab_id {
                entry.header_widget.add_css_class("editor-tab-active");
                active_path = entry.editor.file_path.borrow().clone();
                focus_view = Some(entry.editor.text_view.clone());
            } else {
                entry.header_widget.remove_css_class("editor-tab-active");
            }
        }

        self.sidebar.set_active_file(active_path);
        self.refresh_ui_state();

        if let Some(tv) = focus_view {
            tv.grab_focus();
        }
    }

    pub fn close_tab(self: &Rc<Self>, tab_id: &str) {
        let mut tabs = self.tabs.borrow_mut();
        let Some(idx) = tabs.iter().position(|t| t.id == tab_id) else {
            return;
        };

        let removed = tabs.remove(idx);
        self.tab_strip.remove(&removed.header_widget);
        self.editor_stack.remove(&removed.editor.root);

        let was_active = self.active_tab_id.borrow().as_deref() == Some(tab_id);
        if tabs.is_empty() {
            drop(tabs);
            *self.active_tab_id.borrow_mut() = None;
            self.new_untitled_tab("");
        } else if was_active {
            let next_idx = idx.min(tabs.len() - 1);
            let next_id = tabs[next_idx].id.clone();
            drop(tabs);
            self.activate_tab(&next_id);
        }
    }

    fn cycle_tab(self: &Rc<Self>, backwards: bool) {
        let tabs = self.tabs.borrow();
        if tabs.len() <= 1 {
            return;
        }
        let cur_id = self.active_tab_id.borrow().clone();
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
        let next_id = tabs[next_idx].id.clone();
        drop(tabs);
        self.activate_tab(&next_id);
    }

    fn current_editor(&self) -> Option<Rc<EditorTab>> {
        let active = self.active_tab_id.borrow().clone()?;
        self.tabs
            .borrow()
            .iter()
            .find(|t| t.id == active)
            .map(|t| t.editor.clone())
    }

    pub fn format_active_tab(self: &Rc<Self>) {
        let Some(editor) = self.current_editor() else {
            return;
        };
        let outcome = editor.format_document(self.use_sys_fmt.is_active());
        if outcome.changed {
            self.set_status(&format!("Formatted via {}", outcome.engine_used));
        } else {
            self.set_status(&format!("Already formatted ({})", outcome.engine_used));
        }
        self.refresh_ui_state();
        editor.text_view.grab_focus();
    }

    pub fn save_current_tab(self: &Rc<Self>, force_save_as: bool) {
        let Some(editor) = self.current_editor() else {
            return;
        };

        if self.format_on_save.is_active() {
            let _ = editor.format_document(self.use_sys_fmt.is_active());
        }

        let existing_path = editor.file_path.borrow().clone();
        if let (false, Some(path)) = (force_save_as, existing_path) {
            match editor.save_to_path(&path) {
                Ok(()) => {
                    let name = editor.title.borrow().clone();
                    self.set_status(&format!("Saved {name}"));
                    self.sidebar.rebuild_tree();
                    self.refresh_ui_state();
                }
                Err(e) => {
                    self.set_status(&format!("Save failed: {e}"));
                }
            }
        } else {
            self.prompt_save_as(editor);
        }
    }

    fn prompt_save_as(self: &Rc<Self>, editor: Rc<EditorTab>) {
        let dialog = FileDialog::builder()
            .title("Save File")
            .modal(true)
            .initial_name(editor.title.borrow().as_str())
            .build();

        if let Some(folder) = self.sidebar.current_folder() {
            dialog.set_initial_folder(Some(&gio::File::for_path(folder)));
        }

        let ui = self.clone();
        dialog.save(Some(&self.window), gio::Cancellable::NONE, move |res| {
            if let Ok(file) = res {
                if let Some(path) = file.path() {
                    if ui.format_on_save.is_active() {
                        // Update language before format-on-save so newly named file gets right formatter
                        let new_lang = Language::from_path_and_content(Some(&path), &editor.text());
                        editor.set_language(new_lang);
                        let _ = editor.format_document(ui.use_sys_fmt.is_active());
                    }
                    match editor.save_to_path(&path) {
                        Ok(()) => {
                            let name = editor.title.borrow().clone();
                            ui.set_status(&format!("Saved {name}"));
                            ui.sidebar.rebuild_tree();
                            ui.sidebar.set_active_file(Some(path));
                            ui.refresh_ui_state();
                        }
                        Err(e) => ui.set_status(&format!("Save error: {e}")),
                    }
                }
            }
        });
    }

    pub fn prompt_open_file(self: &Rc<Self>) {
        let dialog = FileDialog::builder().title("Open File").modal(true).build();
        if let Some(folder) = self.sidebar.current_folder() {
            dialog.set_initial_folder(Some(&gio::File::for_path(folder)));
        }

        let ui = self.clone();
        dialog.open(Some(&self.window), gio::Cancellable::NONE, move |res| {
            if let Ok(file) = res {
                if let Some(path) = file.path() {
                    ui.open_file_from_disk(&path);
                }
            }
        });
    }

    pub fn prompt_open_folder(self: &Rc<Self>) {
        let dialog = FileDialog::builder()
            .title("Open Workspace Folder")
            .modal(true)
            .build();

        let ui = self.clone();
        dialog.select_folder(Some(&self.window), gio::Cancellable::NONE, move |res| {
            if let Ok(file) = res {
                if let Some(path) = file.path() {
                    ui.sidebar.set_root_folder(path.clone());
                    if !ui.sidebar_visible.get() {
                        ui.toggle_sidebar();
                    }
                    ui.set_status(&format!("Workspace opened: {}", path.display()));
                }
            }
        });
    }

    pub fn toggle_sidebar(self: &Rc<Self>) {
        let next = !self.sidebar_visible.get();
        self.sidebar_visible.set(next);
        self.sidebar.root.set_visible(next);
        if next && self.paned.position() < 120 {
            self.paned.set_position(235);
        }
    }

    fn open_find_bar(&self) {
        self.find_bar.set_visible(true);
        self.find_entry.grab_focus();
        self.find_entry.select_region(0, -1);
    }

    fn toggle_find_bar(&self) {
        if self.find_bar.is_visible() {
            self.close_find_bar();
        } else {
            self.open_find_bar();
        }
    }

    fn close_find_bar(&self) {
        self.find_bar.set_visible(false);
        if let Some(tab) = self.current_editor() {
            tab.highlight_search("");
            tab.text_view.grab_focus();
        }
    }

    pub fn refresh_ui_state(&self) {
        // Update all tab labels (dirty dot + badge)
        for entry in self.tabs.borrow().iter() {
            let base = entry.editor.title.borrow().clone();
            if entry.editor.dirty.get() {
                entry.title_label.set_text(&format!("● {base}"));
            } else {
                entry.title_label.set_text(&base);
            }

            let (b_txt, b_cls) = entry.editor.language.get().badge();
            entry.badge_label.set_text(b_txt);
            // Reset badge color classes cleanly
            for cls in [
                "badge-rust", "badge-c", "badge-cpp", "badge-py", "badge-js", "badge-ts",
                "badge-json", "badge-html", "badge-xml", "badge-css", "badge-go", "badge-java",
                "badge-sh", "badge-lua", "badge-toml", "badge-yaml", "badge-md", "badge-txt",
            ] {
                entry.badge_label.remove_css_class(cls);
            }
            entry.badge_label.add_css_class(b_cls);
        }

        if let Some(editor) = self.current_editor() {
            let (ln, col) = editor.cursor_line_col();
            self.cursor_pos_lbl.set_text(&format!("Ln {ln}, Col {col}"));
            self.indent_btn
                .set_label(&format!("Spaces: {}", editor.indent_size.get()));
            self.lang_btn
                .set_label(editor.language.get().display_name());

            let title = editor.title.borrow().clone();
            let dirty_mark = if editor.dirty.get() { " ●" } else { "" };
            self.window
                .set_title(Some(&format!("{title}{dirty_mark} — LiteCode")));
        }
    }

    pub fn set_status(&self, msg: &str) {
        self.status_msg_lbl.set_text(msg);
    }
}
