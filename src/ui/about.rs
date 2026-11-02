//! Shared native About dialog for the production and demo workspaces.
use crate::i18n::tr;
use gtk::{gio, glib, prelude::*};

fn create(parent: &gtk::ApplicationWindow, demo: bool) -> gtk::AboutDialog {
    let builder = crate::i18n::builder(include_str!("../../ui/about.ui"));
    let dialog: gtk::AboutDialog = builder.object("about").expect("About dialog");
    dialog.set_transient_for(Some(parent));
    dialog.set_version(Some(env!("CARGO_PKG_VERSION")));
    dialog.set_website(Some(env!("CARGO_PKG_REPOSITORY")));
    dialog.set_authors(&[&format!(
        "{} {}/graphs/contributors",
        tr("Open Data Recovery contributors"),
        env!("CARGO_PKG_REPOSITORY")
    )]);
    if demo {
        dialog.set_comments(Some(&format!(
            "{}\n\n{}",
            dialog.comments().unwrap_or_default(),
            tr("Demo mode uses sample data. No devices are accessed or recovery engines run.")
        )));
    }

    // Embed the packaged artwork so source builds have the same logo without installation.
    let bytes = glib::Bytes::from_static(include_bytes!(
        "../../packaging/org.opendatarecovery.App.png"
    ));
    let stream = gio::MemoryInputStream::from_bytes(&bytes);
    let logo = gtk::gdk_pixbuf::Pixbuf::from_stream_at_scale(
        &stream,
        128,
        128,
        true,
        gio::Cancellable::NONE,
    )
    .expect("Bundled application logo");
    dialog.set_logo(Some(&gtk::gdk::Texture::for_pixbuf(&logo)));
    // Setting the name/version resets GTK's title in the desktop locale.
    // Apply ODR's selected language after populating the dialog.
    dialog.set_title(Some(tr("About")));
    dialog
}

pub fn install(window: &gtk::ApplicationWindow, demo: bool) {
    let action = gio::SimpleAction::new("about", None);
    let parent = window.downgrade();
    let cached = glib::WeakRef::<gtk::AboutDialog>::new();
    action.connect_activate(move |_, _| {
        let Some(parent) = parent.upgrade() else {
            return;
        };
        let dialog = cached.upgrade().unwrap_or_else(|| {
            let dialog = create(&parent, demo);
            cached.set(Some(&dialog));
            dialog
        });
        dialog.present();
    });
    window.add_action(&action);
}

/// Exercise the real menu action in the existing GTK smoke checks.
pub fn smoke(window: &gtk::ApplicationWindow, demo: bool) {
    let action = window.lookup_action("about").expect("About menu action");
    let dialogs = || {
        gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::AboutDialog>().ok())
            .filter(|d| d.transient_for().as_ref() == Some(window.upcast_ref()))
            .collect::<Vec<_>>()
    };
    action.activate(None);
    let dialog = dialogs().pop().expect("Native About dialog opened");
    assert!(dialog.is_visible() && dialog.is_modal());
    assert_eq!(
        dialog.title().as_deref(),
        Some(tr("About"))
    );
    assert_eq!(dialog.version().as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert_eq!(
        dialog.website().as_deref(),
        Some(env!("CARGO_PKG_REPOSITORY"))
    );
    assert_eq!(dialog.license_type(), gtk::License::Gpl30Only);
    assert!(dialog.logo().is_some());
    assert!(!dialog.authors().is_empty());
    assert_eq!(dialog.direction(), window.direction());
    assert_eq!(
        dialog.comments().unwrap().contains(tr(
            "Demo mode uses sample data. No devices are accessed or recovery engines run."
        )),
        demo
    );
    action.activate(None);
    assert_eq!(dialogs(), vec![dialog.clone()], "Reuse the open dialog");
    dialog.close();
    assert!(!dialog.is_visible());
    action.activate(None);
    assert_eq!(dialogs(), vec![dialog.clone()], "Reuse the closed dialog");
    assert!(dialog.is_visible());
    dialog.close();
}
