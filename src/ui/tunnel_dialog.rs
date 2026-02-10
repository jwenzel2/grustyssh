use gtk4 as gtk;
use gtk::prelude::*;
use libadwaita as adw;
use adw::prelude::*;

use crate::models::tunnel::TunnelConfig;

/// Show a dialog to add/edit a tunnel configuration.
pub fn show_tunnel_dialog(
    parent: &adw::ApplicationWindow,
    existing: Option<TunnelConfig>,
    on_save: impl Fn(TunnelConfig) + 'static,
) {
    let is_edit = existing.is_some();
    let dialog = adw::Dialog::builder()
        .title(if is_edit {
            "Edit Tunnel"
        } else {
            "Add Tunnel"
        })
        .content_width(400)
        .content_height(350)
        .build();

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();

    let save_btn = gtk::Button::builder()
        .label("Save")
        .css_classes(["suggested-action"])
        .build();
    header.pack_end(&save_btn);
    toolbar_view.add_top_bar(&header);

    let group = adw::PreferencesGroup::builder()
        .title("Local Port Forward")
        .margin_start(16)
        .margin_end(16)
        .margin_top(8)
        .build();

    let name_row = adw::EntryRow::builder().title("Tunnel Name").build();
    let local_host_row = adw::EntryRow::builder().title("Local Host").build();
    local_host_row.set_text("127.0.0.1");

    let local_port_adj = gtk::Adjustment::new(8080.0, 1.0, 65535.0, 1.0, 10.0, 0.0);
    let local_port_row = adw::SpinRow::builder()
        .title("Local Port")
        .adjustment(&local_port_adj)
        .build();

    let remote_host_row = adw::EntryRow::builder().title("Remote Host").build();
    remote_host_row.set_text("127.0.0.1");

    let remote_port_adj = gtk::Adjustment::new(80.0, 1.0, 65535.0, 1.0, 10.0, 0.0);
    let remote_port_row = adw::SpinRow::builder()
        .title("Remote Port")
        .adjustment(&remote_port_adj)
        .build();

    let enabled_row = adw::SwitchRow::builder()
        .title("Enabled")
        .active(true)
        .build();

    group.add(&name_row);
    group.add(&local_host_row);
    group.add(&local_port_row);
    group.add(&remote_host_row);
    group.add(&remote_port_row);
    group.add(&enabled_row);

    // Populate existing
    let tunnel_id = if let Some(ref tc) = existing {
        name_row.set_text(&tc.name);
        local_host_row.set_text(&tc.local_host);
        local_port_row.set_value(tc.local_port as f64);
        remote_host_row.set_text(&tc.remote_host);
        remote_port_row.set_value(tc.remote_port as f64);
        enabled_row.set_active(tc.enabled);
        tc.id
    } else {
        uuid::Uuid::new_v4()
    };

    toolbar_view.set_content(Some(&group));
    dialog.set_child(Some(&toolbar_view));

    let dialog_clone = dialog.clone();
    save_btn.connect_clicked(move |_| {
        let name = name_row.text().to_string();
        if name.is_empty() {
            return;
        }

        let tc = TunnelConfig {
            id: tunnel_id,
            name,
            tunnel_type: crate::models::tunnel::TunnelType::LocalForward,
            local_host: local_host_row.text().to_string(),
            local_port: local_port_row.value() as u16,
            remote_host: remote_host_row.text().to_string(),
            remote_port: remote_port_row.value() as u16,
            enabled: enabled_row.is_active(),
        };

        on_save(tc);
        dialog_clone.close();
    });

    dialog.present(Some(parent));
}
