//! Isolated GTK localization checks; never opens a recovery source or starts a job.
use super::*;
use crate::ui::engine_picker;

pub fn run(app: &gtk::Application) {
    let window_builder = builder(include_str!("../../ui/window.ui"));
    let window: gtk::ApplicationWindow = window_builder.object("window").unwrap();
    window.set_application(Some(app));
    let expected = if language() == "ar" {
        gtk::TextDirection::Rtl
    } else {
        gtk::TextDirection::Ltr
    };
    assert_eq!(window.direction(), expected);
    assert_eq!(
        window_builder
            .object::<gtk::Button>("start")
            .unwrap()
            .label()
            .as_deref(),
        Some(tr("Scan again"))
    );
    let menu: gtk::gio::MenuModel = window_builder.object("menubar").unwrap();
    assert_eq!(
        menu.item_attribute_value(0, "label", None)
            .and_then(|v| v.get::<String>())
            .as_deref(),
        Some(tr("_File"))
    );
    let scan = builder(include_str!("../../ui/scan-options.ui"));
    let dialog: gtk::Window = scan.object("dialog").unwrap();
    dialog.set_transient_for(Some(&window));
    assert_eq!(dialog.title().as_deref(), Some(tr("Custom scan")));
    assert_eq!(
        scan.object::<gtk::Button>("confirm")
            .unwrap()
            .label()
            .as_deref(),
        Some(tr("Start scan"))
    );
    let extensions: gtk::Entry = scan.object("extensions").unwrap();
    assert_eq!(extensions.direction(), gtk::TextDirection::Ltr);
    assert_eq!(
        extensions.tooltip_text().as_deref(),
        Some(tr(
            "Extension filters constrain extraction where supported; other engines filter results. File families can include several extensions."
        ))
    );
    extensions.set_text("jpg,pdf");
    assert_eq!(extensions.text(), "jpg,pdf");
    let kind: gtk::DropDown = scan.object("ext_kind").unwrap();
    let model = kind.model().unwrap().downcast::<gtk::StringList>().unwrap();
    assert_eq!(model.string(0).as_deref(), Some("all"));
    assert_eq!(model.string(1).as_deref(), Some("file"));
    kind.set_selected(1);
    assert_eq!(
        kind.selected_item()
            .and_downcast::<gtk::StringObject>()
            .unwrap()
            .string(),
        "file"
    );
    engine_picker::setup_scan(&scan);
    for name in ["auto", "tsk", "extundelete"] {
        engine_picker::availability(&scan, "metadata", name, None);
    }
    engine_picker::check(&scan, "metadata", "tsk").set_active(true);
    engine_picker::check(&scan, "metadata", "extundelete").set_active(true);
    assert_eq!(
        engine_picker::selected(&scan, "metadata"),
        vec!["tsk", "extundelete"]
    );
    engine_picker::check(&scan, "metadata", "auto").set_active(true);
    assert_eq!(engine_picker::selected(&scan, "metadata"), vec!["auto"]);
    assert_eq!(
        scan.object::<gtk::Label>("metadata_value").unwrap().text(),
        tr("Auto")
    );
    for xml in [
        include_str!("../../ui/recovery.ui"),
        include_str!("../../ui/saved-scans.ui"),
        include_str!("../../ui/delete-scan.ui"),
        include_str!("../../ui/source.ui"),
        include_str!("../../ui/partitions.ui"),
        include_str!("../../ui/resume-imaging.ui"),
        include_str!("../../ui/restore-image.ui"),
        include_str!("../../ui/restore-confirm.ui"),
        include_str!("../../ui/findings.ui"),
    ] {
        let b = builder(xml);
        let w: gtk::Window = b.object("dialog").unwrap();
        assert_eq!(w.direction(), expected);
        w.destroy();
    }
    // Map real widgets so GTK also lays out translated labels and the dropdown factory.
    window.present();
    dialog.present();
    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
        let notebook: gtk::Notebook = scan.object("options_tabs").unwrap();
        let tab = notebook
            .tab_label(&notebook.nth_page(Some(2)).unwrap())
            .unwrap()
            .downcast::<gtk::Label>()
            .unwrap();
        assert_eq!(tab.text(), tr("Engine options"));
        assert!(!tab.wraps());
        assert!(
            dialog.width() < 1280,
            "Translated dialog exceeds a 1280px screen"
        );
        assert!(
            tab.width() >= tab.measure(gtk::Orientation::Horizontal, -1).0,
            "Tab clipped"
        );
        dialog.destroy();
        window.destroy();
        println!("Localization GTK check passed: {}", language());
    });
}
