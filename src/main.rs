#[allow(dead_code)]
mod app;
#[allow(dead_code)]
mod config;
#[allow(dead_code)]
mod error;
#[allow(dead_code)]
mod keys;
#[allow(dead_code)]
mod models;
#[allow(dead_code)]
mod ssh;
#[allow(dead_code)]
mod storage;
mod ui;

use gtk4 as gtk;
use gtk::prelude::*;
use libadwaita as adw;
use std::sync::OnceLock;

use app::SharedState;

static TOKIO_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

pub fn runtime() -> &'static tokio::runtime::Runtime {
    TOKIO_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime")
    })
}

fn main() {
    env_logger::init();

    if let Err(e) = config::ensure_directories() {
        eprintln!("Failed to create application directories: {e}");
        std::process::exit(1);
    }

    // Initialize the Tokio runtime eagerly
    let _ = runtime();

    let app = adw::Application::builder()
        .application_id("com.grustyssh.app")
        .build();

    app.connect_startup(|_| {
        log::info!("GrustySSH starting up");
        gtk::Window::set_default_icon_name("grustyssh");
    });

    app.connect_activate(move |app| {
        let state = SharedState::new();
        let window = ui::window::build_window(app, state);
        window.present();
    });

    app.run();
}
