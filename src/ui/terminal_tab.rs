use gtk4 as gtk;
use gtk::prelude::*;
use gtk::glib;
use libadwaita as adw;
use vte4::prelude::*;
use zeroize::Zeroizing;

use std::cell::Cell;
use std::rc::Rc;

use crate::app::{SharedState, SshCommand, SshEvent};
use crate::config::Settings;
use crate::models::connection::ConnectionProfile;
use crate::ssh::session;

/// Create a new terminal tab connected to the given profile.
/// Returns the tab page widget.
pub fn create_terminal_tab(
    tab_view: &adw::TabView,
    profile: &ConnectionProfile,
    password: Option<Zeroizing<String>>,
    key_passphrase: Option<Zeroizing<String>>,
    state: &SharedState,
) -> adw::TabPage {
    let terminal = vte4::Terminal::new();

    // Explicitly set erase bindings so VTE doesn't try to read from a
    // real TTY (there is none – we feed data directly over SSH).
    terminal.set_backspace_binding(vte4::EraseBinding::AsciiBackspace);
    terminal.set_delete_binding(vte4::EraseBinding::DeleteSequence);

    // Apply settings
    {
        let settings = state.settings.lock().unwrap();
        apply_terminal_settings(&terminal, &settings);
    }

    // Right-click = copy if selection exists, paste if nothing selected (PuTTY style)
    let gesture_click = gtk::GestureClick::new();
    gesture_click.set_button(gtk::gdk::BUTTON_SECONDARY);
    let term_for_rclick = terminal.clone();
    gesture_click.connect_pressed(move |gesture, _n_press, _x, _y| {
        if term_for_rclick.has_selection() {
            term_for_rclick.copy_clipboard_format(vte4::Format::Text);
        } else {
            term_for_rclick.paste_clipboard();
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    terminal.add_controller(gesture_click);

    // Ctrl+Shift+C = copy, Ctrl+Shift+V = paste
    let key_ctrl = gtk::EventControllerKey::new();
    let term_for_keys = terminal.clone();
    key_ctrl.connect_key_pressed(move |_, keyval, _keycode, modifiers| {
        let ctrl_shift = gtk::gdk::ModifierType::CONTROL_MASK
            | gtk::gdk::ModifierType::SHIFT_MASK;
        if modifiers.contains(ctrl_shift) {
            match keyval {
                gtk::gdk::Key::C => {
                    term_for_keys.copy_clipboard_format(vte4::Format::Text);
                    return glib::Propagation::Stop;
                }
                gtk::gdk::Key::V => {
                    term_for_keys.paste_clipboard();
                    return glib::Propagation::Stop;
                }
                _ => {}
            }
        }
        glib::Propagation::Proceed
    });
    terminal.add_controller(key_ctrl);

    let scrolled = gtk::ScrolledWindow::builder()
        .child(&terminal)
        .vexpand(true)
        .hexpand(true)
        .build();

    let page = tab_view.append(&scrolled);
    page.set_title(&profile.name);

    // Set up async channels
    let (event_tx, event_rx) = async_channel::bounded::<SshEvent>(256);

    // Spawn the SSH session and get the command sender
    let cmd_tx = session::spawn_session(profile.clone(), password, key_passphrase, event_tx);

    // Store cmd_tx in an Rc for sharing across closures
    let cmd_tx_rc = Rc::new(cmd_tx);

    // Wire terminal input -> SSH command
    let cmd_tx_input = cmd_tx_rc.clone();
    terminal.connect_commit(move |_term, text, _size| {
        let bytes = text.as_bytes().to_vec();
        let tx = (*cmd_tx_input).clone();
        glib::spawn_future_local(async move {
            let _ = tx.send(SshCommand::SendData(bytes)).await;
        });
    });

    // Detect terminal resize via tick callback and forward to SSH session.
    // VTE4 does not emit notify signals for column_count/row_count, so we
    // poll each frame (~60fps). Only two integer comparisons per frame;
    // a resize command is sent only when values actually change.
    let last_cols = Rc::new(Cell::new(terminal.column_count()));
    let last_rows = Rc::new(Cell::new(terminal.row_count()));
    let cmd_tx_resize = cmd_tx_rc.clone();
    let term_for_resize = terminal.clone();
    terminal.add_tick_callback(move |_widget, _clock| {
        let cols = term_for_resize.column_count();
        let rows = term_for_resize.row_count();
        if cols != last_cols.get() || rows != last_rows.get() {
            last_cols.set(cols);
            last_rows.set(rows);
            let tx = (*cmd_tx_resize).clone();
            let cols = cols as u32;
            let rows = rows as u32;
            glib::spawn_future_local(async move {
                let _ = tx.send(SshCommand::Resize { cols, rows }).await;
            });
        }
        glib::ControlFlow::Continue
    });

    // Poll SSH events and feed data to terminal
    let terminal_clone = terminal.clone();
    glib::spawn_future_local(async move {
        while let Ok(event) = event_rx.recv().await {
            match event {
                SshEvent::Connected => {
                    log::info!("SSH session connected");
                    terminal_clone.grab_focus();
                }
                SshEvent::Data(data) => {
                    terminal_clone.feed(&data);
                }
                SshEvent::Disconnected(reason) => {
                    if let Some(reason) = reason {
                        let msg = format!("\r\n[Disconnected: {}]\r\n", reason);
                        terminal_clone.feed(msg.as_bytes());
                    } else {
                        terminal_clone.feed(b"\r\n[Disconnected]\r\n");
                    }
                    break;
                }
                SshEvent::Error(msg) => {
                    let err_msg = format!("\r\n[Error: {}]\r\n", msg);
                    terminal_clone.feed(err_msg.as_bytes());
                }
                SshEvent::HostKeyVerify {
                    key_type,
                    fingerprint,
                } => {
                    let msg = format!(
                        "\r\n[Host key ({key_type}): {fingerprint}]\r\n\
                         [Accepting host key automatically (TOFU)]\r\n"
                    );
                    terminal_clone.feed(msg.as_bytes());
                }
                SshEvent::TunnelEstablished(id) => {
                    let msg = format!("\r\n[Tunnel {} established]\r\n", id);
                    terminal_clone.feed(msg.as_bytes());
                }
                SshEvent::TunnelFailed(id, err) => {
                    let msg = format!("\r\n[Tunnel {} failed: {}]\r\n", id, err);
                    terminal_clone.feed(msg.as_bytes());
                }
            }
        }
    });

    // Send initial terminal size
    let cols = terminal.column_count() as u32;
    let rows = terminal.row_count() as u32;
    let cmd_tx_init = cmd_tx_rc.clone();
    glib::spawn_future_local(async move {
        let _ = cmd_tx_init.send(SshCommand::Resize { cols, rows }).await;
    });

    // Store cmd_tx Rc in the page's GObject data for later disconnect
    let cmd_tx_for_page = cmd_tx_rc.clone();
    // SAFETY: We only store and retrieve our own typed data under a known key
    unsafe {
        page.set_data::<Rc<async_channel::Sender<SshCommand>>>("cmd_tx", cmd_tx_for_page);
    }

    page
}

fn apply_terminal_settings(terminal: &vte4::Terminal, settings: &Settings) {
    let font_desc = gtk::pango::FontDescription::from_string(&format!(
        "{} {}",
        settings.font_family, settings.font_size
    ));
    terminal.set_font(Some(&font_desc));
    terminal.set_scrollback_lines(settings.scrollback_lines);
}

/// Disconnect the SSH session for a tab page.
pub fn disconnect_tab(page: &adw::TabPage) {
    // Retrieve the stored cmd_tx and send Disconnect
    unsafe {
        if let Some(cmd_tx) =
            page.data::<Rc<async_channel::Sender<SshCommand>>>("cmd_tx")
        {
            let tx = cmd_tx.as_ref().clone();
            let sender = (*tx).clone();
            glib::spawn_future_local(async move {
                let _ = sender.send(SshCommand::Disconnect).await;
            });
        }
    }
}
