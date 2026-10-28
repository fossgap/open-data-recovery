use super::*;

fn set_choice(b: &gtk::Builder, id: &str, value: &str) {
    let drop: gtk::DropDown = obj(b, id);
    if let Some(model) = drop.model() {
        for i in 0..model.n_items() {
            if model
                .item(i)
                .and_downcast::<gtk::StringObject>()
                .is_some_and(|s| s.string() == value)
            {
                drop.set_selected(i);
                break;
            }
        }
    }
}
fn put(b: &gtk::Builder, id: &str, value: impl ToString) {
    obj::<gtk::Entry>(b, id).set_text(&value.to_string());
}
fn put_optional(b: &gtk::Builder, id: &str, value: Option<u64>) {
    put(b, id, value.map(|n| n.to_string()).unwrap_or_default());
}
fn active(b: &gtk::Builder, id: &str) -> bool {
    obj::<gtk::Widget>(b, id).is_sensitive()
}
fn optional_number(b: &gtk::Builder, id: &str) -> engine::Result<Option<u64>> {
    if !active(b, id) || value(b, id).is_empty() {
        return Ok(None);
    }
    value(b, id)
        .parse()
        .map(Some)
        .map_err(|_| engine::Error::Message(format!("{id}: enter a nonnegative integer.")))
}
fn numeric(b: &gtk::Builder, id: &str) -> engine::Result<u64> {
    Ok(optional_number(b, id)?.unwrap_or(0))
}
fn optional_u32(b: &gtk::Builder, id: &str) -> engine::Result<Option<u32>> {
    optional_number(b, id)?
        .map(|n| {
            u32::try_from(n).map_err(|_| engine::Error::Message(format!("{id} is too large.")))
        })
        .transpose()
}
fn enabled_value(b: &gtk::Builder, id: &str) -> String {
    if active(b, id) {
        value(b, id)
    } else {
        String::new()
    }
}
fn path(b: &gtk::Builder, id: &str) -> Option<PathBuf> {
    let s = enabled_value(b, id);
    (!s.is_empty()).then(|| s.into())
}
pub(super) fn read_imaging(b: &gtk::Builder) -> engine::Result<DdrescueOptions> {
    Ok(DdrescueOptions {
        direct: checked(b, "dd_direct"),
        reverse: checked(b, "dd_reverse"),
        scrape: checked(b, "dd_scrape"),
        min_read_rate: numeric(b, "dd_min_rate")?,
        max_read_rate: numeric(b, "dd_max_rate")?,
        read_timeout: numeric(b, "dd_timeout")?,
        max_read_errors: optional_number(b, "dd_errors")?,
        import_image: b
            .object::<gtk::Entry>("import_image")
            .and_then(|_| path(b, "import_image")),
        import_map: b
            .object::<gtk::Entry>("import_map")
            .and_then(|_| path(b, "import_map")),
    })
}
pub(super) fn read(b: &gtk::Builder, o: &mut ScanOptions) -> engine::Result<()> {
    o.capture_metadata = active(b, "capture_metadata") && checked(b, "capture_metadata");
    o.metadata_sequence = enabled_value(b, "metadata_sequence")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if active(b, "photorec_options") {
        o.photorec = PhotoRecOptions {
            families: enabled_value(b, "photo_families")
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect(),
            free_space: obj::<gtk::DropDown>(b, "photo_space").selected() == 1,
            brute_force: checked(b, "photo_brute"),
            block_size: optional_u32(b, "photo_block")?,
            ..Default::default()
        };
    }
    if active(b, "ddrescue_options") {
        o.ddrescue = read_imaging(b)?;
    }
    if active(b, "ext_options") {
        o.ext = ExtOptions {
            target_kind: choice(b, "ext_kind"),
            target: enabled_value(b, "ext_target"),
            after: optional_number(b, "ext_after")?,
            before: optional_number(b, "ext_before")?,
            journal: path(b, "ext_journal"),
            superblock: optional_number(b, "ext_super")?,
            block_size: optional_u32(b, "ext_block")?,
            transaction: optional_number(b, "ext_transaction")?,
        };
    }
    if active(b, "btrfs_options") {
        o.btrfs = BtrfsOptions {
            subvolume: optional_number(b, "btrfs_subvolume")?,
            subvolume_root: optional_number(b, "btrfs_subroot")?,
            superblock: obj::<gtk::DropDown>(b, "btrfs_super").selected() as u8,
            snapshots: checked(b, "btrfs_snapshots"),
            path: enabled_value(b, "btrfs_path"),
            ignore_errors: checked(b, "btrfs_errors"),
        };
    }
    o.interpretation = Interpretation {
        filesystem: choice(b, "filesystem"),
        sector_size: match choice(b, "sector_size").as_str() {
            "auto" => None,
            n => Some(
                n.parse()
                    .map_err(|_| engine::Error::Message("Invalid sector size.".into()))?,
            ),
        },
        directory_inode: optional_number(b, "directory_inode")?,
    };
    if active(b, "ntfs_options") {
        o.ntfs = NtfsOptions {
            pattern: enabled_value(b, "ntfs_pattern"),
            inodes: enabled_value(b, "ntfs_inodes"),
            minimum_percent: u8::try_from(numeric(b, "ntfs_percent")?)
                .map_err(|_| engine::Error::Message("NTFS percentage must be 0–100.".into()))?,
            case_sensitive: checked(b, "ntfs_case"),
        };
    }
    Ok(())
}
fn fill_imaging(b: &gtk::Builder, o: &ScanOptions) {
    for (id, v) in [
        ("dd_direct", o.ddrescue.direct),
        ("dd_reverse", o.ddrescue.reverse),
        ("dd_scrape", o.ddrescue.scrape),
    ] {
        obj::<gtk::CheckButton>(b, id).set_active(v);
    }
    for (id, v) in [
        ("retries", o.retry_passes as u64),
        ("dd_min_rate", o.ddrescue.min_read_rate),
        ("dd_max_rate", o.ddrescue.max_read_rate),
        ("dd_timeout", o.ddrescue.read_timeout),
    ] {
        put(b, id, v);
    }
    put_optional(b, "dd_errors", o.ddrescue.max_read_errors);
}
fn fill(b: &gtk::Builder, o: &ScanOptions) {
    for (id, v) in [
        ("metadata", o.metadata_backend.as_str()),
        ("carver", &o.carver),
        ("clone_engine", &o.clone_backend),
        ("ext_kind", &o.ext.target_kind),
        ("filesystem", &o.interpretation.filesystem),
    ] {
        set_choice(b, id, v);
    }
    set_choice(
        b,
        "sector_size",
        &o.interpretation
            .sector_size
            .map_or("auto".into(), |n| n.to_string()),
    );
    for (id, v) in [
        ("extensions", o.extensions.join(", ")),
        ("metadata_sequence", o.metadata_sequence.join(", ")),
        ("ext_target", o.ext.target.clone()),
        (
            "ext_journal",
            o.ext
                .journal
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
        ("btrfs_path", o.btrfs.path.clone()),
        ("ntfs_pattern", o.ntfs.pattern.clone()),
        ("ntfs_inodes", o.ntfs.inodes.clone()),
        (
            "import_image",
            o.ddrescue
                .import_image
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
        (
            "import_map",
            o.ddrescue
                .import_map
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
    ] {
        put(b, id, v);
    }
    for (id, v) in [
        ("existing", o.include_existing),
        ("consolidate", o.consolidate),
        ("capture_metadata", o.capture_metadata),
        ("corrupted", o.keep_corrupted),
        ("low_memory", o.low_memory),
        ("photo_brute", o.photorec.brute_force),
        ("btrfs_snapshots", o.btrfs.snapshots),
        ("btrfs_errors", o.btrfs.ignore_errors),
        ("ntfs_case", o.ntfs.case_sensitive),
    ] {
        obj::<gtk::CheckButton>(b, id).set_active(v);
    }
    for (id, v) in [
        ("minimum", o.minimum_size),
        ("offset", o.offset_bytes),
        ("timeout", o.timeout_seconds),
        ("ntfs_percent", o.ntfs.minimum_percent as u64),
    ] {
        put(b, id, v);
    }
    for (id, v) in [
        ("maximum", o.maximum_size),
        ("length", o.length_bytes),
        ("btrfs_root", o.btrfs_root),
        ("photo_block", o.photorec.block_size.map(u64::from)),
        ("ext_after", o.ext.after),
        ("ext_before", o.ext.before),
        ("ext_super", o.ext.superblock),
        ("ext_block", o.ext.block_size.map(u64::from)),
        ("ext_transaction", o.ext.transaction),
        ("btrfs_subvolume", o.btrfs.subvolume),
        ("btrfs_subroot", o.btrfs.subvolume_root),
        ("directory_inode", o.interpretation.directory_inode),
    ] {
        put_optional(b, id, v);
    }
    obj::<gtk::DropDown>(b, "photo_space").set_selected(o.photorec.free_space as u32);
    obj::<gtk::DropDown>(b, "btrfs_super").set_selected(o.btrfs.superblock as u32);
    put(b, "photo_families", o.photorec.families.join(", "));
    fill_imaging(b, o);
}
pub(super) fn context(ui: &Live, b: &gtk::Builder) {
    let source = ui
        .selected
        .get()
        .and_then(|i| ui.sources.borrow().get(i).cloned());
    let versions = ui.custom_versions.borrow();
    let mode = mode(obj::<gtk::DropDown>(b, "mode").selected());
    let mut metadata = vec![];
    let mut carver = None;
    if let (Some(source), Some(versions)) = (&source, versions.as_ref()) {
        let mut o = ScanOptions {
            mode,
            metadata_backend: choice(b, "metadata"),
            carver: choice(b, "carver"),
            ..Default::default()
        };
        o.interpretation.filesystem = choice(b, "filesystem");
        if o.metadata_backend == "auto" {
            o.metadata_sequence = value(b, "metadata_sequence")
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
        }
        if let Ok(plan) = engine::plan::resolve(source, &o, versions) {
            metadata = plan.metadata;
            carver = plan.carver;
        } else {
            if mode != Mode::Deep && o.metadata_backend != "auto" {
                metadata.push(o.metadata_backend.clone());
            }
            if mode != Mode::Quick {
                carver = adapters::carving_engine(&o, versions).ok();
            }
        }
    }
    let has = |name: &str| metadata.iter().any(|n| n == name);
    for (id, on) in [
        ("photorec_options", carver.as_deref() == Some("photorec")),
        (
            "ddrescue_options",
            checked(b, "clone") && choice(b, "clone_engine") == "ddrescue",
        ),
        ("ext_options", has("ext4magic") || has("extundelete")),
        ("btrfs_options", has("btrfs")),
        ("ntfs_options", has("ntfsundelete")),
        ("metadata", mode != Mode::Deep),
        (
            "metadata_sequence",
            mode != Mode::Deep && choice(b, "metadata") == "auto",
        ),
        ("carver", mode != Mode::Quick),
        ("clone_engine", checked(b, "clone")),
        ("directory_inode", has("tsk")),
        (
            "existing",
            has("tsk") || has("tsk_recover") || has("ext4magic"),
        ),
        ("capture_metadata", !metadata.is_empty()),
    ] {
        obj::<gtk::Widget>(b, id).set_sensitive(on);
    }
    for (enabled, ids) in [
        (
            carver.as_deref() == Some("photorec"),
            &[
                "photo_space",
                "photo_brute",
                "photo_block",
                "photo_families",
                "corrupted",
                "low_memory",
            ][..],
        ),
        (
            checked(b, "clone") && choice(b, "clone_engine") == "ddrescue",
            &[
                "retries",
                "dd_direct",
                "dd_reverse",
                "dd_scrape",
                "dd_min_rate",
                "dd_max_rate",
                "dd_timeout",
                "dd_errors",
                "import_image",
                "import_map",
            ][..],
        ),
        (
            has("ext4magic") || has("extundelete"),
            &["ext_kind", "ext_after", "ext_before", "ext_journal"][..],
        ),
        (
            has("btrfs"),
            &[
                "btrfs_root",
                "btrfs_subvolume",
                "btrfs_subroot",
                "btrfs_super",
                "btrfs_path",
                "btrfs_snapshots",
                "btrfs_errors",
                "btrfs_list",
                "btrfs_preview",
            ][..],
        ),
        (
            has("ntfsundelete"),
            &["ntfs_pattern", "ntfs_inodes", "ntfs_percent", "ntfs_case"][..],
        ),
    ] {
        for id in ids {
            obj::<gtk::Widget>(b, id).set_sensitive(enabled);
        }
    }
    for id in ["ext_super", "ext_block"] {
        obj::<gtk::Widget>(b, id).set_sensitive(has("extundelete") && !has("ext4magic"));
    }
    obj::<gtk::Widget>(b, "ext_transaction").set_sensitive(has("ext4magic") && !has("extundelete"));
    obj::<gtk::Widget>(b, "ext_target")
        .set_sensitive((has("ext4magic") || has("extundelete")) && choice(b, "ext_kind") != "all");
    text(
        b,
        "engine_context",
        &format!(
            "Metadata: {}\nCarver: {}\nDisabled controls do not apply to this engine/mode combination.",
            if metadata.is_empty() {
                "none / unavailable".into()
            } else {
                metadata.join(" → ")
            },
            carver.unwrap_or("none / unavailable".into())
        ),
    );
    obj::<gtk::Button>(b, "confirm").set_sensitive(source.is_some() && versions.is_some());
}
impl Live {
    pub(super) fn custom_scan_dialog(self: &Rc<Self>) {
        let b = self.show(include_str!("../../ui/scan-options.ui"));
        self.custom_versions.replace(None);
        let options = self.options.borrow().clone();
        fill(&b, &options);
        obj::<gtk::DropDown>(&b, "mode")
            .set_selected(self.widget::<gtk::DropDown>("scan_mode").selected());
        put(&b, "workspace", options.workspace.display());
        obj::<gtk::CheckButton>(&b, "clone")
            .set_active(self.widget::<gtk::CheckButton>("clone_first").is_active());
        if let Some(partition) = self
            .selected
            .get()
            .and_then(|i| self.sources.borrow().get(i)?.partition.clone())
        {
            for (id, v) in [
                ("offset", partition.offset_bytes),
                ("length", partition.length_bytes),
            ] {
                put(&b, id, v);
                obj::<gtk::Entry>(&b, id).set_sensitive(false);
            }
        }
        Self::browse(&b, "browse", "workspace", "chooser");
        for id in [
            "mode",
            "metadata",
            "carver",
            "clone_engine",
            "filesystem",
            "ext_kind",
        ] {
            let weak = Rc::downgrade(self);
            let dialog = b.downgrade();
            obj::<gtk::DropDown>(&b, id).connect_selected_notify(move |_| {
                if let (Some(ui), Some(b)) = (weak.upgrade(), dialog.upgrade()) {
                    context(&ui, &b);
                }
            });
        }
        let weak = Rc::downgrade(self);
        let dialog = b.downgrade();
        obj::<gtk::CheckButton>(&b, "clone").connect_toggled(move |_| {
            if let (Some(ui), Some(b)) = (weak.upgrade(), dialog.upgrade()) {
                context(&ui, &b);
            }
        });
        let weak = Rc::downgrade(self);
        let dialog = b.downgrade();
        obj::<gtk::Entry>(&b, "metadata_sequence").connect_changed(move |_| {
            if let (Some(ui), Some(b)) = (weak.upgrade(), dialog.upgrade()) {
                context(&ui, &b);
            }
        });
        let reviewed: Rc<RefCell<Option<(SourceDescriptor, ScanOptions)>>> =
            Rc::new(RefCell::new(None));
        let state = reviewed.clone();
        let dialog = b.downgrade();
        obj::<gtk::Button>(&b, "back").connect_clicked(move |_| {
            if let Some(b) = dialog.upgrade() {
                state.replace(None);
                obj::<gtk::Notebook>(&b, "options_tabs").set_visible(true);
                obj::<gtk::Box>(&b, "plan_view").set_visible(false);
                obj::<gtk::Button>(&b, "back").set_visible(false);
                obj::<gtk::Button>(&b, "confirm").set_label("Next");
                text(&b, "validation", "");
            }
        });
        let weak = Rc::downgrade(self);
        let dialog = b.downgrade();
        obj::<gtk::Button>(&b, "confirm").connect_clicked(move |_| {
            let (Some(ui), Some(b)) = (weak.upgrade(), dialog.upgrade()) else {
                return;
            };
            if let Some((source, options)) = reviewed.borrow_mut().take() {
                let current = ui
                    .selected
                    .get()
                    .and_then(|i| ui.sources.borrow().get(i).cloned());
                if !current.as_ref().is_some_and(|s| s.same_target(&source)) {
                    text(
                        &b,
                        "validation",
                        "The source changed. Go back and review the plan again.",
                    );
                    return;
                }
                obj::<gtk::Window>(&b, "dialog").close();
                ui.widget::<gtk::DropDown>("scan_mode")
                    .set_selected(index(options.mode));
                ui.begin(options);
                return;
            }
            let result =
                (|| -> engine::Result<(SourceDescriptor, ScanOptions, engine::plan::ScanPlan)> {
                    let source = ui
                        .selected
                        .get()
                        .and_then(|i| ui.sources.borrow().get(i).cloned())
                        .ok_or_else(|| engine::Error::Message("Select a source first.".into()))?;
                    let mut options = scan_options(&b)?;
                    let versions = ui.custom_versions.borrow();
                    let versions = versions.as_ref().ok_or_else(|| {
                        engine::Error::Message("Checking installed engines…".into())
                    })?;
                    let plan = engine::plan::resolve(&source, &options, versions)?;
                    options.metadata_sequence = plan.metadata.clone();
                    if let Some(carver) = &plan.carver {
                        options.carver = carver.clone();
                    }
                    Ok((source, options, plan))
                })();
            match result {
                Ok((source, options, plan)) => {
                    text(&b, "plan_summary", &plan.description);
                    reviewed.replace(Some((source, options)));
                    obj::<gtk::Notebook>(&b, "options_tabs").set_visible(false);
                    obj::<gtk::Box>(&b, "plan_view").set_visible(true);
                    obj::<gtk::Button>(&b, "back").set_visible(true);
                    obj::<gtk::Button>(&b, "confirm").set_label("Start scan");
                    text(&b, "validation", "");
                }
                Err(e) => text(&b, "validation", &e.to_string()),
            }
        });
        for (id, roots) in [("btrfs_list", true), ("btrfs_preview", false)] {
            let weak = Rc::downgrade(self);
            let dialog = b.downgrade();
            obj::<gtk::Button>(&b, id).connect_clicked(move |_| {
                let (Some(ui), Some(b)) = (weak.upgrade(), dialog.upgrade()) else {
                    return;
                };
                let Some(source) = ui
                    .selected
                    .get()
                    .and_then(|i| ui.sources.borrow().get(i).cloned())
                else {
                    return;
                };
                match scan_options(&b) {
                    Ok(o) => {
                        let generation = ui.dialog_generation.get();
                        text(&b, "inspection_report", "Inspecting Btrfs…");
                        ui.submit(move || {
                            Message::BtrfsInspection(
                                generation,
                                engine::advanced::inspect_btrfs(&source, &o, roots),
                            )
                        });
                    }
                    Err(e) => text(&b, "validation", &e.to_string()),
                }
            });
        }
        context(self, &b);
        let generation = self.dialog_generation.get();
        self.submit(move || Message::CustomEngines(generation, adapters::versions()));
    }
    pub(super) fn resume_imaging_dialog(self: &Rc<Self>, scan: SavedScan) {
        let b = self.show(include_str!("../../ui/resume-imaging.ui"));
        fill_imaging(&b, &scan.manifest.options);
        text(
            &b,
            "summary",
            &format!(
                "{}\n{}\nContinue the saved acquisition using its existing clone and map. Scanning remains a separate action.",
                scan.manifest.source.path.display(),
                scan.directory.display()
            ),
        );
        let weak = Rc::downgrade(self);
        let dialog = b.downgrade();
        obj::<gtk::Button>(&b, "confirm").connect_clicked(move |_| {
            let (Some(ui), Some(b)) = (weak.upgrade(), dialog.upgrade()) else {
                return;
            };
            let result = (|| -> engine::Result<_> {
                let tuning = read_imaging(&b)?;
                let retries = u32::try_from(numeric(&b, "retries")?)
                    .map_err(|_| engine::Error::Message("Too many retries.".into()))?;
                let mut o = scan.manifest.options.clone();
                o.ddrescue = tuning.clone();
                o.retry_passes = retries;
                o.validate()?;
                Ok((tuning, retries))
            })();
            match result {
                Ok((tuning, retries)) => {
                    obj::<gtk::Window>(&b, "dialog").close();
                    ui.view_source.replace(Some(scan.manifest.source.clone()));
                    ui.invalidate_view();
                    ui.page.replace(Page::default());
                    ui.directory.replace(Some(scan.directory.clone()));
                    ui.job.replace(Some(jobs::start_imaging_resume(
                        scan.directory.clone(),
                        tuning,
                        retries,
                    )));
                    ui.pulse.set(true);
                    ui.paused.set(false);
                    ui.label("progress_text", "");
                    ui.label("status_text", "Resuming imaging…");
                    ui.render_files();
                    ui.controls();
                }
                Err(e) => text(&b, "validation", &e.to_string()),
            }
        });
    }
}
