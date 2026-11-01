//! Theme checks use a private temporary preference and never open a recovery source.
use super::*;

pub fn run(app: &gtk::Application) {
    let root = glib::mkdtemp(std::env::temp_dir().join("odr-theme-XXXXXX")).unwrap();
    let path = root.join("theme");
    let seed = std::env::var("ODR_SMOKE_THEME").unwrap_or_default();
    if !seed.is_empty() {
        std::fs::write(&path, &seed).unwrap();
    }
    let expected = Theme::parse(&seed).unwrap_or_default();
    let b = crate::i18n::builder(include_str!("../../ui/window.ui"));
    let window: gtk::ApplicationWindow = b.object("window").unwrap();
    window.set_application(Some(app));
    let scan = crate::i18n::builder(include_str!("../../ui/scan-options.ui"));
    let dialog: gtk::Window = scan.object("dialog").unwrap();
    dialog.set_transient_for(Some(&window));
    let notebook: gtk::Notebook = scan.object("options_tabs").unwrap();
    let tab = notebook
        .tab_label(&notebook.nth_page(Some(0)).unwrap())
        .unwrap();
    let settings = gtk::Settings::default().unwrap();
    let native_settings = (
        settings.gtk_theme_name(),
        settings.is_gtk_application_prefer_dark_theme(),
        settings.gtk_font_name(),
    );
    let native_color = window.style_context().color();
    let native_dialog_color = dialog.style_context().color();
    install(app, Some(path.clone()));
    let action = app.lookup_action("theme").unwrap();
    assert_eq!(action.state(), Some(expected.name().to_variant()));
    assert_eq!(
        filesystem_color(&window, "fs-linux").is_some(),
        expected == Theme::Custom,
        "Saved preference must apply before the window is shown"
    );
    // Verify the translated menu's actual targets, not a separate test-only control.
    let menu: gio::MenuModel = b.object("menubar").unwrap();
    let view = menu.item_link(2, "submenu").unwrap();
    let appearance = view.item_link(1, "section").unwrap();
    let themes = appearance.item_link(0, "submenu").unwrap();
    assert_eq!(themes.n_items(), 2);
    for (index, (label, theme)) in [
        (tr("System Default"), "system"),
        (tr("Evergreen"), "custom"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            themes.item_attribute_value(index as i32, "label", None),
            Some(label.to_variant())
        );
        assert_eq!(
            themes.item_attribute_value(index as i32, "action", None),
            Some("app.theme".to_variant())
        );
        assert_eq!(
            themes.item_attribute_value(index as i32, "target", None),
            Some(theme.to_variant())
        );
    }
    window.present();
    dialog.present();
    glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
        for _ in 0..3 {
            action.activate(Some(&"custom".to_variant()));
            assert_eq!(action.state(), Some("custom".to_variant()));
            assert_eq!(Theme::read(Some(&path)), Theme::Custom);
            assert!(filesystem_color(&window, "fs-linux").is_some());
            assert!(filesystem_color(&dialog, "fs-linux").is_some());
            assert_eq!(
                tab.style_context().color(),
                dialog.style_context().color(),
                "Custom notebook labels must not inherit light text from dark GTK"
            );
            let fresh = crate::i18n::builder(include_str!("../../ui/info.ui"));
            let fresh: gtk::Window = fresh.object("dialog").unwrap();
            assert!(filesystem_color(&fresh, "fs-linux").is_some());
            fresh.destroy();

            action.change_state(&"system".to_variant());
            assert_eq!(action.state(), Some("system".to_variant()));
            assert_eq!(Theme::read(Some(&path)), Theme::System);
            assert!(filesystem_color(&window, "fs-linux").is_none());
            assert!(filesystem_color(&dialog, "fs-linux").is_none());
            assert_eq!(window.style_context().color(), native_color);
            assert_eq!(dialog.style_context().color(), native_dialog_color);
        }
        action.activate(Some(&"unknown".to_variant()));
        assert_eq!(action.state(), Some("system".to_variant()));
        // A real save error must leave the applied style and the radio state unchanged.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        action.activate(Some(&"custom".to_variant()));
        assert_eq!(action.state(), Some("system".to_variant()));
        assert!(filesystem_color(&window, "fs-linux").is_none());
        let error = gtk::Window::list_toplevels()
            .into_iter()
            .find_map(|window| window.downcast::<gtk::MessageDialog>().ok())
            .expect("Visible save failure");
        assert_eq!(
            error.text().as_deref(),
            Some(tr("Could not save theme preference"))
        );
        error.close();
        std::fs::remove_dir(&path).unwrap();
        action.activate(Some(&"custom".to_variant()));
        assert_eq!(Theme::read(Some(&path)), Theme::Custom);
        assert_eq!(
            (
                settings.gtk_theme_name(),
                settings.is_gtk_application_prefer_dark_theme(),
                settings.gtk_font_name(),
            ),
            native_settings,
            "Switching must never change desktop GTK settings"
        );
        dialog.destroy();
        window.destroy();
        std::fs::remove_dir_all(root).unwrap();
        println!(
            "GTK theme check passed: startup, menu targets, live switching, dialogs, persistence, save errors and system restoration."
        );
    });
}
