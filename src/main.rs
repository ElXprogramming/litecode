mod app;
mod editor;
mod formatter;
mod language;
mod sidebar;
mod syntax;
mod terminal;

use std::path::PathBuf;

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::Application;

use crate::app::LiteCodeUi;

const APP_ID: &str = "org.archlinux.litecode";

fn main() -> glib::ExitCode {
    let raw_args: Vec<String> = std::env::args().collect();
    if raw_args.iter().any(|a| a == "--version" || a == "-V") {
        println!("litecode {}", env!("CARGO_PKG_VERSION"));
        return glib::ExitCode::SUCCESS;
    }

    // Force GTK4's ultra-light 2D Cairo renderer by default (even if global GSK_RENDERER=gl is set),
    // saving ~90 MB of OpenGL/Mesa shader memory (~17 MB private RAM). Set LITECODE_GPU=1 to opt into GPU.
    if std::env::var_os("LITECODE_GPU").is_none() {
        std::env::set_var("GSK_RENDERER", "cairo");
    }

    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN | gio::ApplicationFlags::NON_UNIQUE)
        .build();

    app.connect_activate(|app| {
        let ui = LiteCodeUi::build(app, &[]);
        ui.window.present();
    });

    app.connect_open(|app, files, _hint| {
        let paths: Vec<PathBuf> = files.iter().filter_map(|f| f.path()).collect();
        let ui = LiteCodeUi::build(app, &paths);
        ui.window.present();
    });

    app.run()
}
