use gtk4 as gtk;
use gtk::prelude::*;
use gtk::glib;
use libadwaita as adw;
use zeroize::Zeroizing;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::models::connection::ConnectionProfile;
use crate::ssh::sftp::{SftpCommand, SftpEntry, SftpEvent};

/// Create a new SFTP file browser tab connected to the given profile.
pub fn create_sftp_tab(
    tab_view: &adw::TabView,
    profile: &ConnectionProfile,
    password: Option<Zeroizing<String>>,
    key_passphrase: Option<Zeroizing<String>>,
) -> adw::TabPage {
    // Main vertical box
    let main_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    main_box.add_css_class("sftp-browser");

    // Status bar at the top
    let status_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    status_bar.set_margin_start(8);
    status_bar.set_margin_end(8);
    status_bar.set_margin_top(4);
    status_bar.set_margin_bottom(4);
    status_bar.add_css_class("sftp-status-bar");

    let status_label = gtk::Label::builder()
        .label("Connecting...")
        .halign(gtk::Align::Start)
        .hexpand(true)
        .build();
    status_label.add_css_class("dim-label");

    let transfer_label = gtk::Label::builder()
        .label("")
        .halign(gtk::Align::End)
        .build();
    transfer_label.add_css_class("dim-label");

    status_bar.append(&status_label);
    status_bar.append(&transfer_label);
    main_box.append(&status_bar);

    // Paned split view
    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    paned.set_vexpand(true);
    paned.set_hexpand(true);
    paned.set_shrink_start_child(false);
    paned.set_shrink_end_child(false);
    paned.set_resize_start_child(true);
    paned.set_resize_end_child(true);

    // Local pane
    let local_state = Rc::new(RefCell::new(LocalPaneState {
        current_path: glib::home_dir(),
    }));
    let local_pane = build_local_pane(local_state.clone());

    // Remote pane (placeholder until connected)
    let remote_entries: Rc<RefCell<Vec<SftpEntry>>> = Rc::new(RefCell::new(Vec::new()));
    let remote_path: Rc<RefCell<String>> = Rc::new(RefCell::new(String::from(".")));
    let remote_pane = build_remote_pane(remote_path.clone(), remote_entries.clone());

    paned.set_start_child(Some(&local_pane.container));
    paned.set_end_child(Some(&remote_pane.container));

    main_box.append(&paned);

    // Transfer buttons bar
    let transfer_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    transfer_bar.set_margin_start(8);
    transfer_bar.set_margin_end(8);
    transfer_bar.set_margin_top(4);
    transfer_bar.set_margin_bottom(4);
    transfer_bar.set_halign(gtk::Align::Center);

    let upload_btn = gtk::Button::builder()
        .label("Upload →")
        .tooltip_text("Upload selected local file to remote directory")
        .sensitive(false)
        .build();
    upload_btn.add_css_class("suggested-action");

    let download_btn = gtk::Button::builder()
        .label("← Download")
        .tooltip_text("Download selected remote file to local directory")
        .sensitive(false)
        .build();
    download_btn.add_css_class("suggested-action");

    transfer_bar.append(&upload_btn);
    transfer_bar.append(&download_btn);
    main_box.append(&transfer_bar);

    let page = tab_view.append(&main_box);
    page.set_title(&format!("SFTP - {}", profile.name));
    page.set_icon(Some(&gtk::gio::ThemedIcon::new("folder-symbolic")));

    // Set up SFTP channels
    let (event_tx, event_rx) = async_channel::bounded::<SftpEvent>(256);

    let cmd_tx = crate::ssh::sftp::spawn_sftp_session(
        profile.clone(),
        password,
        key_passphrase,
        event_tx,
    );

    let cmd_tx_rc = Rc::new(cmd_tx);

    // Enable transfer buttons once connected
    let upload_btn_rc = upload_btn.clone();
    let download_btn_rc = download_btn.clone();

    // Upload button handler
    let cmd_tx_upload = cmd_tx_rc.clone();
    let local_state_upload = local_state.clone();
    let local_list_upload = local_pane.listbox.clone();
    let remote_path_upload = remote_path.clone();
    let cmd_tx_refresh_upload = cmd_tx_rc.clone();
    upload_btn.connect_clicked(move |_| {
        let selected = local_list_upload.selected_row();
        if let Some(row) = selected {
            let name = get_row_name(&row);
            if let Some(name) = name {
                let local_path = local_state_upload.borrow().current_path.join(&name);
                let rpath = remote_path_upload.borrow().clone();
                let remote = format!("{}/{}", rpath, name);
                let tx = (*cmd_tx_upload).clone();
                let tx2 = (*cmd_tx_refresh_upload).clone();
                let rpath2 = rpath.clone();
                glib::spawn_future_local(async move {
                    let _ = tx.send(SftpCommand::Upload {
                        local: local_path,
                        remote,
                    }).await;
                    // Refresh remote listing after upload
                    let _ = tx2.send(SftpCommand::ListDir(rpath2)).await;
                });
            }
        }
    });

    // Download button handler
    let cmd_tx_download = cmd_tx_rc.clone();
    let local_state_download = local_state.clone();
    let remote_list_download = remote_pane.listbox.clone();
    let remote_path_download = remote_path.clone();
    let local_pane_refresh = local_pane.clone();
    let local_state_refresh = local_state.clone();
    download_btn.connect_clicked(move |_| {
        let selected = remote_list_download.selected_row();
        if let Some(row) = selected {
            let name = get_row_name(&row);
            if let Some(name) = name {
                let rpath = remote_path_download.borrow().clone();
                let remote = format!("{}/{}", rpath, name);
                let local = local_state_download.borrow().current_path.clone();
                let tx = (*cmd_tx_download).clone();
                let ls = local_state_refresh.clone();
                let lp = local_pane_refresh.clone();
                glib::spawn_future_local(async move {
                    let _ = tx.send(SftpCommand::Download {
                        remote,
                        local,
                    }).await;
                    // Small delay then refresh local listing
                    glib::timeout_future(std::time::Duration::from_millis(500)).await;
                    let path = ls.borrow().current_path.clone();
                    refresh_local_listing(&lp, &path);
                });
            }
        }
    });

    // Wire local pane navigation
    wire_local_navigation(&local_pane, local_state.clone());

    // Wire remote pane navigation
    wire_remote_navigation(
        &remote_pane,
        remote_path.clone(),
        remote_entries.clone(),
        cmd_tx_rc.clone(),
    );

    // Poll SFTP events
    let remote_pane_events = remote_pane.clone();
    let remote_path_events = remote_path.clone();
    let remote_entries_events = remote_entries.clone();
    let status_label_c = status_label.clone();
    let transfer_label_c = transfer_label.clone();
    glib::spawn_future_local(async move {
        while let Ok(event) = event_rx.recv().await {
            match event {
                SftpEvent::Connected => {
                    status_label_c.set_label("Connected");
                    upload_btn_rc.set_sensitive(true);
                    download_btn_rc.set_sensitive(true);
                    // Request initial directory listing
                    let tx = (*cmd_tx_rc).clone();
                    let rp = remote_path_events.borrow().clone();
                    glib::spawn_future_local(async move {
                        let _ = tx.send(SftpCommand::ListDir(rp)).await;
                    });
                }
                SftpEvent::DirListing { path, entries } => {
                    *remote_path_events.borrow_mut() = path.clone();
                    *remote_entries_events.borrow_mut() = entries.clone();
                    remote_pane_events.path_entry.set_text(&path);
                    populate_remote_listbox(&remote_pane_events.listbox, &entries);
                }
                SftpEvent::TransferProgress { name, bytes, total } => {
                    if total > 0 {
                        let pct = (bytes as f64 / total as f64 * 100.0) as u32;
                        transfer_label_c.set_label(&format!("{name}: {pct}%"));
                    } else {
                        transfer_label_c.set_label(&format!("{name}: {bytes} bytes"));
                    }
                }
                SftpEvent::TransferComplete { name } => {
                    transfer_label_c.set_label(&format!("{name}: complete"));
                }
                SftpEvent::Error(msg) => {
                    status_label_c.set_label(&format!("Error: {msg}"));
                }
                SftpEvent::Disconnected => {
                    status_label_c.set_label("Disconnected");
                    upload_btn_rc.set_sensitive(false);
                    download_btn_rc.set_sensitive(false);
                    break;
                }
            }
        }
    });

    page
}

struct LocalPaneState {
    current_path: PathBuf,
}

#[derive(Clone)]
struct PaneWidgets {
    container: gtk::Box,
    path_entry: gtk::Entry,
    listbox: gtk::ListBox,
    up_btn: gtk::Button,
    home_btn: gtk::Button,
    refresh_btn: gtk::Button,
}

fn build_local_pane(state: Rc<RefCell<LocalPaneState>>) -> PaneWidgets {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    container.add_css_class("sftp-pane");

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    header.set_margin_start(4);
    header.set_margin_end(4);
    header.set_margin_top(4);
    header.set_margin_bottom(4);

    let pane_label = gtk::Label::builder()
        .label("Local")
        .css_classes(["heading"])
        .build();
    header.append(&pane_label);

    container.append(&header);

    // Navigation bar
    let nav_bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    nav_bar.set_margin_start(4);
    nav_bar.set_margin_end(4);
    nav_bar.set_margin_bottom(4);

    let up_btn = gtk::Button::builder()
        .icon_name("go-up-symbolic")
        .tooltip_text("Parent directory")
        .css_classes(["flat"])
        .build();

    let home_btn = gtk::Button::builder()
        .icon_name("go-home-symbolic")
        .tooltip_text("Home directory")
        .css_classes(["flat"])
        .build();

    let refresh_btn = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Refresh")
        .css_classes(["flat"])
        .build();

    let path_entry = gtk::Entry::builder()
        .hexpand(true)
        .text(state.borrow().current_path.to_string_lossy().as_ref())
        .build();
    path_entry.add_css_class("sftp-path-entry");

    nav_bar.append(&up_btn);
    nav_bar.append(&home_btn);
    nav_bar.append(&refresh_btn);
    nav_bar.append(&path_entry);
    container.append(&nav_bar);

    let listbox = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .build();
    listbox.add_css_class("sftp-file-list");

    let scrolled = gtk::ScrolledWindow::builder()
        .child(&listbox)
        .vexpand(true)
        .build();
    container.append(&scrolled);

    let pane = PaneWidgets {
        container,
        path_entry,
        listbox,
        up_btn,
        home_btn,
        refresh_btn,
    };

    // Initial listing
    let path = state.borrow().current_path.clone();
    refresh_local_listing(&pane, &path);

    pane
}

fn build_remote_pane(
    remote_path: Rc<RefCell<String>>,
    _remote_entries: Rc<RefCell<Vec<SftpEntry>>>,
) -> PaneWidgets {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    container.add_css_class("sftp-pane");

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    header.set_margin_start(4);
    header.set_margin_end(4);
    header.set_margin_top(4);
    header.set_margin_bottom(4);

    let pane_label = gtk::Label::builder()
        .label("Remote")
        .css_classes(["heading"])
        .build();
    header.append(&pane_label);

    container.append(&header);

    // Navigation bar
    let nav_bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    nav_bar.set_margin_start(4);
    nav_bar.set_margin_end(4);
    nav_bar.set_margin_bottom(4);

    let up_btn = gtk::Button::builder()
        .icon_name("go-up-symbolic")
        .tooltip_text("Parent directory")
        .css_classes(["flat"])
        .build();

    let home_btn = gtk::Button::builder()
        .icon_name("go-home-symbolic")
        .tooltip_text("Home directory")
        .css_classes(["flat"])
        .build();

    let refresh_btn = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Refresh")
        .css_classes(["flat"])
        .build();

    let path_entry = gtk::Entry::builder()
        .hexpand(true)
        .text(remote_path.borrow().as_str())
        .build();
    path_entry.add_css_class("sftp-path-entry");

    nav_bar.append(&up_btn);
    nav_bar.append(&home_btn);
    nav_bar.append(&refresh_btn);
    nav_bar.append(&path_entry);
    container.append(&nav_bar);

    let listbox = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .build();
    listbox.add_css_class("sftp-file-list");

    // Placeholder
    let placeholder = gtk::Label::builder()
        .label("Connecting...")
        .css_classes(["dim-label"])
        .margin_top(24)
        .margin_bottom(24)
        .build();
    listbox.set_placeholder(Some(&placeholder));

    let scrolled = gtk::ScrolledWindow::builder()
        .child(&listbox)
        .vexpand(true)
        .build();
    container.append(&scrolled);

    PaneWidgets {
        container,
        path_entry,
        listbox,
        up_btn,
        home_btn,
        refresh_btn,
    }
}

fn refresh_local_listing(pane: &PaneWidgets, path: &PathBuf) {
    // Clear existing entries
    while let Some(child) = pane.listbox.first_child() {
        pane.listbox.remove(&child);
    }

    pane.path_entry.set_text(&path.to_string_lossy());

    match std::fs::read_dir(path) {
        Ok(entries) => {
            let mut items: Vec<(String, bool, u64, Option<u64>)> = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let metadata = entry.metadata().ok();
                let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
                let modified = metadata
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());
                items.push((name, is_dir, size, modified));
            }

            // Sort: directories first, then alphabetical
            items.sort_by(|a, b| {
                b.1.cmp(&a.1).then(a.0.to_lowercase().cmp(&b.0.to_lowercase()))
            });

            for (name, is_dir, size, _modified) in &items {
                let row = create_file_row(name, *is_dir, *size);
                pane.listbox.append(&row);
            }
        }
        Err(e) => {
            let label = gtk::Label::builder()
                .label(&format!("Error: {e}"))
                .css_classes(["dim-label"])
                .margin_top(12)
                .margin_bottom(12)
                .build();
            pane.listbox.append(&label);
        }
    }
}

fn populate_remote_listbox(listbox: &gtk::ListBox, entries: &[SftpEntry]) {
    while let Some(child) = listbox.first_child() {
        listbox.remove(&child);
    }

    if entries.is_empty() {
        let label = gtk::Label::builder()
            .label("(empty directory)")
            .css_classes(["dim-label"])
            .margin_top(12)
            .margin_bottom(12)
            .build();
        listbox.append(&label);
        return;
    }

    for entry in entries {
        let row = create_file_row(&entry.name, entry.is_dir, entry.size);
        listbox.append(&row);
    }
}

fn create_file_row(name: &str, is_dir: bool, size: u64) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();

    let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    hbox.set_margin_start(8);
    hbox.set_margin_end(8);
    hbox.set_margin_top(4);
    hbox.set_margin_bottom(4);

    let icon_name = if is_dir {
        "folder-symbolic"
    } else {
        "text-x-generic-symbolic"
    };

    let icon = gtk::Image::from_icon_name(icon_name);
    hbox.append(&icon);

    let name_label = gtk::Label::builder()
        .label(name)
        .halign(gtk::Align::Start)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build();
    // Store the file name as widget name for retrieval
    name_label.set_widget_name(name);
    hbox.append(&name_label);

    if !is_dir {
        let size_str = format_size(size);
        let size_label = gtk::Label::builder()
            .label(&size_str)
            .halign(gtk::Align::End)
            .css_classes(["dim-label", "caption"])
            .build();
        hbox.append(&size_label);
    }

    row.set_child(Some(&hbox));
    // Store the entry name and type in the row
    row.set_widget_name(&format!("{}:{}", if is_dir { "d" } else { "f" }, name));
    row
}

fn get_row_name(row: &gtk::ListBoxRow) -> Option<String> {
    let widget_name = row.widget_name().to_string();
    // Format is "d:name" or "f:name"
    widget_name.get(2..).map(|s| s.to_string())
}

fn is_row_dir(row: &gtk::ListBoxRow) -> bool {
    row.widget_name().starts_with("d:")
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn wire_local_navigation(pane: &PaneWidgets, state: Rc<RefCell<LocalPaneState>>) {
    // Double-click / row activation to navigate into directory
    let state_activate = state.clone();
    let pane_activate = pane.clone();
    pane.listbox.connect_row_activated(move |_, row| {
        if is_row_dir(row) {
            if let Some(name) = get_row_name(row) {
                let new_path = state_activate.borrow().current_path.join(&name);
                if new_path.is_dir() {
                    state_activate.borrow_mut().current_path = new_path.clone();
                    refresh_local_listing(&pane_activate, &new_path);
                }
            }
        }
    });

    // Up button
    let state_up = state.clone();
    let pane_up = pane.clone();
    pane.up_btn.connect_clicked(move |_| {
        let parent = state_up.borrow().current_path.parent().map(|p| p.to_path_buf());
        if let Some(parent) = parent {
            state_up.borrow_mut().current_path = parent.clone();
            refresh_local_listing(&pane_up, &parent);
        }
    });

    // Home button
    let state_home = state.clone();
    let pane_home = pane.clone();
    pane.home_btn.connect_clicked(move |_| {
        let home = glib::home_dir();
        state_home.borrow_mut().current_path = home.clone();
        refresh_local_listing(&pane_home, &home);
    });

    // Refresh button
    let state_refresh = state.clone();
    let pane_refresh = pane.clone();
    pane.refresh_btn.connect_clicked(move |_| {
        let path = state_refresh.borrow().current_path.clone();
        refresh_local_listing(&pane_refresh, &path);
    });

    // Path entry activation (Enter key)
    let state_entry = state.clone();
    let pane_entry = pane.clone();
    pane.path_entry.connect_activate(move |entry| {
        let text = entry.text().to_string();
        let new_path = PathBuf::from(&text);
        if new_path.is_dir() {
            state_entry.borrow_mut().current_path = new_path.clone();
            refresh_local_listing(&pane_entry, &new_path);
        }
    });
}

fn wire_remote_navigation(
    pane: &PaneWidgets,
    remote_path: Rc<RefCell<String>>,
    _remote_entries: Rc<RefCell<Vec<SftpEntry>>>,
    cmd_tx: Rc<async_channel::Sender<SftpCommand>>,
) {
    // Double-click to navigate into directory
    let remote_path_activate = remote_path.clone();
    let cmd_tx_activate = cmd_tx.clone();
    pane.listbox.connect_row_activated(move |_, row| {
        if is_row_dir(row) {
            if let Some(name) = get_row_name(row) {
                let current = remote_path_activate.borrow().clone();
                let new_path = format!("{}/{}", current, name);
                let tx = (*cmd_tx_activate).clone();
                glib::spawn_future_local(async move {
                    let _ = tx.send(SftpCommand::ListDir(new_path)).await;
                });
            }
        }
    });

    // Up button
    let remote_path_up = remote_path.clone();
    let cmd_tx_up = cmd_tx.clone();
    pane.up_btn.connect_clicked(move |_| {
        let current = remote_path_up.borrow().clone();
        let parent = if current.contains('/') {
            let pos = current.rfind('/').unwrap();
            if pos == 0 {
                "/".to_string()
            } else {
                current[..pos].to_string()
            }
        } else {
            ".".to_string()
        };
        let tx = (*cmd_tx_up).clone();
        glib::spawn_future_local(async move {
            let _ = tx.send(SftpCommand::ListDir(parent)).await;
        });
    });

    // Home button
    let cmd_tx_home = cmd_tx.clone();
    pane.home_btn.connect_clicked(move |_| {
        let tx = (*cmd_tx_home).clone();
        glib::spawn_future_local(async move {
            let _ = tx.send(SftpCommand::ListDir(".".to_string())).await;
        });
    });

    // Refresh button
    let remote_path_refresh = remote_path.clone();
    let cmd_tx_refresh = cmd_tx.clone();
    pane.refresh_btn.connect_clicked(move |_| {
        let path = remote_path_refresh.borrow().clone();
        let tx = (*cmd_tx_refresh).clone();
        glib::spawn_future_local(async move {
            let _ = tx.send(SftpCommand::ListDir(path)).await;
        });
    });

    // Path entry activation
    let cmd_tx_entry = cmd_tx.clone();
    pane.path_entry.connect_activate(move |entry| {
        let path = entry.text().to_string();
        let tx = (*cmd_tx_entry).clone();
        glib::spawn_future_local(async move {
            let _ = tx.send(SftpCommand::ListDir(path)).await;
        });
    });
}
