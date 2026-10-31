use super::*;
use crate::engine::restore as engine_restore;
use std::rc::Weak;

#[cfg(test)]
#[path = "restore_tests.rs"]
mod tests;

fn path_text(path: &Path) -> String {
    let text = path.to_string_lossy();
    if crate::i18n::language() == "ar" {
        // Isolate paths in translated prose; never add these marks to input values.
        format!("\u{2066}{text}\u{2069}")
    } else {
        text.into_owned()
    }
}

enum Reply {
    Image(engine::Result<engine_restore::Image>),
    Disks(engine::Result<Vec<engine_restore::Disk>>),
    Plan(engine::Result<engine_restore::Plan>),
}

struct Confirmation {
    generation: u64,
    builder: gtk::Builder,
    plan: engine_restore::Plan,
}

pub(super) struct Wizard {
    pub(super) builder: gtk::Builder,
    parent: Weak<Live>,
    image: RefCell<Option<engine_restore::Image>>,
    disks: RefCell<Vec<engine_restore::Disk>>,
    plan: RefCell<Option<engine_restore::Plan>>,
    confirmation: RefCell<Option<Confirmation>>,
    job: RefCell<Option<engine_restore::Job>>,
    generation: Cell<u64>,
    pending: Cell<bool>,
    sender: Sender<(u64, Reply)>,
    replies: Receiver<(u64, Reply)>,
}

impl Wizard {
    fn window(&self) -> gtk::Window {
        obj(&self.builder, "dialog")
    }
    fn page(&self) -> String {
        obj::<gtk::Stack>(&self.builder, "pages")
            .visible_child_name()
            .unwrap_or_default()
            .into()
    }
    pub(super) fn running(&self) -> bool {
        self.job.borrow().is_some()
    }
    fn selected_disk(&self) -> Option<engine_restore::Disk> {
        let row = obj::<gtk::ListBox>(&self.builder, "disks").selected_row()?;
        self.disks.borrow().get(row.index() as usize).cloned()
    }
    fn show_page(&self, page: &str) {
        self.dismiss_confirmation();
        self.generation.set(self.generation.get() + 1);
        self.pending.set(false);
        obj::<gtk::Stack>(&self.builder, "pages").set_visible_child_name(page);
        text(&self.builder, "validation", "");
        text(
            &self.builder,
            "heading",
            match page {
                "image" => tr("Choose an image"),
                "disk" => tr("Choose a target disk"),
                "review" => tr("Review restoration"),
                _ => tr("Restoring image"),
            },
        );
        self.controls();
    }
    fn controls(&self) {
        let b = &self.builder;
        let page = self.page();
        let pending = self.pending.get();
        let running = self.running();
        obj::<gtk::Spinner>(b, "spinner").set_spinning(pending);
        obj::<gtk::Spinner>(b, "spinner").set_visible(pending);
        for id in ["path", "browse", "refresh", "disks"] {
            obj::<gtk::Widget>(b, id).set_sensitive(!pending && !running);
        }
        obj::<gtk::Button>(b, "back").set_visible(page == "disk" || page == "review");
        obj::<gtk::Button>(b, "back").set_sensitive(!pending);
        obj::<gtk::Button>(b, "next").set_visible(page == "image" || page == "disk");
        obj::<gtk::Button>(b, "next").set_sensitive(
            !pending
                && match page.as_str() {
                    "image" => !value(b, "path").is_empty(),
                    "disk" => self.selected_disk().is_some_and(|d| {
                        self.image
                            .borrow()
                            .as_ref()
                            .is_some_and(|image| engine_restore::validate_pair(image, &d).is_ok())
                    }),
                    _ => false,
                },
        );
        obj::<gtk::Button>(b, "restore").set_visible(page == "review");
        obj::<gtk::Button>(b, "restore").set_sensitive(
            page == "review"
                && !pending
                && !running
                && self.confirmation.borrow().is_none()
                && self.plan.borrow().is_some(),
        );
        obj::<gtk::Button>(b, "close").set_visible(!running);
        obj::<gtk::Button>(b, "close").set_label(if page == "progress" {
            tr("Close")
        } else {
            tr("Cancel")
        });
        obj::<gtk::Button>(b, "stop").set_visible(running);
    }
    fn request(&self, operation: impl FnOnce() -> Reply + Send + 'static) {
        self.dismiss_confirmation();
        self.generation.set(self.generation.get() + 1);
        let generation = self.generation.get();
        let sender = self.sender.clone();
        self.pending.set(true);
        text(&self.builder, "validation", tr("Checking…"));
        self.controls();
        std::thread::spawn(move || {
            let _ = sender.send((generation, operation()));
        });
    }
    fn refresh_disks(&self) {
        self.plan.replace(None);
        clear(&obj::<gtk::ListBox>(&self.builder, "disks"));
        self.disks.borrow_mut().clear();
        self.request(|| Reply::Disks(engine_restore::disks()));
    }
    fn next(&self) {
        if self.pending.get() || self.running() {
            return;
        }
        match self.page().as_str() {
            "image" => {
                let path = PathBuf::from(value(&self.builder, "path"));
                self.request(move || Reply::Image(engine_restore::inspect_image(&path)));
            }
            "disk" => {
                let (Some(image), Some(disk)) = (self.image.borrow().clone(), self.selected_disk())
                else {
                    return;
                };
                self.request(move || Reply::Plan(engine_restore::prepare(&image, &disk)));
            }
            _ => {}
        }
    }
    fn review(&self, plan: engine_restore::Plan) {
        let b = &self.builder;
        text(
            b,
            "review_details",
            &crate::trf!(
                "Image: {image}\n{image_size} ({image_bytes} bytes)\n\nTarget: {disk}\n{model} · Serial: {serial}\n{disk_size} ({disk_bytes} bytes) · Logical sector: {sector} bytes",
                image = path_text(&plan.image.path),
                image_size = model::size(plan.image.size),
                image_bytes = plan.image.size,
                disk = path_text(&plan.disk.path),
                model = if plan.disk.model.is_empty() {
                    tr("Unknown")
                } else {
                    &plan.disk.model
                },
                serial = if plan.disk.serial.is_empty() {
                    tr("Unknown")
                } else {
                    &plan.disk.serial
                },
                disk_size = model::size(plan.disk.size),
                disk_bytes = plan.disk.size,
                sector = plan.disk.sector_size
            ),
        );
        self.plan.replace(Some(plan));
        self.show_page("review");
        obj::<gtk::Button>(b, "restore").grab_focus();
    }
    fn dismiss_confirmation(&self) {
        let confirmation = self.confirmation.borrow_mut().take();
        if let Some(confirmation) = confirmation {
            obj::<gtk::Window>(&confirmation.builder, "dialog").destroy();
        }
    }
    fn confirm_restoration(self: &Rc<Self>) {
        if self.page() != "review" || self.pending.get() || self.running() {
            return;
        }
        if let Some(confirmation) = self.confirmation.borrow().as_ref() {
            obj::<gtk::Window>(&confirmation.builder, "dialog").present();
            return;
        }
        let Some(plan) = self.plan.borrow().clone() else {
            return;
        };
        self.generation.set(self.generation.get() + 1);
        let generation = self.generation.get();
        let b = crate::i18n::builder(include_str!("../../ui/restore-confirm.ui"));
        let window: gtk::Window = obj(&b, "dialog");
        window.set_transient_for(Some(&self.window()));
        window.set_destroy_with_parent(true);
        text(
            &b,
            "details",
            &obj::<gtk::Label>(&self.builder, "review_details").text(),
        );
        for (id, accepted) in [("close", false), ("confirm", true)] {
            let weak = Rc::downgrade(self);
            obj::<gtk::Button>(&b, id).connect_clicked(move |_| {
                if let Some(wizard) = weak.upgrade() {
                    if let Some(plan) = wizard.resolve_confirmation(generation, accepted) {
                        wizard.begin(plan);
                    }
                }
            });
        }
        let weak = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            if let Some(wizard) = weak.upgrade() {
                wizard.resolve_confirmation(generation, false);
            }
            glib::Propagation::Stop
        });
        let escape = gtk::EventControllerKey::new();
        let weak = window.downgrade();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if let Some(window) = weak.upgrade() {
                    window.close();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        window.add_controller(escape);
        self.confirmation.replace(Some(Confirmation {
            generation,
            builder: b.clone(),
            plan,
        }));
        self.controls();
        window.present();
        obj::<gtk::Button>(&b, "close").grab_focus();
        obj::<gtk::Label>(&b, "details").select_region(0, 0);
    }
    fn resolve_confirmation(
        &self,
        generation: u64,
        accepted: bool,
    ) -> Option<engine_restore::Plan> {
        if self
            .confirmation
            .borrow()
            .as_ref()
            .is_none_or(|c| c.generation != generation)
        {
            return None;
        }
        let confirmation = self.confirmation.borrow_mut().take()?;
        let valid = generation == self.generation.get()
            && self.page() == "review"
            && self.window().is_visible()
            && !self.pending.get()
            && !self.running()
            && self.plan.borrow().is_some();
        obj::<gtk::Window>(&confirmation.builder, "dialog").destroy();
        self.controls();
        (accepted && valid).then_some(confirmation.plan)
    }
    fn begin(&self, plan: engine_restore::Plan) {
        let b = &self.builder;
        if self.page() != "review"
            || !obj::<gtk::Button>(b, "restore").is_sensitive()
            || self.running()
        {
            return;
        }
        let Some(parent) = self.parent.upgrade() else {
            return;
        };
        if parent.busy() {
            return;
        }
        // Bind the backend's confirmation to the disk displayed in the accepted dialog.
        let confirmation = plan.disk.path.to_string_lossy().into_owned();
        let description = crate::trf!(
            "{image}\n→ {disk}",
            image = path_text(&plan.image.path),
            disk = path_text(&plan.disk.path)
        );
        text(b, "progress_details", &description);
        text(
            b,
            "result",
            tr("Validating the disk and requesting write access…"),
        );
        self.show_page("progress");
        obj::<gtk::ProgressBar>(b, "progress").set_fraction(0.0);
        self.job
            .replace(Some(engine_restore::start(plan, confirmation)));
        self.controls();
        parent.controls();
        parent.label("status_text", tr("Restoring image to disk…"));
        parent.event(&description);
    }
    pub(super) fn cancel(&self) {
        if let Some(job) = self.job.borrow().as_ref() {
            job.control.cancel();
            text(
                &self.builder,
                "result",
                tr("Stopping restoration and flushing written data…"),
            );
            obj::<gtk::Button>(&self.builder, "stop").set_sensitive(false);
        }
    }
    pub(super) fn tick(&self) {
        for (generation, reply) in self.replies.try_iter() {
            if generation != self.generation.get() || !self.window().is_visible() {
                continue;
            }
            self.pending.set(false);
            text(&self.builder, "validation", "");
            let result = match reply {
                Reply::Image(result) => result.map(|image| {
                    text(
                        &self.builder,
                        "image_details",
                        &crate::trf!(
                            "{path}\nImage size: {size} ({bytes} bytes)",
                            path = path_text(&image.path),
                            size = model::size(image.size),
                            bytes = image.size
                        ),
                    );
                    self.image.replace(Some(image));
                    self.show_page("disk");
                    self.refresh_disks();
                }),
                Reply::Disks(result) => result.map(|disks| self.show_disks(disks)),
                Reply::Plan(result) => result.map(|plan| self.review(plan)),
            };
            if let Err(error) = result {
                text(&self.builder, "validation", &error.to_string());
            }
            self.controls();
        }
        let events: Vec<_> = self
            .job
            .borrow()
            .as_ref()
            .map(|j| j.events.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            match event {
                engine_restore::Event::Progress {
                    phase,
                    bytes,
                    total,
                } => {
                    if self.job.borrow().as_ref().is_some_and(|job| {
                        job.control
                            .cancelled
                            .load(std::sync::atomic::Ordering::SeqCst)
                    }) {
                        continue;
                    }
                    let fraction = bytes as f64 / total.max(1) as f64;
                    let progress = match phase {
                        engine_restore::Phase::Writing => fraction * 0.5,
                        engine_restore::Phase::Verifying => 0.5 + fraction * 0.5,
                    };
                    // Reserve 100% for the terminal event after the hash comparison.
                    let progress = progress.clamp(0.0, 0.99);
                    let bar: gtk::ProgressBar = obj(&self.builder, "progress");
                    bar.set_fraction(progress);
                    bar.set_text(Some(&format!("{:.0}%", progress * 100.0)));
                    text(
                        &self.builder,
                        "heading",
                        match phase {
                            engine_restore::Phase::Writing => tr("Writing image"),
                            engine_restore::Phase::Verifying => tr("Verifying written data"),
                        },
                    );
                    text(
                        &self.builder,
                        "result",
                        &crate::trf!(
                            "{done} of {total}",
                            done = model::size(bytes),
                            total = model::size(total)
                        ),
                    );
                }
                engine_restore::Event::Finished(result) => {
                    self.job.replace(None);
                    let (heading, details) = match result {
                        Ok(done) => {
                            obj::<gtk::ProgressBar>(&self.builder, "progress").set_fraction(1.0);
                            obj::<gtk::ProgressBar>(&self.builder, "progress")
                                .set_text(Some("100%"));
                            (
                                tr("Restoration complete"),
                                crate::trf!(
                                    "{bytes} bytes written and verified.\nSHA-256: {hash}\n\nYou can now close this window. Reconnect the disk if its new partitions are not visible.",
                                    bytes = done.bytes,
                                    hash = done.sha256
                                ),
                            )
                        }
                        Err(error) => {
                            let cancelled = matches!(error, engine::Error::Cancelled);
                            (
                                if cancelled {
                                    tr("Restoration stopped")
                                } else {
                                    tr("Restoration failed")
                                },
                                crate::trf!(
                                    "{error}\n\nThe target may contain incomplete data. It has not been verified as a complete restoration. The source image was not modified.",
                                    error = if cancelled {
                                        tr("Restoration was cancelled.").into()
                                    } else {
                                        error.to_string()
                                    }
                                ),
                            )
                        }
                    };
                    text(&self.builder, "heading", heading);
                    text(&self.builder, "result", &details);
                    self.controls();
                    if let Some(parent) = self.parent.upgrade() {
                        parent.label("status_text", heading);
                        parent.event(&format!("{heading}\n{details}"));
                        parent.controls();
                        if parent.quitting.get() {
                            parent.window().close();
                        }
                    }
                }
            }
        }
    }
    fn show_disks(&self, disks: Vec<engine_restore::Disk>) {
        let list: gtk::ListBox = obj(&self.builder, "disks");
        clear(&list);
        if disks.is_empty() {
            text(
                &self.builder,
                "validation",
                tr(
                    "No physical disks with verifiable identities were found. Connect a disk and refresh.",
                ),
            );
        }
        for disk in &disks {
            let b = crate::i18n::builder(include_str!("../../ui/restore-disk-row.ui"));
            text(
                &b,
                "name",
                &format!(
                    "{} · {} · {}",
                    path_text(&disk.path),
                    if disk.model.is_empty() {
                        tr("Unknown")
                    } else {
                        &disk.model
                    },
                    model::size(disk.size)
                ),
            );
            text(
                &b,
                "details",
                &crate::trf!(
                    "Serial: {serial} · {bytes} bytes",
                    serial = if disk.serial.is_empty() {
                        tr("Unknown")
                    } else {
                        &disk.serial
                    },
                    bytes = disk.size
                ),
            );
            let reason = self
                .image
                .borrow()
                .as_ref()
                .and_then(|image| engine_restore::validate_pair(image, disk).err());
            text(
                &b,
                "reason",
                &reason.as_ref().map(ToString::to_string).unwrap_or_default(),
            );
            obj::<gtk::Label>(&b, "reason").set_visible(reason.is_some());
            let row: gtk::ListBoxRow = obj(&b, "row");
            row.set_selectable(reason.is_none());
            row.set_sensitive(reason.is_none());
            list.append(&row);
        }
        self.disks.replace(disks);
        list.unselect_all();
    }
}

pub(super) fn smoke(ui: &Rc<Live>) -> bool {
    let Some(source) = std::env::var_os("ODR_SMOKE_RESTORE_SOURCE") else {
        return false;
    };
    let source = PathBuf::from(source);
    ui.restore_image();
    let wizard = ui.restoration.borrow().clone().unwrap();
    obj::<gtk::Entry>(&wizard.builder, "path").set_text("/nonexistent/odr-fixture.img");
    obj::<gtk::Button>(&wizard.builder, "next").emit_clicked();
    let weak = Rc::downgrade(ui);
    let mut step = 0;
    let mut ticks = 0;
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let Some(ui) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        ticks += 1;
        assert!(ticks < 300, "Restoration wizard timed out at step {step}");
        if wizard.pending.get() {
            return glib::ControlFlow::Continue;
        }
        let b = &wizard.builder;
        match step {
            0 => {
                assert_eq!(wizard.page(), "image");
                assert!(!obj::<gtk::Label>(b, "validation").text().is_empty());
                obj::<gtk::Entry>(b, "path").set_text(&source.to_string_lossy());
                obj::<gtk::Button>(b, "next").emit_clicked();
                step = 1;
            }
            1 => {
                assert_eq!(wizard.page(), "disk");
                assert!(wizard.image.borrow().is_some());
                assert!(obj::<gtk::ListBox>(b, "disks").selected_row().is_none());
                assert!(!obj::<gtk::Button>(b, "next").is_sensitive());
                obj::<gtk::Button>(b, "back").emit_clicked();
                assert_eq!(wizard.page(), "image");
                obj::<gtk::Button>(b, "close").emit_clicked();
                assert!(!ui.busy());
                ui.window().close();
                println!("Restoration wizard smoke checks passed; no disk was opened for writing.");
                return glib::ControlFlow::Break;
            }
            _ => unreachable!(),
        }
        glib::ControlFlow::Continue
    });
    true
}

impl Wizard {
    fn new(parent: Weak<Live>) -> Rc<Self> {
        let (sender, replies) = mpsc::channel();
        let wizard = Rc::new(Wizard {
            builder: crate::i18n::builder(include_str!("../../ui/restore-image.ui")),
            parent,
            image: RefCell::new(None),
            disks: RefCell::new(vec![]),
            plan: RefCell::new(None),
            confirmation: RefCell::new(None),
            job: RefCell::new(None),
            generation: Cell::new(0),
            pending: Cell::new(false),
            sender,
            replies,
        });
        let b = &wizard.builder;
        let window = wizard.window();
        if let Some(parent) = wizard.parent.upgrade() {
            window.set_transient_for(Some(&parent.window()));
        }
        window.set_destroy_with_parent(true);
        Live::browse(b, "browse", "path", "chooser");
        for (id, handler) in [
            ("next", Wizard::next as fn(&Wizard)),
            ("refresh", Wizard::refresh_disks),
            ("stop", Wizard::cancel),
        ] {
            let weak = Rc::downgrade(&wizard);
            obj::<gtk::Button>(b, id).connect_clicked(move |_| {
                if let Some(w) = weak.upgrade() {
                    handler(&w);
                }
            });
        }
        let weak = Rc::downgrade(&wizard);
        obj::<gtk::Button>(b, "restore").connect_clicked(move |_| {
            if let Some(wizard) = weak.upgrade() {
                wizard.confirm_restoration();
            }
        });
        let weak = Rc::downgrade(&wizard);
        obj::<gtk::Button>(b, "back").connect_clicked(move |_| {
            if let Some(w) = weak.upgrade() {
                w.plan.replace(None);
                w.show_page(if w.page() == "review" {
                    "disk"
                } else {
                    "image"
                });
            }
        });
        let weak = Rc::downgrade(&wizard);
        obj::<gtk::Entry>(b, "path").connect_changed(move |_| {
            if let Some(w) = weak.upgrade() {
                w.dismiss_confirmation();
                w.image.replace(None);
                w.plan.replace(None);
                w.controls();
            }
        });
        let weak = Rc::downgrade(&wizard);
        obj::<gtk::ListBox>(b, "disks").connect_row_selected(move |_, _| {
            if let Some(w) = weak.upgrade() {
                w.dismiss_confirmation();
                w.plan.replace(None);
                w.controls();
            }
        });
        let weak = window.downgrade();
        obj::<gtk::Button>(b, "close").connect_clicked(move |_| {
            if let Some(w) = weak.upgrade() {
                w.close();
            }
        });
        let weak = Rc::downgrade(&wizard);
        window.connect_close_request(move |_| {
            if let Some(w) = weak.upgrade() {
                if w.running() {
                    w.cancel();
                    return glib::Propagation::Stop;
                }
                w.dismiss_confirmation();
                w.generation.set(w.generation.get() + 1);
            }
            glib::Propagation::Proceed
        });
        let escape = gtk::EventControllerKey::new();
        let weak = window.downgrade();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if let Some(w) = weak.upgrade() {
                    w.close();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        window.add_controller(escape);
        wizard.controls();
        wizard
    }
}

impl Live {
    pub(super) fn restore_image(self: &Rc<Self>) {
        if self.busy() {
            return;
        }
        if let Some(wizard) = self
            .restoration
            .borrow()
            .as_ref()
            .filter(|w| w.window().is_visible())
        {
            wizard.window().present();
            return;
        }
        let wizard = Wizard::new(Rc::downgrade(self));
        if let Some(source) = self
            .selected
            .get()
            .and_then(|i| self.sources.borrow().get(i).cloned())
            .filter(|s| s.is_image_file() && !s.ewf && s.partition.is_none())
        {
            obj::<gtk::Entry>(&wizard.builder, "path").set_text(&source.path.to_string_lossy());
        }
        self.restoration.replace(Some(wizard.clone()));
        wizard.window().present();
        obj::<gtk::Entry>(&wizard.builder, "path").grab_focus();
    }
}
