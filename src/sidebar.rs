use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4::gdk;
use gtk4::glib;
use gtk4::pango::EllipsizeMode;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, Entry, EventControllerKey, GestureClick, Label, Orientation,
    PolicyType, PropagationPhase, ScrolledWindow,
};

use crate::language::Language;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateKind {
    File,
    Directory,
}

#[derive(Debug, Clone)]
struct PendingCreate {
    parent_dir: PathBuf,
    kind: CreateKind,
    draft_text: String,
}

pub struct ExplorerSidebar {
    pub root: GtkBox,
    tree_box: GtkBox,
    folder_label: Label,
    root_add_btn: Button,
    current_root: RefCell<Option<PathBuf>>,
    expanded_dirs: RefCell<HashSet<PathBuf>>,
    active_file: RefCell<Option<PathBuf>>,
    pending_create: RefCell<Option<PendingCreate>>,
    on_open_file: Rc<dyn Fn(PathBuf)>,
    on_status: Rc<dyn Fn(&str)>,
}

impl ExplorerSidebar {
    pub fn new(
        on_open_file: Rc<dyn Fn(PathBuf)>,
        on_pick_folder: Rc<dyn Fn()>,
        on_status: Rc<dyn Fn(&str)>,
    ) -> Rc<Self> {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("sidebar-panel");
        root.set_width_request(240);
        root.set_vexpand(true);

        // Top EXPLORER header bar
        let top_header = GtkBox::new(Orientation::Horizontal, 6);
        top_header.add_css_class("sidebar-header");

        let title_lbl = Label::new(Some("EXPLORER"));
        title_lbl.add_css_class("sidebar-title");
        title_lbl.set_halign(Align::Start);
        title_lbl.set_hexpand(true);

        let open_dir_btn = Button::with_label("Folder…");
        open_dir_btn.add_css_class("sidebar-btn");
        open_dir_btn.set_tooltip_text(Some("Open Folder (Ctrl+Shift+O)"));

        let refresh_btn = Button::with_label("↻");
        refresh_btn.add_css_class("sidebar-btn");
        refresh_btn.set_tooltip_text(Some("Refresh Explorer"));

        top_header.append(&title_lbl);
        top_header.append(&open_dir_btn);
        top_header.append(&refresh_btn);

        // Workspace root folder header row with '+' button on the right
        let workspace_row = GtkBox::new(Orientation::Horizontal, 6);
        workspace_row.add_css_class("sidebar-workspace-row");

        let folder_label = Label::new(Some("NO FOLDER OPEN"));
        folder_label.add_css_class("sidebar-workspace-name");
        folder_label.set_halign(Align::Start);
        folder_label.set_hexpand(true);
        folder_label.set_ellipsize(EllipsizeMode::End);

        let root_add_btn = Button::with_label("+");
        root_add_btn.add_css_class("folder-add-btn");
        root_add_btn.set_tooltip_text(Some("New File or Directory in workspace root"));
        root_add_btn.set_visible(false);

        workspace_row.append(&folder_label);
        workspace_row.append(&root_add_btn);

        let tree_box = GtkBox::new(Orientation::Vertical, 1);
        tree_box.add_css_class("sidebar-tree");
        tree_box.set_vexpand(true);

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vscrollbar_policy(PolicyType::Automatic)
            .vexpand(true)
            .child(&tree_box)
            .build();

        root.append(&top_header);
        root.append(&workspace_row);
        root.append(&scrolled);

        let sidebar = Rc::new(Self {
            root,
            tree_box,
            folder_label,
            root_add_btn: root_add_btn.clone(),
            current_root: RefCell::new(None),
            expanded_dirs: RefCell::new(HashSet::new()),
            active_file: RefCell::new(None),
            pending_create: RefCell::new(None),
            on_open_file,
            on_status,
        });

        {
            let pick_cb = on_pick_folder;
            open_dir_btn.connect_clicked(move |_| {
                pick_cb();
            });
        }
        {
            let sb_weak = Rc::downgrade(&sidebar);
            refresh_btn.connect_clicked(move |_| {
                if let Some(sb) = sb_weak.upgrade() {
                    sb.rebuild_tree();
                }
            });
        }
        {
            let sb_weak = Rc::downgrade(&sidebar);
            root_add_btn.connect_clicked(move |_| {
                if let Some(sb) = sb_weak.upgrade() {
                    let maybe_root = sb.current_root.borrow().clone();
                    if let Some(root_dir) = maybe_root {
                        sb.open_creator_for(root_dir, CreateKind::File);
                    }
                }
            });
        }

        sidebar.rebuild_tree();
        sidebar
    }

    pub fn set_root_folder(self: &Rc<Self>, path: PathBuf) {
        let canonical = fs::canonicalize(&path).unwrap_or(path);
        let folder_name = canonical
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("/")
            .to_uppercase();
        self.folder_label.set_text(&format!("▾  {folder_name}"));
        self.root_add_btn.set_visible(true);
        self.expanded_dirs.borrow_mut().clear();
        *self.pending_create.borrow_mut() = None;
        *self.current_root.borrow_mut() = Some(canonical);
        self.rebuild_tree();
    }

    pub fn current_folder(&self) -> Option<PathBuf> {
        self.current_root.borrow().clone()
    }

    pub fn set_active_file(self: &Rc<Self>, path: Option<PathBuf>) {
        let normalized = path.map(|p| fs::canonicalize(&p).unwrap_or(p));
        if *self.active_file.borrow() != normalized {
            *self.active_file.borrow_mut() = normalized;
            self.rebuild_tree();
        }
    }

    fn open_creator_for(self: &Rc<Self>, folder_path: PathBuf, kind: CreateKind) {
        let canonical = fs::canonicalize(&folder_path).unwrap_or(folder_path);
        // Ensure target subfolder is expanded so the inline creator appears right inside it
        if self.current_root.borrow().as_ref() != Some(&canonical) {
            self.expanded_dirs.borrow_mut().insert(canonical.clone());
        }
        *self.pending_create.borrow_mut() = Some(PendingCreate {
            parent_dir: canonical,
            kind,
            draft_text: String::new(),
        });
        self.rebuild_tree();
    }

    pub fn rebuild_tree(self: &Rc<Self>) {
        while let Some(child) = self.tree_box.first_child() {
            self.tree_box.remove(&child);
        }

        let Some(root_dir) = self.current_root.borrow().clone() else {
            let empty_lbl = Label::new(Some("Open a folder to browse files."));
            empty_lbl.add_css_class("sidebar-empty");
            empty_lbl.set_wrap(true);
            empty_lbl.set_halign(Align::Start);
            self.tree_box.append(&empty_lbl);
            return;
        };

        // Check if inline creator is open at the workspace root level
        if let Some(pending) = self.pending_create.borrow().clone() {
            if pending.parent_dir == root_dir {
                let creator_widget = self.build_inline_creator(&pending, 0);
                self.tree_box.append(&creator_widget);
            }
        }

        self.populate_dir(&root_dir, 0);
    }

    fn populate_dir(self: &Rc<Self>, dir: &Path, depth: usize) {
        if depth > 12 {
            return;
        }
        let Ok(read_dir) = fs::read_dir(dir) else {
            return;
        };

        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut files: Vec<PathBuf> = Vec::new();

        for entry in read_dir.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if should_skip_entry(name) {
                continue;
            }
            if path.is_dir() {
                dirs.push(path);
            } else if path.is_file() {
                files.push(path);
            }
        }

        dirs.sort_by_key(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
        });
        files.sort_by_key(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
        });

        for sub_dir in dirs {
            let canonical_sub = fs::canonicalize(&sub_dir).unwrap_or_else(|_| sub_dir.clone());
            let is_expanded = self.expanded_dirs.borrow().contains(&canonical_sub);
            let row = self.build_dir_row(&canonical_sub, depth, is_expanded);
            self.tree_box.append(&row);

            if is_expanded {
                if let Some(pending) = self.pending_create.borrow().clone() {
                    if pending.parent_dir == canonical_sub {
                        let creator_widget = self.build_inline_creator(&pending, depth + 1);
                        self.tree_box.append(&creator_widget);
                    }
                }
                self.populate_dir(&canonical_sub, depth + 1);
            }
        }

        for file_path in files {
            let row = self.build_file_row(&file_path, depth);
            self.tree_box.append(&row);
        }
    }

    fn build_dir_row(self: &Rc<Self>, dir_path: &Path, depth: usize, expanded: bool) -> GtkBox {
        let row = GtkBox::new(Orientation::Horizontal, 4);
        row.add_css_class("tree-row");
        row.set_margin_start((depth as i32 * 14) + 8);

        // Left clickable region (chevron + folder name) to expand/collapse
        let clickable_area = GtkBox::new(Orientation::Horizontal, 6);
        clickable_area.set_hexpand(true);

        let chevron = Label::new(Some(if expanded { "▾" } else { "▸" }));
        chevron.add_css_class("tree-chevron");

        let name = dir_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("folder");
        let lbl = Label::new(Some(name));
        lbl.add_css_class("tree-dir-label");
        lbl.set_halign(Align::Start);
        lbl.set_hexpand(true);
        lbl.set_ellipsize(EllipsizeMode::End);

        clickable_area.append(&chevron);
        clickable_area.append(&lbl);

        // Right '+' button to add a file or subdirectory inside this folder
        let add_btn = Button::with_label("+");
        add_btn.add_css_class("folder-add-btn");
        add_btn.set_tooltip_text(Some(&format!("New File or Directory inside '{name}'")));

        row.append(&clickable_area);
        row.append(&add_btn);

        {
            let click = GestureClick::new();
            let sb_weak = Rc::downgrade(self);
            let owned = dir_path.to_path_buf();
            click.connect_released(move |_, _, _, _| {
                if let Some(sb) = sb_weak.upgrade() {
                    let mut exp = sb.expanded_dirs.borrow_mut();
                    if exp.contains(&owned) {
                        exp.remove(&owned);
                    } else {
                        exp.insert(owned.clone());
                    }
                    drop(exp);
                    sb.rebuild_tree();
                }
            });
            clickable_area.add_controller(click);
        }

        {
            let sb_weak = Rc::downgrade(self);
            let owned = dir_path.to_path_buf();
            add_btn.connect_clicked(move |_| {
                if let Some(sb) = sb_weak.upgrade() {
                    sb.open_creator_for(owned.clone(), CreateKind::File);
                }
            });
        }

        row
    }

    fn build_inline_creator(self: &Rc<Self>, pending: &PendingCreate, depth: usize) -> GtkBox {
        let card = GtkBox::new(Orientation::Vertical, 5);
        card.add_css_class("inline-creator-box");
        card.set_margin_start((depth as i32 * 14) + 10);

        let kind_cell = Rc::new(Cell::new(pending.kind));

        // Top row: [File] [Folder] ............ [✕]
        let mode_row = GtkBox::new(Orientation::Horizontal, 4);

        let file_btn = Button::with_label("New File");
        file_btn.add_css_class("creator-kind-btn");

        let dir_btn = Button::with_label("New Folder");
        dir_btn.add_css_class("creator-kind-btn");

        if pending.kind == CreateKind::File {
            file_btn.add_css_class("creator-kind-btn-active");
        } else {
            dir_btn.add_css_class("creator-kind-btn-active");
        }

        let spacer = GtkBox::new(Orientation::Horizontal, 0);
        spacer.set_hexpand(true);

        let cancel_btn = Button::with_label("✕");
        cancel_btn.add_css_class("tab-close-btn");
        cancel_btn.set_tooltip_text(Some("Cancel (Esc)"));

        mode_row.append(&file_btn);
        mode_row.append(&dir_btn);
        mode_row.append(&spacer);
        mode_row.append(&cancel_btn);

        // Bottom row: [Entry] [Add]
        let input_row = GtkBox::new(Orientation::Horizontal, 4);

        let entry = Entry::new();
        entry.add_css_class("creator-entry");
        entry.set_hexpand(true);
        entry.set_text(&pending.draft_text);
        entry.set_placeholder_text(Some(match pending.kind {
            CreateKind::File => "filename.rs (Enter)",
            CreateKind::Directory => "folder_name (Enter)",
        }));

        let submit_btn = Button::with_label("Add");
        submit_btn.add_css_class("creator-submit-btn");

        input_row.append(&entry);
        input_row.append(&submit_btn);

        card.append(&mode_row);
        card.append(&input_row);

        // Toggle between File and Directory without rebuilding widget
        let update_mode_ui = {
            let file_btn_c = file_btn.clone();
            let dir_btn_c = dir_btn.clone();
            let entry_c = entry.clone();
            let kind_c = kind_cell.clone();
            Rc::new(move |new_kind: CreateKind| {
                kind_c.set(new_kind);
                if new_kind == CreateKind::File {
                    file_btn_c.add_css_class("creator-kind-btn-active");
                    dir_btn_c.remove_css_class("creator-kind-btn-active");
                    entry_c.set_placeholder_text(Some("filename.rs (Enter)"));
                } else {
                    dir_btn_c.add_css_class("creator-kind-btn-active");
                    file_btn_c.remove_css_class("creator-kind-btn-active");
                    entry_c.set_placeholder_text(Some("folder_name (Enter)"));
                }
                entry_c.grab_focus();
            })
        };

        {
            let upd = update_mode_ui.clone();
            file_btn.connect_clicked(move |_| upd(CreateKind::File));
        }
        {
            let upd = update_mode_ui.clone();
            dir_btn.connect_clicked(move |_| upd(CreateKind::Directory));
        }

        // Cancel handler
        {
            let sb_weak = Rc::downgrade(self);
            cancel_btn.connect_clicked(move |_| {
                if let Some(sb) = sb_weak.upgrade() {
                    *sb.pending_create.borrow_mut() = None;
                    sb.rebuild_tree();
                }
            });
        }

        // Submit handler
        let commit_action = {
            let sb_weak = Rc::downgrade(self);
            let parent_dir = pending.parent_dir.clone();
            let entry_c = entry.clone();
            let kind_c = kind_cell.clone();
            Rc::new(move || {
                let Some(sb) = sb_weak.upgrade() else {
                    return;
                };
                let raw_name = entry_c.text().trim().to_string();
                sb.execute_create(&parent_dir, &raw_name, kind_c.get());
            })
        };

        {
            let act = commit_action.clone();
            submit_btn.connect_clicked(move |_| act());
        }

        // Key controller on Entry: Enter = create, Esc = cancel, Tab = switch File/Folder
        {
            let key_ctrl = EventControllerKey::new();
            key_ctrl.set_propagation_phase(PropagationPhase::Capture);
            let sb_weak = Rc::downgrade(self);
            let act = commit_action;
            let upd = update_mode_ui;
            let kind_c = kind_cell;
            key_ctrl.connect_key_pressed(move |_, key, _, _| {
                if key == gdk::Key::Return || key == gdk::Key::KP_Enter {
                    act();
                    return glib::Propagation::Stop;
                }
                if key == gdk::Key::Escape {
                    if let Some(sb) = sb_weak.upgrade() {
                        *sb.pending_create.borrow_mut() = None;
                        sb.rebuild_tree();
                    }
                    return glib::Propagation::Stop;
                }
                if key == gdk::Key::Tab || key == gdk::Key::ISO_Left_Tab {
                    let next = if kind_c.get() == CreateKind::File {
                        CreateKind::Directory
                    } else {
                        CreateKind::File
                    };
                    upd(next);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            entry.add_controller(key_ctrl);
        }

        // Grab keyboard focus automatically once attached
        let entry_focus = entry.clone();
        glib::idle_add_local_once(move || {
            entry_focus.grab_focus();
        });

        card
    }

    fn execute_create(self: &Rc<Self>, parent_dir: &Path, raw_name: &str, chosen_kind: CreateKind) {
        if raw_name.is_empty() {
            (self.on_status)("Enter a file or folder name first");
            return;
        }

        // Treat trailing '/' as directory creation even if File mode was active
        let effective_kind = if raw_name.ends_with('/') {
            CreateKind::Directory
        } else {
            chosen_kind
        };

        let clean_rel = raw_name.trim_matches('/');
        if clean_rel.is_empty()
            || clean_rel
                .split('/')
                .any(|seg| seg.is_empty() || seg == "." || seg == "..")
        {
            (self.on_status)("Invalid name: relative path inside folder required");
            return;
        }

        let target_path = parent_dir.join(clean_rel);
        if target_path.exists() {
            (self.on_status)(&format!("'{}' already exists", clean_rel));
            return;
        }

        match effective_kind {
            CreateKind::Directory => match fs::create_dir_all(&target_path) {
                Ok(()) => {
                    if let Ok(canon) = fs::canonicalize(&target_path) {
                        self.expanded_dirs.borrow_mut().insert(canon);
                    }
                    *self.pending_create.borrow_mut() = None;
                    self.rebuild_tree();
                    (self.on_status)(&format!("Created folder '{clean_rel}'"));
                }
                Err(e) => {
                    (self.on_status)(&format!("Failed to create folder: {e}"));
                }
            },
            CreateKind::File => {
                if let Some(p) = target_path.parent() {
                    if !p.exists() {
                        if let Err(e) = fs::create_dir_all(p) {
                            (self.on_status)(&format!("Failed to create parent folder: {e}"));
                            return;
                        }
                    }
                }
                match OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target_path)
                {
                    Ok(_) => {
                        *self.pending_create.borrow_mut() = None;
                        self.rebuild_tree();
                        (self.on_open_file)(target_path);
                        (self.on_status)(&format!("Created file '{clean_rel}'"));
                    }
                    Err(e) => {
                        (self.on_status)(&format!("Failed to create file: {e}"));
                    }
                }
            }
        }
    }

    fn build_file_row(self: &Rc<Self>, file_path: &Path, depth: usize) -> GtkBox {
        let row = GtkBox::new(Orientation::Horizontal, 6);
        row.add_css_class("tree-row");
        row.set_margin_start((depth as i32 * 14) + 12);

        let canonical = fs::canonicalize(file_path).unwrap_or_else(|_| file_path.to_path_buf());
        if self.active_file.borrow().as_ref() == Some(&canonical) {
            row.add_css_class("tree-row-active");
        }

        let lang = Language::from_path_and_content(Some(file_path), "");
        let (badge_txt, badge_cls) = lang.badge();

        let badge = Label::new(Some(badge_txt));
        badge.add_css_class("file-badge");
        badge.add_css_class(badge_cls);
        badge.set_width_chars(2);

        let name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");
        let lbl = Label::new(Some(name));
        lbl.add_css_class("tree-file-label");
        lbl.set_halign(Align::Start);
        lbl.set_hexpand(true);
        lbl.set_ellipsize(EllipsizeMode::End);

        row.append(&badge);
        row.append(&lbl);

        let click = GestureClick::new();
        let sb_weak = Rc::downgrade(self);
        let owned = file_path.to_path_buf();
        click.connect_released(move |_, _, _, _| {
            if let Some(sb) = sb_weak.upgrade() {
                (sb.on_open_file)(owned.clone());
            }
        });
        row.add_controller(click);

        row
    }
}

fn should_skip_entry(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | "__pycache__" | ".venv" | ".idea" | ".cache"
    )
}
