//! Disposable-image GUI regression, enabled only by the existing smoke-test mode.
use super::*;

pub(super) fn run(ui: &Rc<Live>) -> bool {
    let Some(source) = std::env::var_os("ODR_SMOKE_DISCOVERY_SOURCE").map(PathBuf::from) else {
        return false;
    };
    let workspace = PathBuf::from(
        std::env::var_os("ODR_SMOKE_DISCOVERY_WORKSPACE").expect("discovery workspace"),
    );
    let session = PathBuf::from(std::env::var_os("ODR_SMOKE_SESSION").expect("saved scan"));
    let destination =
        PathBuf::from(std::env::var_os("ODR_SMOKE_DESTINATION").expect("recovery destination"));
    ui.open(session.clone());
    let weak = Rc::downgrade(ui);
    let mut step = 0;
    let mut ticks = 0;
    let mut report_path = PathBuf::new();
    let mut selected = 0;
    let mut preview = String::new();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let Some(ui) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        ticks += 1;
        assert!(
            ticks < 500,
            "Partition GUI workflow stalled at {step}: {} · {}",
            ui.widget::<gtk::Label>("status_text").text(),
            ui.dialog
                .borrow()
                .as_ref()
                .and_then(|b| b.object::<gtk::Label>("validation"))
                .map(|l| l.text().to_string())
                .unwrap_or_default()
        );
        match step {
            0 if ui.directory.borrow().as_ref() == Some(&session)
                && !ui.page.borrow().files.is_empty() =>
            {
                ui.widget::<gtk::CheckButton>("select_all").set_active(true);
                let id = ui.page.borrow().files[0].id;
                ui.preview(id);
                ui.widget::<gtk::SearchEntry>("search").set_text("txt");
                step = 1;
            }
            1 if ui.page.borrow().selected > 0
                && ui
                    .widget::<gtk::Label>("preview_text")
                    .text()
                    .contains("partition discovery") =>
            {
                selected = ui.page.borrow().selected;
                preview = ui.widget::<gtk::Label>("preview_text").text().to_string();
                ui.add_source();
                let b = ui.dialog.borrow().clone().unwrap();
                obj::<gtk::Entry>(&b, "path").set_text(&source.to_string_lossy());
                obj::<gtk::Button>(&b, "confirm").emit_clicked();
                step = 2;
            }
            2 if ui
                .selected
                .get()
                .is_some_and(|i| ui.sources.borrow()[i].path == source) =>
            {
                ui.find_partitions();
                let b = ui.dialog.borrow().clone().unwrap();
                obj::<gtk::Entry>(&b, "workspace").set_text(&workspace.to_string_lossy());
                obj::<gtk::DropDown>(&b, "table").set_selected(1);
                obj::<gtk::Button>(&b, "search").emit_clicked();
                assert!(ui.busy());
                // Closing an active search cancels its worker before dismissal.
                obj::<gtk::Button>(&b, "close").emit_clicked();
                step = 3;
            }
            3 if !ui.busy() => {
                assert_eq!(ui.directory.borrow().as_ref(), Some(&session));
                assert_eq!(ui.page.borrow().selected, selected);
                ui.find_partitions();
                let b = ui.dialog.borrow().clone().unwrap();
                obj::<gtk::Entry>(&b, "workspace").set_text(&workspace.to_string_lossy());
                obj::<gtk::DropDown>(&b, "table").set_selected(1);
                obj::<gtk::Button>(&b, "search").emit_clicked();
                step = 4;
            }
            4 if !ui.busy() && ui.partition_report.borrow().is_some() => {
                let report = ui.partition_report.borrow().clone().unwrap();
                assert!(report.complete, "{:?}", report.error);
                assert!(
                    report
                        .partitions
                        .iter()
                        .any(|p| p.offset_bytes == 2 * 1024 * 1024 && p.verified)
                );
                report_path = report.path();
                assert!(report_path.is_file());
                assert_eq!(ui.directory.borrow().as_ref(), Some(&session));
                assert_eq!(ui.page.borrow().selected, selected);
                assert_eq!(ui.query.borrow().text, "txt");
                assert_eq!(
                    ui.widget::<gtk::Label>("preview_text").text().as_str(),
                    preview
                );
                // Supply a selected path to the chooser's production handler.
                // Native/portal file selection belongs to GTK (or another
                // process) and cannot be synthesized with a response signal.
                ui.find_partitions();
                ui.open_partition_report(report_path.clone());
                step = 5;
            }
            5 if ui
                .partition_report
                .borrow()
                .as_ref()
                .is_some_and(|r| r.path() == report_path) =>
            {
                let report = ui.partition_report.borrow().clone().unwrap();
                let index = report
                    .partitions
                    .iter()
                    .position(|p| p.offset_bytes == 2 * 1024 * 1024 && p.verified)
                    .unwrap();
                let b = ui.dialog.borrow().clone().unwrap();
                let list: gtk::ListBox = obj(&b, "partitions");
                list.select_row(list.row_at_index(index as i32).as_ref());
                assert!(obj::<gtk::Button>(&b, "confirm").is_sensitive());
                // A report must refresh an existing target's identity, not
                // silently reuse stale samples for the same path and range.
                let mut stale = report.target(index).unwrap();
                stale.partition.as_mut().unwrap().sample_sha256 = "0".repeat(64);
                ui.sources.borrow_mut().push(stale);
                obj::<gtk::Button>(&b, "confirm").emit_clicked();
                let target = ui.sources.borrow()[ui.selected.get().unwrap()].clone();
                assert_eq!(target.partition.as_ref(), Some(&report.partitions[index]));
                assert_eq!(
                    ui.sources
                        .borrow()
                        .iter()
                        .filter(|s| s.same_target(&target))
                        .count(),
                    1
                );
                assert_eq!(
                    target.partition.as_ref().unwrap().offset_bytes,
                    2 * 1024 * 1024
                );
                assert_eq!(ui.directory.borrow().as_ref(), Some(&session));
                assert_eq!(ui.page.borrow().selected, selected);
                assert!(
                    ui.widget::<gtk::Label>("source_title")
                        .text()
                        .starts_with("Next scan:")
                );
                ui.custom_scan();
                let custom = ui.dialog.borrow().clone().unwrap();
                assert!(!obj::<gtk::Entry>(&custom, "offset").is_sensitive());
                assert_eq!(scan_options(&custom).unwrap().offset_bytes, 2 * 1024 * 1024);
                obj::<gtk::Window>(&custom, "dialog").close();
                ui.refresh();
                step = 6;
            }
            6 if ui
                .widget::<gtk::Label>("status_text")
                .text()
                .starts_with("Sources refreshed") =>
            {
                assert!(
                    ui.sources.borrow()[ui.selected.get().unwrap()]
                        .partition
                        .is_some()
                );
                assert_eq!(ui.directory.borrow().as_ref(), Some(&session));
                ui.widget::<gtk::DropDown>("scan_mode").set_selected(0);
                ui.widget::<gtk::CheckButton>("clone_first")
                    .set_active(false);
                ui.custom_scan();
                let b = ui.dialog.borrow().clone().unwrap();
                obj::<gtk::CheckButton>(&b, "existing").set_active(true);
                obj::<gtk::Button>(&b, "confirm").emit_clicked();
                step = 7;
            }
            7 if !ui.busy()
                && ui
                    .directory
                    .borrow()
                    .as_ref()
                    .is_some_and(|p| p != &session)
                && !ui.page.borrow().files.is_empty() =>
            {
                let directory = ui.directory.borrow().clone().unwrap();
                let store = Store::open(&directory).unwrap();
                assert!(store.manifest.complete, "{:?}", store.manifest.errors);
                assert!(store.manifest.source.partition.is_some());
                assert!(store.manifest.options.include_existing);
                assert!(
                    ui.page
                        .borrow()
                        .files
                        .iter()
                        .any(|f| f.path == "kept.txt" && !f.deleted)
                );
                assert_eq!(
                    std::fs::metadata(directory.join("source-range.img"))
                        .unwrap()
                        .len(),
                    16 * 1024 * 1024
                );
                ui.widget::<gtk::CheckButton>("select_all").set_active(true);
                step = 8;
            }
            8 if ui.page.borrow().selected > 0 => {
                ui.recovery();
                let b = ui.dialog.borrow().clone().unwrap();
                obj::<gtk::Entry>(&b, "destination_path").set_text(&destination.to_string_lossy());
                step = 9;
            }
            9 => {
                let b = ui.dialog.borrow().clone().unwrap();
                if obj::<gtk::Button>(&b, "confirm").is_sensitive() {
                    obj::<gtk::Button>(&b, "confirm").emit_clicked();
                    step = 10;
                }
            }
            10 if !ui.busy() => {
                assert!(destination.join("recovered.txt").is_file());
                assert!(destination.join("kept.txt").is_file());
                let directory = ui.directory.borrow().clone().unwrap();
                assert!(
                    std::fs::read_to_string(directory.join("events.log"))
                        .unwrap()
                        .contains("\"verified\":true")
                );
                println!(
                    "GTK partition discovery passed: cancel, search, report reopen, preserved open scan, virtual target, source refresh, range scan and verified recovery."
                );
                ui.window().close();
                return glib::ControlFlow::Break;
            }
            _ => {}
        }
        glib::ControlFlow::Continue
    });
    true
}
