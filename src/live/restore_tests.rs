use super::*;

#[test]
#[ignore = "Requires an isolated GTK display; uses synthetic disks and never opens a device"]
fn gtk_restore_wizard_confirmation_navigation_and_cancellation() {
    gtk::init().unwrap();
    crate::i18n::init();
    crate::i18n::direction();
    let wizard = Wizard::new(Weak::new());
    wizard.window().present();
    let b = &wizard.builder;
    let (mut image, mut disk) = engine_restore::tests::pair();
    image.path = "/media/backup/workstation.img".into();
    image.size = 64 * 1024 * 1024 * 1024;
    disk.path = "/dev/fixture".into();
    disk.model = "Disposable test disk".into();
    disk.size = 256 * 1024 * 1024 * 1024;
    obj::<gtk::Entry>(b, "path").set_text(&image.path.to_string_lossy());
    let mut unavailable = disk.clone();
    unavailable.path = "/dev/busy-fixture".into();
    unavailable.unavailable = Some("Mounted fixture".into());
    wizard.image.replace(Some(image.clone()));
    wizard.show_page("disk");
    wizard.show_disks(vec![unavailable, disk.clone()]);
    let list: gtk::ListBox = obj(b, "disks");
    assert!(list.selected_row().is_none());
    assert!(!obj::<gtk::Button>(b, "next").is_sensitive());
    assert!(!list.row_at_index(0).unwrap().is_selectable());
    list.select_row(list.row_at_index(1).as_ref());
    assert!(obj::<gtk::Button>(b, "next").is_sensitive());
    let plan = engine_restore::Plan { image, disk };
    wizard.review(plan.clone());
    let confirm: gtk::Button = obj(b, "restore");
    assert!(b.object::<gtk::Widget>("confirmation").is_none());
    assert!(b.object::<gtk::Widget>("acknowledge").is_none());
    assert!(confirm.is_sensitive());
    confirm.emit_clicked();
    assert!(
        !wizard.running(),
        "Opening confirmation must not start writes"
    );
    let (generation, dialog) = {
        let c = wizard.confirmation.borrow();
        let c = c.as_ref().unwrap();
        (c.generation, c.builder.clone())
    };
    let window: gtk::Window = obj(&dialog, "dialog");
    assert!(window.is_modal());
    assert_eq!(window.transient_for(), Some(wizard.window()));
    assert_eq!(
        window.default_widget(),
        Some(obj::<gtk::Button>(&dialog, "close").upcast())
    );
    assert_eq!(
        gtk::prelude::GtkWindowExt::focus(&window),
        Some(obj::<gtk::Button>(&dialog, "close").upcast())
    );
    assert!(
        obj::<gtk::Label>(&dialog, "details")
            .text()
            .contains("/dev/fixture")
    );
    assert!(
        obj::<gtk::Label>(&dialog, "details")
            .text()
            .contains("workstation.img")
    );
    assert!(!confirm.is_sensitive());
    confirm.emit_clicked();
    assert_eq!(
        wizard.confirmation.borrow().as_ref().unwrap().generation,
        generation
    );
    obj::<gtk::Button>(&dialog, "close").emit_clicked();
    assert!(wizard.confirmation.borrow().is_none());
    assert!(!window.is_visible());
    assert_eq!(wizard.page(), "review");
    assert!(!wizard.running());
    assert!(confirm.is_sensitive());
    confirm.emit_clicked();
    let c = wizard
        .confirmation
        .borrow()
        .as_ref()
        .unwrap()
        .builder
        .clone();
    obj::<gtk::Window>(&c, "dialog").close();
    assert!(wizard.confirmation.borrow().is_none());
    assert!(!wizard.running());
    confirm.emit_clicked();
    obj::<gtk::Button>(b, "back").emit_clicked();
    assert_eq!(wizard.page(), "disk");
    assert!(wizard.plan.borrow().is_none());
    assert!(wizard.confirmation.borrow().is_none());
    // An old dialog cannot authorize a later review, even with the same target.
    wizard.review(plan.clone());
    confirm.emit_clicked();
    let current = wizard.confirmation.borrow().as_ref().unwrap().generation;
    obj::<gtk::Button>(&dialog, "confirm").emit_clicked();
    assert_eq!(
        wizard.confirmation.borrow().as_ref().unwrap().generation,
        current
    );
    assert!(!wizard.running());
    let accepted = wizard
        .resolve_confirmation(current, true)
        .expect("Explicit confirmation");
    assert_eq!(accepted.disk, plan.disk);
    assert_eq!(accepted.image.path, plan.image.path);
    assert!(
        wizard.resolve_confirmation(current, true).is_none(),
        "Confirmation is consumed once"
    );
    confirm.emit_clicked();
    let c = wizard
        .confirmation
        .borrow()
        .as_ref()
        .unwrap()
        .builder
        .clone();
    obj::<gtk::Button>(&c, "confirm").emit_clicked();
    assert!(wizard.confirmation.borrow().is_none());
    assert!(
        !wizard.running(),
        "A fixture wizard has no live controller or write target"
    );
    assert!(wizard.window().default_widget().is_none());
    // Optional captures are taken from this synthetic, non-writing test window.
    if let Some(directory) = std::env::var_os("ODR_RESTORE_SCREENSHOTS") {
        use std::process::Command;
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let context = glib::MainContext::default();
        for (page, title) in [
            ("image", "image"),
            ("disk", "disk"),
            ("review", "review"),
            ("confirmation", "confirmation"),
        ] {
            if page == "review" || page == "confirmation" {
                wizard.review(plan.clone());
                obj::<gtk::Label>(b, "review_details").select_region(0, 0);
            } else {
                wizard.show_page(page);
            }
            let capture = if page == "confirmation" {
                confirm.emit_clicked();
                obj::<gtk::Window>(
                    &wizard.confirmation.borrow().as_ref().unwrap().builder,
                    "dialog",
                )
            } else {
                wizard.window()
            };
            if page == "image" {
                obj::<gtk::Button>(b, "browse").grab_focus();
            }
            if page == "disk" {
                text(
                    b,
                    "image_details",
                    &crate::trf!(
                        "{path}\nImage size: {size} ({bytes} bytes)",
                        path = path_text(&plan.image.path),
                        size = model::size(plan.image.size),
                        bytes = plan.image.size
                    ),
                );
                obj::<gtk::Label>(b, "image_details").select_region(0, 0);
                obj::<gtk::Button>(b, "next").grab_focus();
            }
            for _ in 0..20 {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let ids = Command::new("xdotool")
                .args([
                    "search",
                    "--onlyvisible",
                    "--all",
                    "--pid",
                    &std::process::id().to_string(),
                    "--name",
                    if page == "confirmation" {
                        tr("Confirm disk restoration")
                    } else {
                        tr("Restore image to disk")
                    },
                ])
                .output()
                .unwrap();
            assert!(ids.status.success());
            let id = String::from_utf8(ids.stdout)
                .unwrap()
                .lines()
                .last()
                .unwrap()
                .to_string();
            assert!(capture.width() >= 520, "Wizard collapsed horizontally");
            // A bare Xvfb server has no window manager to place RTL windows.
            assert!(
                Command::new("xdotool")
                    .args(["windowmove", &id, "0", "0"])
                    .status()
                    .unwrap()
                    .success()
            );
            for _ in 0..5 {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(
                Command::new("import")
                    .args(["-window", &id])
                    .arg(directory.join(format!("restore-{title}.png")))
                    .status()
                    .unwrap()
                    .success()
            );
        }
    }
    wizard.show_page("progress");
    let (sender, events) = mpsc::channel();
    let control = engine::process::Control::default();
    wizard.job.replace(Some(engine_restore::Job {
        control: control.clone(),
        events,
    }));
    wizard.controls();
    wizard.window().close();
    assert!(control.cancelled.load(std::sync::atomic::Ordering::SeqCst));
    assert!(wizard.window().is_visible());
    assert!(!obj::<gtk::Button>(b, "close").is_visible());
    sender
        .send(engine_restore::Event::Progress {
            phase: engine_restore::Phase::Writing,
            bytes: 1,
            total: 2,
        })
        .unwrap();
    wizard.tick();
    assert_eq!(
        obj::<gtk::Label>(b, "result").text(),
        tr("Stopping restoration and flushing written data…")
    );
    sender
        .send(engine_restore::Event::Finished(Err(
            engine::Error::Cancelled,
        )))
        .unwrap();
    wizard.tick();
    assert!(!wizard.running());
    assert!(obj::<gtk::Button>(b, "close").is_visible());
    assert_eq!(
        obj::<gtk::Label>(b, "heading").text(),
        tr("Restoration stopped")
    );
    assert!(obj::<gtk::ProgressBar>(b, "progress").fraction() < 1.0);
    let (sender, events) = mpsc::channel();
    wizard.job.replace(Some(engine_restore::Job {
        control: Default::default(),
        events,
    }));
    sender
        .send(engine_restore::Event::Progress {
            phase: engine_restore::Phase::Verifying,
            bytes: 4096,
            total: 4096,
        })
        .unwrap();
    sender
        .send(engine_restore::Event::Finished(engine::fail(
            "Fixture checksum mismatch",
        )))
        .unwrap();
    wizard.tick();
    assert_eq!(
        obj::<gtk::Label>(b, "heading").text(),
        tr("Restoration failed")
    );
    assert!(obj::<gtk::ProgressBar>(b, "progress").fraction() < 1.0);
    let (sender, events) = mpsc::channel();
    wizard.job.replace(Some(engine_restore::Job {
        control: Default::default(),
        events,
    }));
    sender
        .send(engine_restore::Event::Finished(Ok(
            engine_restore::Completed {
                bytes: 4096,
                sha256: "fixture hash".into(),
            },
        )))
        .unwrap();
    wizard.tick();
    assert_eq!(
        obj::<gtk::Label>(b, "heading").text(),
        tr("Restoration complete")
    );
    assert_eq!(obj::<gtk::ProgressBar>(b, "progress").fraction(), 1.0);
    wizard.window().close();
}
