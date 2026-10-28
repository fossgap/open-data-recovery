//! Native popover selectors: multiple metadata engines, one carver or imager.
use crate::i18n::tr;
use gtk::{StringList, prelude::*};
use std::{cell::Cell, rc::Rc};

fn object<T: gtk::glib::object::IsA<gtk::glib::Object> + Clone + 'static>(
    b: &gtk::Builder,
    id: &str,
) -> T {
    b.object(id)
        .unwrap_or_else(|| panic!("Missing engine picker object: {id}"))
}
pub fn model(b: &gtk::Builder, id: &str) -> StringList {
    object(b, &format!("{id}_selection"))
}
pub fn selected(b: &gtk::Builder, id: &str) -> Vec<String> {
    let model = model(b, id);
    (0..model.n_items())
        .filter_map(|i| model.string(i).map(|s| s.to_string()))
        .collect()
}
pub fn single(b: &gtk::Builder, id: &str) -> String {
    selected(b, id).into_iter().next().unwrap_or_default()
}
pub fn set_selected(b: &gtk::Builder, id: &str, values: &[String]) {
    let model = model(b, id);
    let names: Vec<_> = values.iter().map(String::as_str).collect();
    model.splice(0, model.n_items(), &names);
}
fn checks(b: &gtk::Builder, id: &str) -> Vec<gtk::CheckButton> {
    let mut result = vec![];
    let mut child = object::<gtk::Box>(b, &format!("{id}_choices")).first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Ok(check) = widget.downcast::<gtk::CheckButton>() {
            result.push(check);
        }
    }
    result
}
pub fn check(b: &gtk::Builder, id: &str, name: &str) -> gtk::CheckButton {
    checks(b, id)
        .into_iter()
        .find(|c| c.widget_name() == name)
        .expect("Known engine")
}
pub fn availability(b: &gtk::Builder, id: &str, name: &str, reason: Option<&str>) {
    let check = check(b, id, name);
    check.set_sensitive(reason.is_none());
    check.set_tooltip_text(reason);
}
pub fn unavailable(b: &gtk::Builder, id: &str) -> Option<String> {
    let checks = checks(b, id);
    selected(b, id).iter().find_map(|name| {
        match checks
            .iter()
            .find(|check| check.widget_name() == name.as_str())
        {
            Some(check) => check
                .tooltip_text()
                .map(|reason| format!("{name}: {reason}")),
            None => Some(crate::trf!("Unknown engine: {name}", name = name)),
        }
    })
}
fn sync(b: &gtk::Builder, id: &str, changing: &Cell<bool>) {
    changing.set(true);
    let selected = selected(b, id);
    for check in checks(b, id) {
        check.set_active(
            selected
                .iter()
                .any(|name| name == check.widget_name().as_str()),
        );
    }
    let label = if selected == ["auto"] {
        tr("Auto").into()
    } else {
        selected.join(", ")
    };
    object::<gtk::Label>(b, &format!("{id}_value")).set_text(&label);
    changing.set(false);
}
pub fn setup(b: &gtk::Builder, id: &'static str, names: &[&str], multiple: bool) {
    let button: gtk::MenuButton = object(b, id);
    let anchor = button.downgrade();
    button.popover().unwrap().connect_show(move |popup| {
        if let Some(button) = anchor.upgrade() {
            popup.set_width_request(button.width());
        }
    });
    let selection = StringList::new(&[names[0]]);
    b.expose_object(&format!("{id}_selection"), &selection);
    let list: gtk::Box = object(b, &format!("{id}_choices"));
    let changing = Rc::new(Cell::new(false));
    let mut group: Option<gtk::CheckButton> = None;
    for &name in names {
        let check = gtk::CheckButton::with_label(if name == "auto" { tr("Auto") } else { name });
        check.set_widget_name(name);
        if !multiple {
            if let Some(group) = &group {
                check.set_group(Some(group));
            } else {
                group = Some(check.clone());
            }
        }
        check.set_sensitive(false);
        check.set_tooltip_text(Some(tr("Checking installed engines…")));
        list.append(&check);
        if name == "auto" {
            list.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        }
        let dialog = b.downgrade();
        let changing = changing.clone();
        let name = name.to_string();
        check.connect_toggled(move |check| {
            if changing.get() || !multiple && !check.is_active() {
                return;
            }
            let Some(b) = dialog.upgrade() else {
                return;
            };
            if !check.is_sensitive() {
                sync(&b, id, &changing);
                return;
            }
            let mut values = selected(&b, id);
            if !multiple || name == "auto" {
                values = vec![name.clone()];
            } else {
                values.retain(|value| value != "auto" && value != &name);
                if check.is_active() {
                    values.push(name.clone());
                }
                if values.is_empty() {
                    values.push("auto".into());
                }
            }
            set_selected(&b, id, &values);
            if !multiple {
                object::<gtk::MenuButton>(&b, id).popdown();
            }
        });
    }
    let dialog = b.downgrade();
    let updating = changing.clone();
    selection.connect_items_changed(move |_, _, _, _| {
        if let Some(b) = dialog.upgrade() {
            sync(&b, id, &updating);
        }
    });
    sync(b, id, &changing);
}
pub fn setup_scan(b: &gtk::Builder) {
    setup(b, "metadata", crate::engine::adapters::METADATA, true);
    setup(b, "carver", crate::engine::adapters::CARVERS, false);
    setup(b, "clone_engine", &["ddrescue", "safecopy"], false);
}
