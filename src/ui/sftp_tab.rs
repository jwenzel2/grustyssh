use gtk4 as gtk;
use gtk::prelude::*;
use gtk::glib;
use libadwaita as adw;
use adw::prelude::*;
use zeroize::Zeroizing;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use crate::models::connection::ConnectionProfile;
use crate::ssh::sftp::{
    SftpCommand,
    SftpConflictDecision,
    SftpConflictDirection,
    SftpConflictResponse,
    SftpEntry,
    SftpEvent,
};

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
    wire_toggle_deselect_on_second_click(&local_pane.listbox);
    wire_toggle_deselect_on_second_click(&remote_pane.listbox);

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
        .tooltip_text("Upload selected local file or directory to remote")
        .sensitive(false)
        .build();
    upload_btn.add_css_class("suggested-action");

    let download_btn = gtk::Button::builder()
        .label("← Download")
        .tooltip_text("Download selected remote file or directory to local")
        .sensitive(false)
        .build();
    download_btn.add_css_class("suggested-action");

    let delete_btn = gtk::Button::builder()
        .label("Delete Selected")
        .tooltip_text("Delete selected local and remote files/directories")
        .sensitive(false)
        .build();
    delete_btn.add_css_class("destructive-action");

    transfer_bar.append(&upload_btn);
    transfer_bar.append(&download_btn);
    transfer_bar.append(&delete_btn);
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
    let remote_connected = Rc::new(Cell::new(false));

    // Enable transfer buttons once connected
    let upload_btn_rc = upload_btn.clone();
    let download_btn_rc = download_btn.clone();

    let upload_action: Rc<dyn Fn()> = {
        let local_list_upload = local_pane.listbox.clone();
        let local_state_upload = local_state.clone();
        let remote_path_upload = remote_path.clone();
        let cmd_tx_upload = cmd_tx_rc.clone();
        Rc::new(move || {
            upload_selected_local_entry(
                &local_list_upload,
                local_state_upload.clone(),
                remote_path_upload.clone(),
                cmd_tx_upload.clone(),
            );
        })
    };
    let upload_action_btn = upload_action.clone();
    upload_btn.connect_clicked(move |_| {
        upload_action_btn();
    });

    let download_action: Rc<dyn Fn()> = {
        let remote_list_download = remote_pane.listbox.clone();
        let remote_path_download = remote_path.clone();
        let local_state_download = local_state.clone();
        let local_pane_refresh = local_pane.clone();
        let cmd_tx_download = cmd_tx_rc.clone();
        Rc::new(move || {
            download_selected_remote_entry(
                &remote_list_download,
                remote_path_download.clone(),
                local_state_download.clone(),
                local_pane_refresh.clone(),
                cmd_tx_download.clone(),
            );
        })
    };
    let download_action_btn = download_action.clone();
    download_btn.connect_clicked(move |_| {
        download_action_btn();
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

    // Local pane right-click actions
    let local_delete_action: Rc<dyn Fn()> = {
        let local_list_delete = local_pane.listbox.clone();
        let local_state_delete = local_state.clone();
        let local_pane_delete = local_pane.clone();
        let status_label_delete = status_label.clone();
        Rc::new(move || {
            delete_selected_local_entries(
                &local_list_delete,
                local_state_delete.clone(),
                local_pane_delete.clone(),
                status_label_delete.clone(),
            );
        })
    };
    let local_rename_action: Rc<dyn Fn()> = {
        let local_list_rename = local_pane.listbox.clone();
        let local_state_rename = local_state.clone();
        let local_pane_rename = local_pane.clone();
        let status_label_rename = status_label.clone();
        Rc::new(move || {
            rename_selected_local_entry(
                &local_list_rename,
                local_state_rename.clone(),
                local_pane_rename.clone(),
                status_label_rename.clone(),
            );
        })
    };

    let local_context_popover = gtk::Popover::builder()
        .autohide(true)
        .has_arrow(false)
        .build();
    local_context_popover.set_parent(&local_pane.listbox);

    let local_context_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let local_context_upload_btn = gtk::Button::builder()
        .label("Upload")
        .halign(gtk::Align::Start)
        .css_classes(["flat"])
        .build();
    let local_context_rename_btn = gtk::Button::builder()
        .label("Rename")
        .halign(gtk::Align::Start)
        .css_classes(["flat"])
        .build();
    let local_context_delete_btn = gtk::Button::builder()
        .label("Delete")
        .halign(gtk::Align::Start)
        .css_classes(["flat", "destructive-action"])
        .build();
    local_context_box.append(&local_context_upload_btn);
    local_context_box.append(&local_context_rename_btn);
    local_context_box.append(&local_context_delete_btn);
    local_context_popover.set_child(Some(&local_context_box));

    let local_context_popover_upload = local_context_popover.clone();
    let upload_action_context_local = upload_action.clone();
    local_context_upload_btn.connect_clicked(move |_| {
        local_context_popover_upload.popdown();
        upload_action_context_local();
    });

    let local_context_popover_rename = local_context_popover.clone();
    let local_rename_action_context = local_rename_action.clone();
    local_context_rename_btn.connect_clicked(move |_| {
        local_context_popover_rename.popdown();
        local_rename_action_context();
    });

    let local_context_popover_delete = local_context_popover.clone();
    let local_delete_action_context = local_delete_action.clone();
    local_context_delete_btn.connect_clicked(move |_| {
        local_context_popover_delete.popdown();
        local_delete_action_context();
    });

    let local_right_click = gtk::GestureClick::builder()
        .button(3)
        .build();
    let local_list_rclick = local_pane.listbox.clone();
    let local_context_popover_rclick = local_context_popover.clone();
    let local_context_upload_btn_rclick = local_context_upload_btn.clone();
    let local_context_rename_btn_rclick = local_context_rename_btn.clone();
    let local_context_delete_btn_rclick = local_context_delete_btn.clone();
    let remote_connected_local_rclick = remote_connected.clone();
    local_right_click.connect_pressed(move |_, _, x, y| {
        let Some(row) = local_list_rclick.row_at_y(y as i32) else {
            return;
        };
        local_list_rclick.select_row(Some(&row));

        local_context_upload_btn_rclick
            .set_sensitive(remote_connected_local_rclick.get());
        local_context_rename_btn_rclick.set_sensitive(get_row_name(&row).is_some());
        local_context_delete_btn_rclick.set_sensitive(get_row_name(&row).is_some());

        let rect = gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
        local_context_popover_rclick.set_pointing_to(Some(&rect));
        local_context_popover_rclick.popup();
    });
    local_pane.listbox.add_controller(local_right_click);

    // Remote pane delete action
    let remote_delete_action: Rc<dyn Fn()> = {
        let remote_list = remote_pane.listbox.clone();
        let remote_path_delete = remote_path.clone();
        let cmd_tx_delete = cmd_tx_rc.clone();
        let remote_connected_delete = remote_connected.clone();
        Rc::new(move || {
            if !remote_connected_delete.get() {
                return;
            }
            delete_selected_remote_entries(
                &remote_list,
                remote_path_delete.clone(),
                cmd_tx_delete.clone(),
            );
        })
    };
    let remote_rename_action: Rc<dyn Fn()> = {
        let remote_list_rename = remote_pane.listbox.clone();
        let remote_path_rename = remote_path.clone();
        let cmd_tx_rename = cmd_tx_rc.clone();
        let remote_connected_rename = remote_connected.clone();
        Rc::new(move || {
            rename_selected_remote_entry(
                &remote_list_rename,
                remote_path_rename.clone(),
                cmd_tx_rename.clone(),
                remote_connected_rename.get(),
            );
        })
    };

    let delete_action_btn_local = local_delete_action.clone();
    let delete_action_btn_remote = remote_delete_action.clone();
    delete_btn.connect_clicked(move |_| {
        delete_action_btn_local();
        delete_action_btn_remote();
    });

    let delete_btn_selection = delete_btn.clone();
    let local_list_selection = local_pane.listbox.clone();
    let remote_list_selection = remote_pane.listbox.clone();
    let remote_connected_selection = remote_connected.clone();
    local_pane.listbox.connect_selected_rows_changed(move |_| {
        update_delete_button_state(
            &delete_btn_selection,
            &local_list_selection,
            &remote_list_selection,
            remote_connected_selection.get(),
        );
    });

    let delete_btn_selection = delete_btn.clone();
    let local_list_selection = local_pane.listbox.clone();
    let remote_list_selection = remote_pane.listbox.clone();
    let remote_connected_selection = remote_connected.clone();
    remote_pane.listbox.connect_selected_rows_changed(move |_| {
        update_delete_button_state(
            &delete_btn_selection,
            &local_list_selection,
            &remote_list_selection,
            remote_connected_selection.get(),
        );
    });

    // Remote pane right-click actions
    let remote_context_popover = gtk::Popover::builder()
        .autohide(true)
        .has_arrow(false)
        .build();
    remote_context_popover.set_parent(&remote_pane.listbox);

    let remote_context_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let remote_context_download_btn = gtk::Button::builder()
        .label("Download")
        .halign(gtk::Align::Start)
        .css_classes(["flat"])
        .build();
    let remote_context_rename_btn = gtk::Button::builder()
        .label("Rename")
        .halign(gtk::Align::Start)
        .css_classes(["flat"])
        .build();
    let remote_context_delete_btn = gtk::Button::builder()
        .label("Delete Selected")
        .halign(gtk::Align::Start)
        .css_classes(["flat", "destructive-action"])
        .build();
    remote_context_box.append(&remote_context_download_btn);
    remote_context_box.append(&remote_context_rename_btn);
    remote_context_box.append(&remote_context_delete_btn);
    remote_context_popover.set_child(Some(&remote_context_box));

    let remote_context_popover_download = remote_context_popover.clone();
    let download_action_context_remote = download_action.clone();
    remote_context_download_btn.connect_clicked(move |_| {
        remote_context_popover_download.popdown();
        download_action_context_remote();
    });

    let remote_context_popover_rename = remote_context_popover.clone();
    let remote_rename_action_context = remote_rename_action.clone();
    remote_context_rename_btn.connect_clicked(move |_| {
        remote_context_popover_rename.popdown();
        remote_rename_action_context();
    });

    let remote_context_popover_delete = remote_context_popover.clone();
    let delete_action_context = remote_delete_action.clone();
    remote_context_delete_btn.connect_clicked(move |_| {
        remote_context_popover_delete.popdown();
        delete_action_context();
    });

    let right_click = gtk::GestureClick::builder()
        .button(3)
        .build();
    let remote_list_rclick = remote_pane.listbox.clone();
    let remote_context_popover_rclick = remote_context_popover.clone();
    let remote_context_download_btn_rclick = remote_context_download_btn.clone();
    let remote_context_rename_btn_rclick = remote_context_rename_btn.clone();
    let remote_context_delete_btn_rclick = remote_context_delete_btn.clone();
    let remote_connected_rclick = remote_connected.clone();
    right_click.connect_pressed(move |_, _, x, y| {
        if !remote_connected_rclick.get() {
            return;
        }

        let Some(row) = remote_list_rclick.row_at_y(y as i32) else {
            return;
        };

        if !row.is_selected() {
            remote_list_rclick.unselect_all();
            remote_list_rclick.select_row(Some(&row));
        }

        let has_selected = !get_selected_row_names(&remote_list_rclick).is_empty();
        remote_context_delete_btn_rclick.set_sensitive(has_selected);
        remote_context_download_btn_rclick
            .set_sensitive(can_download_selected_remote_entries(&remote_list_rclick));
        remote_context_rename_btn_rclick
            .set_sensitive(can_rename_selected_remote_entry(&remote_list_rclick));

        let rect = gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
        remote_context_popover_rclick.set_pointing_to(Some(&rect));
        remote_context_popover_rclick.popup();
    });
    remote_pane.listbox.add_controller(right_click);

    // Poll SFTP events
    let remote_pane_events = remote_pane.clone();
    let remote_path_events = remote_path.clone();
    let remote_entries_events = remote_entries.clone();
    let status_label_c = status_label.clone();
    let transfer_label_c = transfer_label.clone();
    let delete_btn_c = delete_btn.clone();
    let remote_connected_c = remote_connected.clone();
    let local_list_events = local_pane.listbox.clone();
    let conflict_anchor = main_box.clone();
    glib::spawn_future_local(async move {
        while let Ok(event) = event_rx.recv().await {
            match event {
                SftpEvent::Connected => {
                    remote_connected_c.set(true);
                    status_label_c.set_label("Connected");
                    upload_btn_rc.set_sensitive(true);
                    download_btn_rc.set_sensitive(true);
                    update_delete_button_state(
                        &delete_btn_c,
                        &local_list_events,
                        &remote_pane_events.listbox,
                        remote_connected_c.get(),
                    );
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
                    update_delete_button_state(
                        &delete_btn_c,
                        &local_list_events,
                        &remote_pane_events.listbox,
                        remote_connected_c.get(),
                    );
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
                SftpEvent::TransferConflict {
                    path,
                    direction,
                    is_dir,
                    response_tx,
                } => {
                    prompt_transfer_conflict_dialog(
                        &conflict_anchor,
                        &path,
                        direction,
                        is_dir,
                        response_tx,
                    );
                }
                SftpEvent::Error(msg) => {
                    status_label_c.set_label(&format!("Error: {msg}"));
                }
                SftpEvent::Disconnected => {
                    remote_connected_c.set(false);
                    status_label_c.set_label("Disconnected");
                    upload_btn_rc.set_sensitive(false);
                    download_btn_rc.set_sensitive(false);
                    update_delete_button_state(
                        &delete_btn_c,
                        &local_list_events,
                        &remote_pane_events.listbox,
                        remote_connected_c.get(),
                    );
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
    listbox.set_activate_on_single_click(false);
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
        .selection_mode(gtk::SelectionMode::Multiple)
        .build();
    listbox.set_activate_on_single_click(false);
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
    while let Some(row) = pane.listbox.row_at_index(0) {
        pane.listbox.remove(&row);
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
    while let Some(row) = listbox.row_at_index(0) {
        listbox.remove(&row);
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

fn get_selected_row_names(listbox: &gtk::ListBox) -> Vec<String> {
    listbox
        .selected_rows()
        .into_iter()
        .filter_map(|row| get_row_name(&row))
        .collect()
}

fn wire_toggle_deselect_on_second_click(listbox: &gtk::ListBox) {
    let toggle_click = gtk::GestureClick::builder()
        .button(1)
        .build();
    toggle_click.set_propagation_phase(gtk::PropagationPhase::Capture);

    let listbox_toggle = listbox.clone();
    toggle_click.connect_pressed(move |gesture, n_press, _x, y| {
        if n_press != 1 {
            return;
        }

        let Some(row) = listbox_toggle.row_at_y(y as i32) else {
            return;
        };
        if !row.is_selected() {
            return;
        }

        listbox_toggle.unselect_row(&row);
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    listbox.add_controller(toggle_click);
}

fn join_remote_path(base: &str, name: &str) -> String {
    if base == "/" {
        format!("/{name}")
    } else if base.ends_with('/') {
        format!("{base}{name}")
    } else {
        format!("{base}/{name}")
    }
}

fn update_delete_button_state(
    delete_btn: &gtk::Button,
    local_list: &gtk::ListBox,
    remote_list: &gtk::ListBox,
    connected: bool,
) {
    let has_local_selected = !local_list.selected_rows().is_empty();
    let has_remote_selected = connected && !get_selected_row_names(remote_list).is_empty();
    delete_btn.set_sensitive(has_local_selected || has_remote_selected);
}

fn prompt_transfer_conflict_dialog(
    anchor: &impl IsA<gtk::Widget>,
    path: &str,
    direction: SftpConflictDirection,
    is_dir: bool,
    response_tx: async_channel::Sender<SftpConflictResponse>,
) {
    let item_type = if is_dir { "folder" } else { "file" };
    let transfer_direction = match direction {
        SftpConflictDirection::Upload => "uploading to remote",
        SftpConflictDirection::Download => "downloading to local",
    };

    let dialog = adw::AlertDialog::builder()
        .heading("Conflict Detected")
        .body(&format!(
            "A {item_type} already exists while {transfer_direction}:\n{path}\n\nChoose which version to keep."
        ))
        .build();

    dialog.add_response("keep", "Keep Existing");
    dialog.add_response("replace", "Keep Incoming");
    dialog.set_response_appearance("replace", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("keep"));

    let apply_all_check = gtk::CheckButton::builder()
        .label("Apply this choice to all remaining conflicts in this transfer")
        .halign(gtk::Align::Start)
        .build();
    dialog.set_extra_child(Some(&apply_all_check));

    let response_tx_dialog = response_tx.clone();
    let apply_all_check_dialog = apply_all_check.clone();
    dialog.connect_response(None, move |_dialog, response| {
        let decision = if response == "replace" {
            SftpConflictDecision::ReplaceWithIncoming
        } else {
            SftpConflictDecision::KeepExisting
        };
        let response_payload = SftpConflictResponse {
            decision,
            apply_to_all: apply_all_check_dialog.is_active(),
        };
        let tx = response_tx_dialog.clone();
        glib::spawn_future_local(async move {
            let _ = tx.send(response_payload).await;
        });
    });

    if let Some(root) = anchor.as_ref().root() {
        if let Ok(window) = root.downcast::<gtk::Window>() {
            dialog.present(Some(&window));
            return;
        }
    }

    glib::spawn_future_local(async move {
        let _ = response_tx.send(SftpConflictResponse {
            decision: SftpConflictDecision::KeepExisting,
            apply_to_all: false,
        }).await;
    });
}

fn prompt_rename_dialog(
    anchor: &impl IsA<gtk::Widget>,
    current_name: &str,
    on_submit: impl FnOnce(String) + 'static,
) {
    let dialog = adw::AlertDialog::builder()
        .heading("Rename")
        .body("Enter a new name")
        .build();

    let entry = gtk::Entry::builder()
        .text(current_name)
        .build();
    entry.select_region(0, -1);
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("rename", "Rename");
    dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("rename"));

    let on_submit = RefCell::new(Some(on_submit));
    let entry_response = entry.clone();
    dialog.connect_response(None, move |_dialog, response| {
        if response == "rename" {
            if let Some(callback) = on_submit.borrow_mut().take() {
                callback(entry_response.text().to_string());
            }
        }
    });

    if let Some(root) = anchor.as_ref().root() {
        if let Ok(window) = root.downcast::<gtk::Window>() {
            dialog.present(Some(&window));
        }
    }
}

fn rename_selected_local_entry(
    local_list: &gtk::ListBox,
    local_state: Rc<RefCell<LocalPaneState>>,
    local_pane: PaneWidgets,
    status_label: gtk::Label,
) {
    let Some(row) = local_list.selected_row() else {
        return;
    };
    let Some(old_name) = get_row_name(&row) else {
        return;
    };

    let current_path = local_state.borrow().current_path.clone();
    let local_pane_rename = local_pane.clone();
    let status_label_rename = status_label.clone();
    let old_name_prompt = old_name.clone();
    prompt_rename_dialog(local_list, &old_name_prompt, move |new_name| {
        let trimmed = new_name.trim().to_string();
        if trimmed.is_empty() || trimmed == old_name {
            return;
        }

        let from = current_path.join(&old_name);
        let to = current_path.join(&trimmed);
        match std::fs::rename(&from, &to) {
            Ok(_) => refresh_local_listing(&local_pane_rename, &current_path),
            Err(e) => status_label_rename.set_label(&format!(
                "Error renaming {} to {}: {e}",
                from.display(),
                to.display(),
            )),
        }
    });
}

fn upload_selected_local_entry(
    local_list: &gtk::ListBox,
    local_state: Rc<RefCell<LocalPaneState>>,
    remote_path: Rc<RefCell<String>>,
    cmd_tx: Rc<async_channel::Sender<SftpCommand>>,
) {
    let Some(row) = local_list.selected_row() else {
        return;
    };

    let Some(name) = get_row_name(&row) else {
        return;
    };

    let local_path = local_state.borrow().current_path.join(&name);
    if !local_path.exists() {
        return;
    }

    let rpath = remote_path.borrow().clone();
    let remote = join_remote_path(&rpath, &name);
    let tx = (*cmd_tx).clone();
    let tx_refresh = (*cmd_tx).clone();
    glib::spawn_future_local(async move {
        let _ = tx.send(SftpCommand::Upload {
            local: local_path,
            remote,
        }).await;
        let _ = tx_refresh.send(SftpCommand::ListDir(rpath)).await;
    });
}

fn can_download_selected_remote_entries(remote_list: &gtk::ListBox) -> bool {
    !remote_list.selected_rows().is_empty()
}

fn can_rename_selected_remote_entry(remote_list: &gtk::ListBox) -> bool {
    remote_list.selected_rows().len() == 1
}

fn rename_selected_remote_entry(
    remote_list: &gtk::ListBox,
    remote_path: Rc<RefCell<String>>,
    cmd_tx: Rc<async_channel::Sender<SftpCommand>>,
    connected: bool,
) {
    if !connected || !can_rename_selected_remote_entry(remote_list) {
        return;
    }

    let selected_rows = remote_list.selected_rows();
    let Some(row) = selected_rows.first() else {
        return;
    };
    let Some(old_name) = get_row_name(row) else {
        return;
    };

    let current_path = remote_path.borrow().clone();
    let cmd_tx_rename = cmd_tx.clone();
    let old_name_prompt = old_name.clone();
    prompt_rename_dialog(remote_list, &old_name_prompt, move |new_name| {
        let trimmed = new_name.trim().to_string();
        if trimmed.is_empty() || trimmed == old_name {
            return;
        }

        let from = join_remote_path(&current_path, &old_name);
        let to = join_remote_path(&current_path, &trimmed);
        let refresh_path = current_path.clone();
        let tx = (*cmd_tx_rename).clone();
        glib::spawn_future_local(async move {
            let _ = tx.send(SftpCommand::Rename {
                from,
                to,
            }).await;
            let _ = tx.send(SftpCommand::ListDir(refresh_path)).await;
        });
    });
}

fn download_selected_remote_entry(
    remote_list: &gtk::ListBox,
    remote_path: Rc<RefCell<String>>,
    local_state: Rc<RefCell<LocalPaneState>>,
    local_pane: PaneWidgets,
    cmd_tx: Rc<async_channel::Sender<SftpCommand>>,
) {
    if !can_download_selected_remote_entries(remote_list) {
        return;
    }

    let selected_names = get_selected_row_names(remote_list);
    if selected_names.is_empty() {
        return;
    }
    let rpath = remote_path.borrow().clone();
    let local = local_state.borrow().current_path.clone();
    let tx = (*cmd_tx).clone();
    let ls = local_state.clone();
    let lp = local_pane.clone();
    glib::spawn_future_local(async move {
        for name in selected_names {
            let remote = join_remote_path(&rpath, &name);
            let _ = tx.send(SftpCommand::Download {
                remote,
                local: local.clone(),
            }).await;
        }
        glib::timeout_future(std::time::Duration::from_millis(500)).await;
        let path = ls.borrow().current_path.clone();
        refresh_local_listing(&lp, &path);
    });
}

fn delete_selected_remote_entries(
    remote_list: &gtk::ListBox,
    remote_path: Rc<RefCell<String>>,
    cmd_tx: Rc<async_channel::Sender<SftpCommand>>,
) {
    let selected_names = get_selected_row_names(remote_list);
    if selected_names.is_empty() {
        return;
    }

    let current_path = remote_path.borrow().clone();
    let tx = (*cmd_tx).clone();
    glib::spawn_future_local(async move {
        for name in selected_names {
            let target = join_remote_path(&current_path, &name);
            let _ = tx.send(SftpCommand::Remove(target)).await;
        }
        let _ = tx.send(SftpCommand::ListDir(current_path)).await;
    });
}

fn delete_selected_local_entries(
    local_list: &gtk::ListBox,
    local_state: Rc<RefCell<LocalPaneState>>,
    local_pane: PaneWidgets,
    status_label: gtk::Label,
) {
    let selected_names: Vec<String> = local_list
        .selected_rows()
        .into_iter()
        .filter_map(|row| get_row_name(&row))
        .collect();

    if selected_names.is_empty() {
        return;
    }

    let current_path = local_state.borrow().current_path.clone();
    let mut first_error: Option<String> = None;
    for name in selected_names {
        let target = current_path.join(&name);
        let result = if target.is_dir() {
            std::fs::remove_dir_all(&target)
        } else {
            std::fs::remove_file(&target)
        };
        if let Err(e) = result {
            if first_error.is_none() {
                first_error = Some(format!("Error deleting {}: {e}", target.display()));
            }
        }
    }

    refresh_local_listing(&local_pane, &current_path);
    if let Some(msg) = first_error {
        status_label.set_label(&msg);
    }
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
                let new_path = join_remote_path(&current, &name);
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
