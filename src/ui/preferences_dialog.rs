use gtk4 as gtk;
use gtk::prelude::*;
use libadwaita as adw;
use adw::prelude::*;

use crate::app::SharedState;
use crate::config::Settings;

pub fn show_preferences_dialog(parent: &adw::ApplicationWindow, state: &SharedState) {
    let dialog = adw::Dialog::builder()
        .title("Preferences")
        .content_width(450)
        .content_height(400)
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
        .title("Terminal Settings")
        .margin_start(16)
        .margin_end(16)
        .margin_top(8)
        .build();

    let current_settings = state.settings.lock().unwrap().clone();

    let font_family_row = adw::EntryRow::builder()
        .title("Font Family")
        .build();
    font_family_row.set_text(&current_settings.font_family);

    let font_size_adj = gtk::Adjustment::new(
        current_settings.font_size as f64,
        6.0,
        72.0,
        1.0,
        2.0,
        0.0,
    );
    let font_size_row = adw::SpinRow::builder()
        .title("Font Size")
        .adjustment(&font_size_adj)
        .build();

    let scrollback_adj = gtk::Adjustment::new(
        current_settings.scrollback_lines as f64,
        100.0,
        1000000.0,
        100.0,
        1000.0,
        0.0,
    );
    let scrollback_row = adw::SpinRow::builder()
        .title("Scrollback Lines")
        .adjustment(&scrollback_adj)
        .build();

    let term_type_row = adw::EntryRow::builder()
        .title("Terminal Type")
        .build();
    term_type_row.set_text(&current_settings.default_terminal_type);

    group.add(&font_family_row);
    group.add(&font_size_row);
    group.add(&scrollback_row);
    group.add(&term_type_row);

    toolbar_view.set_content(Some(&group));
    dialog.set_child(Some(&toolbar_view));

    let state_clone = state.clone();
    let dialog_clone = dialog.clone();
    save_btn.connect_clicked(move |_| {
        let new_settings = Settings {
            font_family: font_family_row.text().to_string(),
            font_size: font_size_row.value() as u32,
            scrollback_lines: scrollback_row.value() as i64,
            default_terminal_type: term_type_row.text().to_string(),
        };

        if let Err(e) = new_settings.save() {
            log::error!("Failed to save settings: {e}");
        }

        let mut settings = state_clone.settings.lock().unwrap();
        *settings = new_settings;
        dialog_clone.close();
    });

    dialog.present(Some(parent));
}
