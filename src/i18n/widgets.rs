use super::*;

/// Only marked scalar text in our embedded GtkBuilder templates is translated.
/// Escape the result before parsing: translations cannot introduce XML objects or attributes.
fn translate_xml(xml: &str) -> String {
    let mut result = String::with_capacity(xml.len());
    let mut remaining = xml;
    while let Some(start) = remaining.find("translatable=\"yes\"") {
        let Some(end) = remaining[start..].find('>').map(|i| start + i + 1) else {
            break;
        };
        result.push_str(&remaining[..end]);
        remaining = &remaining[end..];
        let Some(end) = remaining.find('<') else {
            break;
        };
        let source = remaining[..end]
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&amp;", "&");
        result.push_str(&gtk::glib::markup_escape_text(tr(&source)));
        remaining = &remaining[end..];
    }
    result.push_str(remaining);
    result
}

fn dropdown_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        item.set_child(Some(&label));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        if let (Some(value), Some(label)) = (
            item.item().and_downcast::<gtk::StringObject>(),
            item.child().and_downcast::<gtk::Label>(),
        ) {
            label.set_text(tr(value.string().as_str()));
        }
    });
    factory
}

pub fn builder(xml: &str) -> gtk::Builder {
    let builder = gtk::Builder::from_string(&translate_xml(xml));
    for object in builder.objects() {
        if let Some(dropdown) = object.downcast_ref::<gtk::DropDown>() {
            // The model remains unchanged: option readers use stable engine/enum values.
            dropdown.set_factory(Some(&dropdown_factory()));
            dropdown.set_list_factory(Some(&dropdown_factory()));
        }
        if let Some(entry) = object.downcast_ref::<gtk::Entry>() {
            // Paths, extensions, patterns, addresses and engine numbers are not RTL prose.
            entry.set_direction(gtk::TextDirection::Ltr);
        }
        if language() == "ar" {
            if let Some(label) = object.downcast_ref::<gtk::Label>() {
                label.set_xalign(1.0 - label.xalign());
            }
        }
        if let Some(widget) = object.downcast_ref::<gtk::Widget>() {
            if matches!(
                gtk::prelude::BuildableExt::buildable_id(widget).as_deref(),
                Some("preview_path" | "preview_text" | "layout")
            ) {
                widget.set_direction(gtk::TextDirection::Ltr);
                if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                    label.set_xalign(0.0);
                }
            }
        }
    }
    builder
}

pub fn language_action(window: &gtk::ApplicationWindow) {
    let action = gtk::gio::SimpleAction::new("language", None);
    let parent = window.downgrade();
    action.connect_activate(move |_, _| {
        let Some(parent) = parent.upgrade() else { return; };
        let dialog = gtk::Window::builder().title(tr("Language")).transient_for(&parent)
            .modal(true).destroy_with_parent(true).default_width(420).resizable(false).build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_top(20);
        content.set_margin_bottom(20);
        content.set_margin_start(20);
        content.set_margin_end(20);
        let names: Vec<_> = std::iter::once(tr("System language")).chain(LANGUAGES.iter().map(|(_, name)| *name)).collect();
        let choice = gtk::DropDown::from_strings(&names);
        let preference = preference();
        choice.set_selected(LANGUAGES.iter().position(|(code, _)| *code == preference).map_or(0, |i| i as u32 + 1));
        content.append(&choice);
        let notice = gtk::Label::new(Some(tr("The new language will be used the next time you open the application. Current scans can continue.")));
        notice.set_wrap(true);
        notice.set_max_width_chars(48);
        notice.set_xalign(0.0);
        content.append(&notice);
        let status = gtk::Label::new(None);
        status.set_wrap(true);
        status.set_max_width_chars(48);
        content.append(&status);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_halign(gtk::Align::End);
        let cancel = gtk::Button::with_label(tr("Cancel"));
        let save = gtk::Button::with_label(tr("Apply"));
        save.add_css_class("suggested-action");
        buttons.append(&cancel);
        buttons.append(&save);
        content.append(&buttons);
        dialog.set_child(Some(&content));
        let weak = dialog.downgrade();
        cancel.connect_clicked(move |_| { if let Some(dialog) = weak.upgrade() { dialog.close(); } });
        let weak = dialog.downgrade();
        save.connect_clicked(move |_| {
            let code = choice.selected().checked_sub(1).and_then(|i| LANGUAGES.get(i as usize)).map_or("system", |(code, _)| *code);
            match save_preference(code) {
                Ok(()) => { if let Some(dialog) = weak.upgrade() { dialog.close(); } }
                Err(error) => status.set_text(&crate::trf!("Could not save language preference: {error}", error = error)),
            }
        });
        dialog.present();
    });
    window.add_action(&action);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builder_text_round_trips_xml_escaping() {
        let xml = "<property name=\"label\" translatable=\"yes\">A &amp; B &lt; C</property>";
        assert_eq!(translate_xml(xml), xml);
    }
}
