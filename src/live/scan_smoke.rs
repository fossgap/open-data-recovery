use super::*;
pub(super) fn run(ui: &Rc<Live>) -> bool {
    let (Some(source), Some(workspace)) = (
        std::env::var_os("ODR_SMOKE_ADVANCED_SOURCE"),
        std::env::var_os("ODR_SMOKE_ADVANCED_WORKSPACE"),
    ) else {
        return false;
    };
    let source = PathBuf::from(source);
    let workspace = PathBuf::from(workspace);
    ui.add_source();
    let b = ui.dialog.borrow().clone().unwrap();
    obj::<gtk::Entry>(&b, "path").set_text(&source.to_string_lossy());
    obj::<gtk::Button>(&b, "confirm").emit_clicked();
    let weak = Rc::downgrade(ui);
    let mut step = 0;
    let mut ticks = 0;
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let Some(ui) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        ticks += 1;
        assert!(
            ticks < 400,
            "Advanced GTK workflow timed out at step {step}: {}",
            ui.widget::<gtk::Label>("status_text").text()
        );
        match step {
            0 if ui
                .selected
                .get()
                .and_then(|i| ui.sources.borrow().get(i).map(|s| s.path.clone()))
                == Some(source.clone()) =>
            {
                ui.custom_scan();
                step = 1;
            }
            1 if ui.custom_versions.borrow().is_some() => {
                let b = ui.dialog.borrow().clone().unwrap();
                obj::<gtk::Entry>(&b, "workspace").set_text(&workspace.to_string_lossy());
                obj::<gtk::DropDown>(&b, "carver").set_selected(1); // PhotoRec
                obj::<gtk::DropDown>(&b, "mode").set_selected(2);
                assert!(obj::<gtk::CheckButton>(&b, "low_memory").is_sensitive());
                assert!(!checked(&b, "low_memory"));
                obj::<gtk::DropDown>(&b, "carver").set_selected(3); // Foremost
                assert!(!obj::<gtk::CheckButton>(&b, "low_memory").is_sensitive());
                obj::<gtk::DropDown>(&b, "metadata").set_selected(4); // extundelete
                assert!(obj::<gtk::Entry>(&b, "ext_super").is_sensitive());
                assert!(!obj::<gtk::Entry>(&b, "ext_transaction").is_sensitive());
                obj::<gtk::DropDown>(&b, "metadata").set_selected(1); // TSK
                obj::<gtk::DropDown>(&b, "mode").set_selected(0); // Quick
                obj::<gtk::CheckButton>(&b, "existing").set_active(true);
                assert!(!obj::<gtk::DropDown>(&b, "carver").is_sensitive());
                assert!(!obj::<gtk::Entry>(&b, "ext_super").is_sensitive());
                assert!(obj::<gtk::Entry>(&b, "directory_inode").is_sensitive());
                obj::<gtk::CheckButton>(&b, "clone").set_active(true);
                assert!(obj::<gtk::CheckButton>(&b, "dd_direct").is_sensitive());
                obj::<gtk::CheckButton>(&b, "clone").set_active(false);
                assert!(!obj::<gtk::CheckButton>(&b, "dd_direct").is_sensitive());
                obj::<gtk::Button>(&b, "confirm").emit_clicked();
                assert!(obj::<gtk::Box>(&b, "plan_view").is_visible());
                assert!(!workspace.exists(), "Next must not create a scan");
                assert!(obj::<gtk::Label>(&b, "plan_summary").text().contains("tsk"));
                obj::<gtk::Button>(&b, "back").emit_clicked();
                assert!(obj::<gtk::Notebook>(&b, "options_tabs").is_visible());
                assert!(checked(&b, "existing"));
                obj::<gtk::Button>(&b, "confirm").emit_clicked();
                obj::<gtk::Button>(&b, "confirm").emit_clicked();
                step = 2;
            }
            2 if !ui.busy() && ui.directory.borrow().is_some() => {
                let store = Store::open(ui.directory.borrow().as_ref().unwrap()).unwrap();
                assert!(store.manifest.complete, "{:?}", store.manifest.errors);
                assert!(store.count().unwrap() > 0);
                assert_eq!(store.manifest.options.metadata_sequence, vec!["tsk"]);
                println!(
                    "Advanced GTK workflow passed: context sensitivity, defaults, Next/Back plan review, frozen execution."
                );
                ui.window().application().unwrap().quit();
                return glib::ControlFlow::Break;
            }
            _ => {}
        }
        glib::ControlFlow::Continue
    });
    true
}
