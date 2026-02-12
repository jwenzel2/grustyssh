use gtk4 as gtk;
use gtk::prelude::*;
use libadwaita as adw;
use adw::prelude::*;

use std::cell::RefCell;
use std::rc::Rc;

use crate::app::SharedState;
use crate::keys::generate::generate_keypair;
use crate::keys::storage::KeyStore;
use crate::models::connection::KeyAlgorithm;

pub fn show_key_manager_dialog(parent: &adw::ApplicationWindow, state: &SharedState) {
    let dialog = adw::Dialog::builder()
        .title("SSH Key Manager")
        .content_width(600)
        .content_height(500)
        .build();

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let content_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content_box.set_margin_start(16);
    content_box.set_margin_end(16);
    content_box.set_margin_top(8);
    content_box.set_margin_bottom(16);

    // Generate new key section
    let gen_group = adw::PreferencesGroup::builder()
        .title("Generate New Key")
        .build();

    let name_row = adw::EntryRow::builder()
        .title("Key Name")
        .build();
    gen_group.add(&name_row);

    let passphrase_row = adw::PasswordEntryRow::builder()
        .title("Passphrase (optional)")
        .build();
    gen_group.add(&passphrase_row);

    let algo_row = adw::ComboRow::builder()
        .title("Algorithm")
        .build();
    let algo_list = gtk::StringList::new(&[
        "Ed25519",
        "ECDSA NIST P-256",
        "RSA SHA2-512",
    ]);
    algo_row.set_model(Some(&algo_list));
    gen_group.add(&algo_row);

    let generate_btn = gtk::Button::builder()
        .label("Generate Key")
        .css_classes(["suggested-action"])
        .halign(gtk::Align::End)
        .margin_top(8)
        .build();

    content_box.append(&gen_group);
    content_box.append(&generate_btn);

    // Existing keys section
    let keys_group = adw::PreferencesGroup::builder()
        .title("Stored Keys")
        .build();

    let keys_listbox = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();

    let state_clone = state.clone();
    let keys_listbox_rc = Rc::new(RefCell::new(keys_listbox.clone()));

    let rebuild_key_list = {
        let state = state_clone.clone();
        let keys_listbox_rc = keys_listbox_rc.clone();
        let _keys_group_ref = keys_group.clone();
        move || {
            let listbox = keys_listbox_rc.borrow();
            // Remove all rows
            while let Some(child) = listbox.first_child() {
                listbox.remove(&child);
            }

            let store = state.key_store.lock().unwrap();
            if store.keys.is_empty() {
                let label = gtk::Label::builder()
                    .label("No keys stored yet")
                    .css_classes(["dim-label"])
                    .margin_top(12)
                    .margin_bottom(12)
                    .build();
                listbox.append(&label);
            } else {
                for key_meta in &store.keys {
                    let row = adw::ActionRow::builder()
                        .title(&key_meta.name)
                        .subtitle(&format!(
                            "{} — {}",
                            key_meta.algorithm, key_meta.public_key_fingerprint
                        ))
                        .build();

                    let export_btn = gtk::Button::builder()
                        .icon_name("edit-copy-symbolic")
                        .tooltip_text("Copy public key")
                        .valign(gtk::Align::Center)
                        .css_classes(["flat"])
                        .build();

                    let key_id = key_meta.id;
                    export_btn.connect_clicked(move |_btn| {
                        if let Ok(pub_key) = KeyStore::read_public_key(&key_id) {
                            if let Some(display) = gtk::gdk::Display::default() {
                                let clipboard = display.clipboard();
                                clipboard.set_text(&pub_key);
                            }
                        }
                    });

                    let delete_btn = gtk::Button::builder()
                        .icon_name("user-trash-symbolic")
                        .tooltip_text("Delete key")
                        .valign(gtk::Align::Center)
                        .css_classes(["flat", "destructive-action"])
                        .build();

                    row.add_suffix(&export_btn);
                    row.add_suffix(&delete_btn);
                    listbox.append(&row);
                }
            }
        }
    };

    rebuild_key_list();

    content_box.append(&keys_group);
    content_box.append(&keys_listbox);

    let scrolled = gtk::ScrolledWindow::builder()
        .child(&content_box)
        .vexpand(true)
        .build();
    toolbar_view.set_content(Some(&scrolled));
    dialog.set_child(Some(&toolbar_view));

    // Generate button handler
    let state_for_gen = state.clone();
    let name_row_clone = name_row.clone();
    let passphrase_row_clone = passphrase_row.clone();
    let algo_row_clone = algo_row.clone();
    let rebuild = rebuild_key_list.clone();
    generate_btn.connect_clicked(move |_btn| {
        let name = name_row_clone.text().to_string();
        if name.is_empty() {
            return;
        }
        let algo_idx = algo_row_clone.selected();
        let algorithm = match algo_idx {
            0 => KeyAlgorithm::Ed25519,
            1 => KeyAlgorithm::EcdsaNistP256,
            2 => KeyAlgorithm::RsaSha2_512,
            _ => KeyAlgorithm::Ed25519,
        };

        let passphrase_text = passphrase_row_clone.text().to_string();
        let passphrase = if passphrase_text.is_empty() {
            None
        } else {
            Some(passphrase_text.as_str())
        };

        match generate_keypair(&name, algorithm, passphrase) {
            Ok(meta) => {
                let mut store = state_for_gen.key_store.lock().unwrap();
                if let Err(e) = store.add(meta) {
                    log::error!("Failed to save key: {e}");
                }
                drop(store);
                name_row_clone.set_text("");
                passphrase_row_clone.set_text("");
                rebuild();
            }
            Err(e) => {
                log::error!("Key generation failed: {e}");
            }
        }
    });

    dialog.present(Some(parent));
}
