use gtk4 as gtk;
use gtk::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use zeroize::Zeroizing;

use std::cell::RefCell;
use std::rc::Rc;

use crate::app::SharedState;
use crate::models::connection::{AuthMethod, ConnectionProfile};
use crate::ui::connection_dialog;
use crate::ui::sftp_tab;
use crate::ui::terminal_tab;

/// Build the sidebar connection list widget.
/// Returns the sidebar box and a closure to refresh the list.
pub fn build_connection_list(
    window: &adw::ApplicationWindow,
    tab_view: &adw::TabView,
    state: &SharedState,
) -> (gtk::Box, Rc<dyn Fn()>) {
    let sidebar_box = gtk::Box::new(gtk::Orientation::Vertical, 0);

    let list_header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    list_header.set_margin_start(8);
    list_header.set_margin_end(8);
    list_header.set_margin_top(8);
    list_header.set_margin_bottom(4);

    let title_label = gtk::Label::builder()
        .label("Connections")
        .css_classes(["title-3"])
        .hexpand(true)
        .halign(gtk::Align::Start)
        .build();

    let add_btn = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("New Connection")
        .css_classes(["flat"])
        .build();

    let backup_btn = gtk::Button::builder()
        .icon_name("document-save-symbolic")
        .tooltip_text("Backup connections")
        .css_classes(["flat"])
        .build();

    let restore_btn = gtk::Button::builder()
        .icon_name("document-open-symbolic")
        .tooltip_text("Restore connections")
        .css_classes(["flat"])
        .build();

    list_header.append(&title_label);
    list_header.append(&add_btn);
    list_header.append(&backup_btn);
    list_header.append(&restore_btn);
    sidebar_box.append(&list_header);

    let listbox = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["navigation-sidebar"])
        .vexpand(true)
        .build();

    let scrolled = gtk::ScrolledWindow::builder()
        .child(&listbox)
        .vexpand(true)
        .build();
    sidebar_box.append(&scrolled);

    let state_rc = Rc::new(state.clone());
    let listbox_rc = Rc::new(listbox.clone());
    let window_rc = Rc::new(window.clone());
    let tab_view_rc = Rc::new(tab_view.clone());

    // Self-referencing rebuild closure so button handlers inside can trigger a list refresh.
    let rebuild_holder: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    let rebuild: Rc<dyn Fn()> = Rc::new({
        let state = state_rc.clone();
        let listbox_rc = listbox_rc.clone();
        let window_rc = window_rc.clone();
        let tab_view_rc = tab_view_rc.clone();
        let rebuild_holder = rebuild_holder.clone();
        move || {
            let listbox = &*listbox_rc;
            while let Some(child) = listbox.first_child() {
                listbox.remove(&child);
            }

            let profiles: Vec<ConnectionProfile> = {
                let store = state.profile_store.lock().unwrap();
                store.profiles.clone()
            };

            if profiles.is_empty() {
                let label = gtk::Label::builder()
                    .label("No saved connections")
                    .css_classes(["dim-label"])
                    .margin_top(24)
                    .margin_bottom(24)
                    .build();
                listbox.append(&label);
                return;
            }

            for profile in profiles {
                let row = adw::ActionRow::builder()
                    .title(&profile.name)
                    .subtitle(&format!(
                        "{}@{}:{}",
                        profile.username, profile.hostname, profile.port
                    ))
                    .activatable(true)
                    .build();

                let sftp_btn = gtk::Button::builder()
                    .icon_name("folder-symbolic")
                    .tooltip_text("SFTP File Transfer")
                    .valign(gtk::Align::Center)
                    .css_classes(["flat"])
                    .build();

                let connect_btn = gtk::Button::builder()
                    .icon_name("media-playback-start-symbolic")
                    .tooltip_text("Connect")
                    .valign(gtk::Align::Center)
                    .css_classes(["flat"])
                    .build();

                let edit_btn = gtk::Button::builder()
                    .icon_name("document-edit-symbolic")
                    .tooltip_text("Edit")
                    .valign(gtk::Align::Center)
                    .css_classes(["flat"])
                    .build();

                let delete_btn = gtk::Button::builder()
                    .icon_name("user-trash-symbolic")
                    .tooltip_text("Delete")
                    .valign(gtk::Align::Center)
                    .css_classes(["flat"])
                    .build();

                row.add_suffix(&sftp_btn);
                row.add_suffix(&connect_btn);
                row.add_suffix(&edit_btn);
                row.add_suffix(&delete_btn);

                // SFTP button
                let profile_for_sftp = profile.clone();
                let tab_view_sftp = tab_view_rc.clone();
                let state_sftp = state.clone();
                let window_sftp = window_rc.clone();
                sftp_btn.connect_clicked(move |_| {
                    let needs_password = matches!(
                        profile_for_sftp.auth_method,
                        AuthMethod::Password | AuthMethod::Both
                    );

                    let key_has_passphrase = if let Some(key_id) = profile_for_sftp.key_pair_id {
                        let store = state_sftp.key_store.lock().unwrap();
                        store.get(&key_id).map(|k| k.has_passphrase).unwrap_or(false)
                    } else {
                        false
                    };

                    let profile_c = profile_for_sftp.clone();
                    let tab_view_cc = tab_view_sftp.clone();

                    if key_has_passphrase && needs_password {
                        let window_c2 = window_sftp.clone();
                        let profile_c2 = profile_c.clone();
                        prompt_secret(
                            &window_sftp,
                            &format!("Key passphrase for {}", profile_c.name),
                            "Enter the passphrase for your SSH key:",
                            move |key_pass| {
                                let key_passphrase = Some(Zeroizing::new(key_pass));
                                let profile_c3 = profile_c2.clone();
                                let tab_view_cc2 = tab_view_cc.clone();
                                prompt_secret(
                                    &window_c2,
                                    &format!("Password for {}", profile_c2.name),
                                    "Enter your SSH password:",
                                    move |password| {
                                        sftp_tab::create_sftp_tab(
                                            &tab_view_cc2,
                                            &profile_c3,
                                            Some(Zeroizing::new(password)),
                                            key_passphrase,
                                        );
                                    },
                                );
                            },
                        );
                    } else if key_has_passphrase {
                        prompt_secret(
                            &window_sftp,
                            &format!("Key passphrase for {}", profile_c.name),
                            "Enter the passphrase for your SSH key:",
                            move |key_pass| {
                                sftp_tab::create_sftp_tab(
                                    &tab_view_cc,
                                    &profile_c,
                                    None,
                                    Some(Zeroizing::new(key_pass)),
                                );
                            },
                        );
                    } else if needs_password {
                        prompt_secret(
                            &window_sftp,
                            &format!("Password for {}", profile_c.name),
                            "Enter your SSH password:",
                            move |password| {
                                sftp_tab::create_sftp_tab(
                                    &tab_view_cc,
                                    &profile_c,
                                    Some(Zeroizing::new(password)),
                                    None,
                                );
                            },
                        );
                    } else {
                        sftp_tab::create_sftp_tab(
                            &tab_view_cc,
                            &profile_c,
                            None,
                            None,
                        );
                    }
                });

                // Connect button
                let profile_for_connect = profile.clone();
                let tab_view_c = tab_view_rc.clone();
                let state_c = state.clone();
                let window_c = window_rc.clone();
                connect_btn.connect_clicked(move |_| {
                    let needs_password = matches!(
                        profile_for_connect.auth_method,
                        AuthMethod::Password | AuthMethod::Both
                    );

                    // Check if the selected key has a passphrase
                    let key_has_passphrase = if let Some(key_id) = profile_for_connect.key_pair_id {
                        let store = state_c.key_store.lock().unwrap();
                        store.get(&key_id).map(|k| k.has_passphrase).unwrap_or(false)
                    } else {
                        false
                    };

                    let profile_c = profile_for_connect.clone();
                    let tab_view_cc = tab_view_c.clone();
                    let state_cc = state_c.clone();

                    if key_has_passphrase && needs_password {
                        // Need both key passphrase and SSH password
                        let window_c2 = window_c.clone();
                        let profile_c2 = profile_c.clone();
                        prompt_secret(
                            &window_c,
                            &format!("Key passphrase for {}", profile_c.name),
                            "Enter the passphrase for your SSH key:",
                            move |key_pass| {
                                let key_passphrase = Some(Zeroizing::new(key_pass));
                                let profile_c3 = profile_c2.clone();
                                let tab_view_cc2 = tab_view_cc.clone();
                                let state_cc2 = state_cc.clone();
                                prompt_secret(
                                    &window_c2,
                                    &format!("Password for {}", profile_c2.name),
                                    "Enter your SSH password:",
                                    move |password| {
                                        terminal_tab::create_terminal_tab(
                                            &tab_view_cc2,
                                            &profile_c3,
                                            Some(Zeroizing::new(password)),
                                            key_passphrase,
                                            &state_cc2,
                                        );
                                    },
                                );
                            },
                        );
                    } else if key_has_passphrase {
                        // Only key passphrase needed
                        prompt_secret(
                            &window_c,
                            &format!("Key passphrase for {}", profile_c.name),
                            "Enter the passphrase for your SSH key:",
                            move |key_pass| {
                                terminal_tab::create_terminal_tab(
                                    &tab_view_cc,
                                    &profile_c,
                                    None,
                                    Some(Zeroizing::new(key_pass)),
                                    &state_cc,
                                );
                            },
                        );
                    } else if needs_password {
                        // Only SSH password needed
                        prompt_secret(
                            &window_c,
                            &format!("Password for {}", profile_c.name),
                            "Enter your SSH password:",
                            move |password| {
                                terminal_tab::create_terminal_tab(
                                    &tab_view_cc,
                                    &profile_c,
                                    Some(Zeroizing::new(password)),
                                    None,
                                    &state_cc,
                                );
                            },
                        );
                    } else {
                        terminal_tab::create_terminal_tab(
                            &tab_view_cc,
                            &profile_c,
                            None,
                            None,
                            &state_cc,
                        );
                    }
                });

                // Edit button
                let profile_for_edit = profile.clone();
                let window_edit = window_rc.clone();
                let state_edit = state.clone();
                let rebuild_edit = rebuild_holder.clone();
                edit_btn.connect_clicked(move |_| {
                    let profile_clone = profile_for_edit.clone();
                    let state_save = state_edit.clone();
                    let rebuild_ref = rebuild_edit.clone();
                    connection_dialog::show_connection_dialog(
                        &window_edit,
                        &state_edit,
                        Some(profile_clone),
                        move |profile| {
                            let mut store = state_save.profile_store.lock().unwrap();
                            let _ = store.update(profile);
                            drop(store);
                            if let Some(ref rebuild_fn) = *rebuild_ref.borrow() {
                                rebuild_fn();
                            }
                        },
                    );
                });

                // Delete button
                let profile_id = profile.id;
                let state_del = state.clone();
                let rebuild_del = rebuild_holder.clone();
                delete_btn.connect_clicked(move |_| {
                    let mut store = state_del.profile_store.lock().unwrap();
                    let _ = store.remove(&profile_id);
                    drop(store);
                    if let Some(ref rebuild_fn) = *rebuild_del.borrow() {
                        rebuild_fn();
                    }
                });

                listbox.append(&row);
            }
        }
    });

    // Store the rebuild closure so button handlers can access it
    *rebuild_holder.borrow_mut() = Some(rebuild.clone());

    rebuild();

    // Add button opens new connection dialog
    let window_for_add = window.clone();
    let state_for_add = state.clone();
    let rebuild_for_add = rebuild.clone();
    add_btn.connect_clicked(move |_| {
        let state_save = state_for_add.clone();
        let rebuild_c = rebuild_for_add.clone();
        connection_dialog::show_connection_dialog(
            &window_for_add,
            &state_for_add,
            None,
            move |profile| {
                let mut store = state_save.profile_store.lock().unwrap();
                let _ = store.add(profile);
                drop(store);
                rebuild_c();
            },
        );
    });

    // Backup button
    let state_for_backup = state.clone();
    let window_for_backup = window.clone();
    backup_btn.connect_clicked(move |_| {
        let backup_json = {
            let store = state_for_backup.profile_store.lock().unwrap();
            store.export_backup()
        };
        match backup_json {
            Ok(json) => {
                let file_dialog = gtk::FileDialog::builder()
                    .title("Save Connections Backup")
                    .initial_name("grustyssh-connections-backup.json")
                    .build();
                let parent_clone = window_for_backup.clone();
                file_dialog.save(
                    Some(&window_for_backup),
                    gtk::gio::Cancellable::NONE,
                    move |result| {
                        if let Ok(file) = result {
                            if let Some(path) = file.path() {
                                if let Err(e) = std::fs::write(&path, &json) {
                                    log::error!("Failed to write backup: {e}");
                                } else {
                                    let alert = adw::AlertDialog::builder()
                                        .heading("Backup Saved")
                                        .body(format!("Connections backed up to {}", path.display()))
                                        .build();
                                    alert.add_response("ok", "OK");
                                    alert.present(Some(&parent_clone));
                                }
                            }
                        }
                    },
                );
            }
            Err(e) => log::error!("Failed to export connections: {e}"),
        }
    });

    // Restore button
    let state_for_restore = state.clone();
    let window_for_restore = window.clone();
    let rebuild_for_restore = rebuild.clone();
    restore_btn.connect_clicked(move |_| {
        let filter = gtk::FileFilter::new();
        filter.add_pattern("*.json");
        filter.set_name(Some("JSON Backup Files"));
        let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);

        let file_dialog = gtk::FileDialog::builder()
            .title("Restore Connections from Backup")
            .filters(&filters)
            .build();

        let state_clone = state_for_restore.clone();
        let parent_clone = window_for_restore.clone();
        let rebuild = rebuild_for_restore.clone();
        file_dialog.open(
            Some(&window_for_restore),
            gtk::gio::Cancellable::NONE,
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        match std::fs::read_to_string(&path) {
                            Ok(json) => {
                                let import_result = {
                                    let mut store = state_clone.profile_store.lock().unwrap();
                                    store.import_backup(&json)
                                };
                                match import_result {
                                    Ok(count) => {
                                        rebuild();
                                        let alert = adw::AlertDialog::builder()
                                            .heading("Restore Complete")
                                            .body(format!("Imported {count} connection(s)."))
                                            .build();
                                        alert.add_response("ok", "OK");
                                        alert.present(Some(&parent_clone));
                                    }
                                    Err(e) => {
                                        log::error!("Failed to import backup: {e}");
                                        let alert = adw::AlertDialog::builder()
                                            .heading("Restore Failed")
                                            .body(format!("{e}"))
                                            .build();
                                        alert.add_response("ok", "OK");
                                        alert.present(Some(&parent_clone));
                                    }
                                }
                            }
                            Err(e) => log::error!("Failed to read backup file: {e}"),
                        }
                    }
                }
            },
        );
    });

    (sidebar_box, rebuild)
}

/// Show a prompt dialog for a secret value (password or passphrase).
fn prompt_secret(
    parent: &adw::ApplicationWindow,
    heading: &str,
    body: &str,
    on_submit: impl FnOnce(String) + 'static,
) {
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(body)
        .build();

    let entry = gtk::PasswordEntry::builder()
        .show_peek_icon(true)
        .build();
    dialog.set_extra_child(Some(&entry));

    dialog.add_response("cancel", "Cancel");
    dialog.add_response("connect", "Connect");
    dialog.set_response_appearance("connect", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("connect"));

    // Wrap FnOnce in Rc<RefCell<Option<>>> so it can be shared between closures
    let on_submit: Rc<RefCell<Option<Box<dyn FnOnce(String) + 'static>>>> =
        Rc::new(RefCell::new(Some(Box::new(on_submit))));

    // Enter key in the password field triggers connect
    let dialog_for_entry = dialog.clone();
    let on_submit_for_entry = on_submit.clone();
    let entry_for_activate = entry.clone();
    entry.connect_activate(move |_| {
        if let Some(callback) = on_submit_for_entry.borrow_mut().take() {
            callback(entry_for_activate.text().to_string());
        }
        dialog_for_entry.close();
    });

    let entry_clone = entry.clone();
    dialog.connect_response(None, move |_dialog, response| {
        if response == "connect" {
            if let Some(callback) = on_submit.borrow_mut().take() {
                let value = entry_clone.text().to_string();
                callback(value);
            }
        }
    });

    dialog.present(Some(parent));
}
