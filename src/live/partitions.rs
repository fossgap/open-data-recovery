use super::*;

impl Live {
    pub(super) fn find_partitions(self: &Rc<Self>) {
        if self.busy() {
            return;
        }
        let selected = self
            .selected
            .get()
            .and_then(|i| self.sources.borrow().get(i).cloned());
        self.partition_source
            .replace(selected.filter(discovery::eligible));
        self.partition_report.replace(None);
        self.close_partition_search.set(false);
        let b = self.show(include_str!("../../ui/partitions.ui"));
        let generation = self.dialog_generation.get();
        self.partition_generation.set(generation);
        obj::<gtk::Entry>(&b, "workspace")
            .set_text(&self.options.borrow().workspace.to_string_lossy());
        Self::browse(&b, "browse", "workspace", "chooser");
        self.partition_controls(false);
        self.partition_source_label();

        let weak = Rc::downgrade(self);
        let dialog = b.downgrade();
        obj::<gtk::Button>(&b, "search").connect_clicked(move |_| {
            let (Some(ui), Some(b)) = (weak.upgrade(), dialog.upgrade()) else {
                return;
            };
            if ui.busy() {
                return;
            }
            let Some(source) = ui.partition_source.borrow().clone() else {
                return;
            };
            let options = discovery::Options {
                workspace: PathBuf::from(value(&b, "workspace")),
                table: match obj::<gtk::DropDown>(&b, "table").selected() {
                    1 => discovery::Table::Intel,
                    2 => discovery::Table::Gpt,
                    _ => discovery::Table::Auto,
                },
                deeper: obj::<gtk::DropDown>(&b, "depth").selected() == 1,
                timeout_seconds: 0,
            };
            ui.partition_report.replace(None);
            clear(&obj::<gtk::ListBox>(&b, "partitions"));
            text(&b, "details", "Searching for partition candidates…");
            text(&b, "report", "");
            text(
                &b,
                "validation",
                if options.deeper {
                    "TestDisk · Deeper Search in progress. Large disks can take a long time."
                } else {
                    "TestDisk · Quick Search in progress…"
                },
            );
            ui.close_partition_search.set(false);
            ui.partition_search
                .replace(Some(discovery::start(source, options)));
            ui.partition_controls(true);
            ui.controls();
        });

        let weak = Rc::downgrade(self);
        let dialog = b.downgrade();
        obj::<gtk::ListBox>(&b, "partitions").connect_row_selected(move |_, row| {
            let (Some(ui), Some(b)) = (weak.upgrade(), dialog.upgrade()) else { return; };
            let report = ui.partition_report.borrow();
            let selected = row.and_then(|r| report.as_ref()?.partitions.get(r.index() as usize));
            obj::<gtk::Button>(&b, "confirm").set_sensitive(!ui.busy() && selected.is_some());
            if let Some(p) = selected {
                text(&b, "details", &format!(
                    "Start byte: {} · Length: {} bytes · Sector: {} bytes\n{}\n{}\nUse Custom Scan → Include existing files to list files lost with the partition.",
                    p.offset_bytes, p.length_bytes, p.sector_size, p.description,
                    if p.verified { "Filesystem header verified. File recoverability must still be checked by a scan." }
                    else { "Unverified filesystem header. This may be a damaged or stale candidate; a scan can fail or find no files." }
                ));
            }
        });

        let weak = Rc::downgrade(self);
        let dialog = b.downgrade();
        obj::<gtk::Button>(&b, "confirm").connect_clicked(move |_| {
            let (Some(ui), Some(b)) = (weak.upgrade(), dialog.upgrade()) else {
                return;
            };
            if ui.busy() {
                return;
            }
            let Some(row) = obj::<gtk::ListBox>(&b, "partitions").selected_row() else {
                return;
            };
            let Some(report) = ui.partition_report.borrow().clone() else {
                return;
            };
            match report.target(row.index() as usize) {
                Ok(target) => {
                    let mut sources = ui.sources.borrow_mut();
                    if let Some(original) =
                        sources.iter_mut().find(|s| s.same_target(&report.source))
                    {
                        *original = report.source.clone();
                    } else {
                        sources.push(report.source.clone());
                    }
                    let index =
                        if let Some(index) = sources.iter().position(|s| s.same_target(&target)) {
                            // A reopened/newer report may have a different content
                            // sample or filesystem probe for an already listed range.
                            sources[index] = target;
                            index
                        } else {
                            let index = sources.len();
                            sources.push(target);
                            index
                        };
                    drop(sources);
                    ui.collapsed_disks.borrow_mut().remove(&report.source.path);
                    ui.selected.set(Some(index));
                    ui.options.borrow_mut().workspace = report.options.workspace.clone();
                    obj::<gtk::Window>(&b, "dialog").close();
                    ui.render_sources();
                    ui.controls();
                    ui.label(
                        "status_text",
                        "Discovered partition selected · choose a scan mode to search its files",
                    );
                    ui.event(&format!(
                        "Selected partition {} from {}",
                        row.index() + 1,
                        report.path().display()
                    ));
                }
                Err(error) => text(&b, "validation", &error.to_string()),
            }
        });

        let chooser: gtk::FileChooserNative = obj(&b, "report_chooser");
        chooser.set_transient_for(Some(&obj::<gtk::Window>(&b, "dialog")));
        let weak_chooser = chooser.downgrade();
        obj::<gtk::Button>(&b, "open_report").connect_clicked(move |_| {
            if let Some(chooser) = weak_chooser.upgrade() {
                chooser.show();
            }
        });
        let weak = Rc::downgrade(self);
        let dialog = b.downgrade();
        chooser.connect_response(move |chooser, response| {
            let selected_path = chooser.file().and_then(|f| f.path());
            chooser.hide();
            if response != gtk::ResponseType::Accept {
                return;
            }
            let (Some(ui), Some(_b)) = (weak.upgrade(), dialog.upgrade()) else {
                return;
            };
            if ui.busy() || ui.dialog_generation.get() != generation {
                return;
            }
            if let Some(path) = selected_path {
                ui.open_partition_report(path);
            }
        });

        let weak = Rc::downgrade(self);
        obj::<gtk::Window>(&b, "dialog").connect_close_request(move |_| {
            if let Some(ui) = weak.upgrade() {
                if let Some(search) = ui.partition_search.borrow().as_ref() {
                    search.control.cancel();
                    ui.close_partition_search.set(true);
                    if let Some(b) = ui.dialog.borrow().as_ref() {
                        text(
                            b,
                            "validation",
                            "Cancelling partition search and retaining its logs…",
                        );
                    }
                    return glib::Propagation::Stop;
                }
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        obj::<gtk::Window>(&b, "dialog").connect_destroy(move |_| {
            if let Some(ui) = weak.upgrade() {
                if ui.partition_generation.get() == generation {
                    if let Some(search) = ui.partition_search.borrow().as_ref() {
                        search.control.cancel();
                    }
                }
            }
        });
    }

    pub(super) fn open_partition_report(self: &Rc<Self>, path: PathBuf) {
        if let Some(b) = self.dialog.borrow().as_ref() {
            text(b, "validation", "Reading partition search report…");
        }
        let generation = self.dialog_generation.get();
        self.submit(move || Message::PartitionReport(generation, discovery::load(&path)));
    }

    fn partition_source_label(&self) {
        if let Some(b) = self.dialog.borrow().as_ref() {
            let caption = self
                .partition_source
                .borrow()
                .as_ref()
                .map(|s| {
                    format!(
                        "{} · {} · {}",
                        s.name,
                        s.path.display(),
                        model::size(s.size)
                    )
                })
                .unwrap_or_else(|| {
                    "Select a raw disk image or device in Sources, or open a search report.".into()
                });
            text(b, "source", &caption);
        }
    }
    fn partition_controls(&self, running: bool) {
        if let Some(b) = self.dialog.borrow().as_ref() {
            for id in ["settings", "open_report", "partitions"] {
                obj::<gtk::Widget>(b, id).set_sensitive(!running);
            }
            obj::<gtk::Button>(b, "search")
                .set_sensitive(!running && self.partition_source.borrow().is_some());
            obj::<gtk::Button>(b, "confirm").set_sensitive(
                !running
                    && self
                        .partition_report
                        .borrow()
                        .as_ref()
                        .is_some_and(|r| r.complete)
                    && obj::<gtk::ListBox>(b, "partitions")
                        .selected_row()
                        .is_some(),
            );
            obj::<gtk::Button>(b, "close").set_label(if running {
                "Cancel search"
            } else {
                "Close"
            });
            let spinner: gtk::Spinner = obj(b, "spinner");
            spinner.set_visible(running);
            spinner.set_spinning(running);
        }
    }
    pub(super) fn show_partition_report(&self, report: discovery::Report) {
        self.partition_source.replace(Some(report.source.clone()));
        self.partition_report.replace(Some(report.clone()));
        self.partition_source_label();
        if let Some(b) = self.dialog.borrow().as_ref() {
            obj::<gtk::Entry>(b, "workspace").set_text(&report.options.workspace.to_string_lossy());
            obj::<gtk::DropDown>(b, "table").set_selected(match report.options.table {
                discovery::Table::Auto => 0,
                discovery::Table::Intel => 1,
                discovery::Table::Gpt => 2,
            });
            obj::<gtk::DropDown>(b, "depth").set_selected(report.options.deeper as u32);
            let list: gtk::ListBox = obj(b, "partitions");
            clear(&list);
            for (i, p) in report.partitions.iter().enumerate() {
                let caption = format!(
                    "{} · {}{} · {}\nByte {} · {}",
                    i + 1,
                    if p.filesystem.is_empty() {
                        "Unknown filesystem"
                    } else {
                        &p.filesystem
                    },
                    if p.label.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", p.label)
                    },
                    model::size(p.length_bytes),
                    p.offset_bytes,
                    if p.verified {
                        "Filesystem header verified"
                    } else {
                        "Unverified candidate"
                    }
                );
                let label = gtk::Label::new(Some(&caption));
                label.set_xalign(0.0);
                label.set_wrap(true);
                label.set_max_width_chars(72);
                label.set_margin_top(8);
                label.set_margin_bottom(8);
                label.set_margin_start(8);
                label.set_margin_end(8);
                list.append(&label);
            }
            let status = if let Some(error) = &report.error {
                error.clone()
            } else if report.partitions.is_empty() {
                "Search complete · no partition candidates found. Try Deeper Search or another table type.".into()
            } else {
                format!(
                    "Search complete · {} candidate(s) · {}",
                    report.partitions.len(),
                    report.table
                )
            };
            text(
                b,
                "validation",
                &format!(
                    "{status}{}",
                    if report.warnings.is_empty() {
                        String::new()
                    } else {
                        format!("\n{}", report.warnings.join("\n"))
                    }
                ),
            );
            text(
                b,
                "details",
                "Select a candidate to inspect its exact byte range. Starting a file scan retains that range in the workspace. Use Custom Scan → Include existing files to list files lost with the partition.",
            );
            let caption = format!("Report: {}", report.path().display());
            text(b, "report", &caption);
            obj::<gtk::Label>(b, "report").set_tooltip_text(Some(&caption));
        }
        self.partition_controls(false);
    }
    pub(super) fn poll_partition_search(self: &Rc<Self>) {
        let result = self.partition_search.borrow().as_ref().and_then(|search| {
            match search.result.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(engine::fail(
                    "The partition search worker stopped unexpectedly.",
                )),
            }
        });
        let Some(result) = result else {
            return;
        };
        self.partition_search.replace(None);
        let same_dialog = self.partition_generation.get() == self.dialog_generation.get();
        match result {
            Ok(report) => {
                self.event(&format!(
                    "Partition search {} · {} candidate(s) · {}",
                    if report.complete {
                        "complete"
                    } else if report.cancelled {
                        "cancelled"
                    } else {
                        "failed"
                    },
                    report.partitions.len(),
                    report.path().display()
                ));
                if same_dialog {
                    self.show_partition_report(report);
                }
            }
            Err(error) => {
                self.event(&error.to_string());
                if same_dialog {
                    if let Some(b) = self.dialog.borrow().as_ref() {
                        text(b, "validation", &error.to_string());
                    }
                    self.partition_controls(false);
                }
            }
        }
        self.controls();
        if self.close_partition_search.replace(false) && same_dialog {
            if let Some(b) = self.dialog.borrow().as_ref() {
                obj::<gtk::Window>(b, "dialog").close();
            }
        }
        if self.quitting.get() {
            self.window().close();
        }
    }
}
