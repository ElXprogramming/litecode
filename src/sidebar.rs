use std::cell::RefCell;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4::pango::EllipsizeMode;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, GestureClick, Label, Orientation, PolicyType, ScrolledWindow,
};

use crate::language::Language;

pub struct ExplorerSidebar {
    pub root: GtkBox,
    tree_box: GtkBox,
    folder_label: Label,
    current_root: RefCell<Option<PathBuf>>,
    expanded_dirs: RefCell<HashSet<PathBuf>>,
    active_file: RefCell<Option<PathBuf>>,
    on_open_file: Rc<dyn Fn(PathBuf)>,
}

impl ExplorerSidebar {
    pub fn new(
        on_open_file: Rc<dyn Fn(PathBuf)>,
        on_pick_folder: Rc<dyn Fn()>,
    ) -> Rc<Self> {
        let root = GtkBox::new(Orientation::Vertical, 0);
        root.add_css_class("sidebar-panel");
        root.set_width_request(230);
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

        // Workspace folder section header
        let folder_label = Label::new(Some("NO FOLDER OPEN"));
        folder_label.add_css_class("sidebar-workspace-name");
        folder_label.set_halign(Align::Start);
        folder_label.set_ellipsize(EllipsizeMode::End);

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
        root.append(&folder_label);
        root.append(&scrolled);

        let sidebar = Rc::new(Self {
            root,
            tree_box,
            folder_label,
            current_root: RefCell::new(None),
            expanded_dirs: RefCell::new(HashSet::new()),
            active_file: RefCell::new(None),
            on_open_file,
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
        self.expanded_dirs.borrow_mut().clear();
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
            let is_expanded = self.expanded_dirs.borrow().contains(&sub_dir);
            let row = self.build_dir_row(&sub_dir, depth, is_expanded);
            self.tree_box.append(&row);
            if is_expanded {
                self.populate_dir(&sub_dir, depth + 1);
            }
        }

        for file_path in files {
            let row = self.build_file_row(&file_path, depth);
            self.tree_box.append(&row);
        }
    }

    fn build_dir_row(self: &Rc<Self>, dir_path: &Path, depth: usize, expanded: bool) -> GtkBox {
        let row = GtkBox::new(Orientation::Horizontal, 6);
        row.add_css_class("tree-row");
        row.set_margin_start((depth as i32 * 14) + 8);

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

        row.append(&chevron);
        row.append(&lbl);

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
        row.add_controller(click);

        row
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
