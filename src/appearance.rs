use gtk::{gdk, prelude::*};

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
        .lookup_color("accent_bg_color")
        .or_else(|| context.lookup_color("theme_selected_bg_color"))
        .unwrap_or(foreground);
    (foreground, accent)
}

pub fn source(context: &gtk::cairo::Context, color: &gdk::RGBA, opacity: f64) {
    context.set_source_rgba(
        color.red().into(),
        color.green().into(),
        color.blue().into(),
        f64::from(color.alpha()) * opacity,
    );
}
