use gtk::{gdk, prelude::*};

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
