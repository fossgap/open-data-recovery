use crate::i18n::tr;
use gtk::{gdk, gio, glib, prelude::*};
use std::path::{Path, PathBuf};

pub mod smoke;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Theme {
    #[default]
    System,
    Custom,
}

impl Theme {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Custom => "custom",
        }
    }

    fn read(path: Option<&Path>) -> Self {
        path.and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|value| Self::parse(value.trim()))
            .unwrap_or_default()
    }

    fn apply(self, display: &gdk::Display, custom: &gtk::CssProvider) {
        match self {
            Self::Custom => gtk::style_context_add_provider_for_display(
                display,
                custom,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
            ),
            Self::System => gtk::style_context_remove_provider_for_display(display, custom),
        }
    }
}

/// One application-wide action keeps existing windows, dialogs and future windows in sync.
/// The system option leaves GTK settings alone and installs only our layout rules.
pub fn init(app: &gtk::Application) {
    install(app, crate::preferences::path("theme"));
}

fn install(app: &gtk::Application, path: Option<PathBuf>) {
    let display = gdk::Display::default().expect("GTK display");
    let layout = gtk::CssProvider::new();
    layout.load_from_data(include_str!("../ui/layout.css"));
    gtk::style_context_add_provider_for_display(
        &display,
        &layout,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let custom = gtk::CssProvider::new();
    custom.load_from_data(include_str!("../ui/style.css"));
    let theme = Theme::read(path.as_deref());
    theme.apply(&display, &custom);
    let action = gio::SimpleAction::new_stateful(
        "theme",
        Some(glib::VariantTy::STRING),
        &theme.name().to_variant(),
    );
    let weak_app = app.downgrade();
    action.connect_change_state(move |action, value| {
        let Some(value) = value else { return };
        let Some(theme) = value.str().and_then(Theme::parse) else {
            return;
        };
        if action.state().as_ref() == Some(value) {
            return;
        }
        let result = path
            .as_deref()
            .ok_or_else(|| std::io::Error::other("No configuration directory"))
            .and_then(|path| crate::preferences::write(path, theme.name()));
        if let Err(error) = result {
            if let Some(app) = weak_app.upgrade() {
                let dialog = gtk::MessageDialog::builder()
                    .message_type(gtk::MessageType::Error)
                    .buttons(gtk::ButtonsType::Close)
                    .modal(true)
                    .text(tr("Could not save theme preference"))
                    .secondary_text(crate::trf!(
                        "The theme was not changed. Check that your configuration folder is writable and try again.\n\n{error}",
                        error = error
                    ))
                    .build();
                dialog.set_transient_for(app.active_window().as_ref());
                dialog.connect_response(|dialog, _| dialog.close());
                dialog.present();
            }
            return;
        }
        theme.apply(&display, &custom);
        action.set_state(value);
        if let Some(app) = weak_app.upgrade() {
            for window in app.windows() {
                window.queue_draw();
            }
        }
    });
    // Parameterized activation is used by the radio items in the View menu.
    action.connect_activate(|action, value| {
        if let Some(value) = value {
            action.change_state(value);
        }
    });
    app.add_action(&action);
}

/// Use the same artwork in source builds and installed packages.
pub fn set_brand_icon(builder: &gtk::Builder) {
    let bytes =
        gtk::glib::Bytes::from_static(include_bytes!("../packaging/org.opendatarecovery.App.png"));
    let icon = gtk::gio::BytesIcon::new(&bytes);
    builder
        .object::<gtk::Image>("brand_icon")
        .expect("Window brand icon")
        .set_from_gicon(&icon);
}

/// Custom disk maps use the active GTK palette, including dark and high-contrast themes.
pub fn map_colors(widget: &impl IsA<gtk::Widget>) -> (gdk::RGBA, gdk::RGBA) {
    let context = widget.style_context();
    let foreground = context.color();
    let accent = context
        .lookup_color("odr_map_accent")
        .or_else(|| context.lookup_color("accent_bg_color"))
        .or_else(|| context.lookup_color("theme_selected_bg_color"))
        .unwrap_or(foreground);
    (foreground, accent)
}

pub fn filesystem_class(filesystem: &str, unallocated: bool) -> &'static str {
    if unallocated {
        return "fs-unallocated";
    }
    match filesystem.to_ascii_lowercase().as_str() {
        "ext2" | "ext3" | "ext4" | "btrfs" | "xfs" => "fs-linux",
        "fat" | "fat12" | "fat16" | "fat32" | "vfat" | "exfat" => "fs-fat",
        _ => "fs-ntfs",
    }
}

/// Filesystem colors exist only in the original custom theme. System maps keep GTK's palette.
pub fn filesystem_color(widget: &impl IsA<gtk::Widget>, class: &str) -> Option<gdk::RGBA> {
    widget
        .style_context()
        .lookup_color(&format!("odr_{}", class.replace('-', "_")))
}

pub fn source(context: &gtk::cairo::Context, color: &gdk::RGBA, opacity: f64) {
    context.set_source_rgba(
        color.red().into(),
        color.green().into(),
        color.blue().into(),
        f64::from(color.alpha()) * opacity,
    );
}
