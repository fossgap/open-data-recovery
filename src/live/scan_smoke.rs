use super::*;
use crate::ui::engine_picker;
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
                assert!(b.object::<gtk::Widget>("metadata_sequence").is_none());
                assert!(b.object::<gtk::Widget>("plan_view").is_none());
                assert!(b.object::<gtk::Widget>("back").is_none());
                assert_eq!(
                    obj::<gtk::Button>(&b, "confirm").label().as_deref(),
                    Some(tr("Start scan"))
                );
                assert!(
                    obj::<gtk::Entry>(&b, "extensions")
                        .tooltip_text()
                        .unwrap()
                        .as_str()
                        == tr(
                            "Extension filters constrain extraction where supported; other engines filter results. File families can include several extensions."
                        )
                );
                assert!(
                    !obj::<gtk::Label>(&b, "engine_context")
                        .text()
                        .contains("Disabled controls")
                );
                let notebook: gtk::Notebook = obj(&b, "options_tabs");
                let tab = notebook
                    .tab_label(&notebook.nth_page(Some(2)).unwrap())
                    .unwrap()
                    .downcast::<gtk::Label>()
                    .unwrap();
                assert!(!tab.wraps(), "Engine options tab must stay on one line");
                assert!(!engine_picker::check(&b, "metadata", "ntfsundelete").is_sensitive());
                assert!(!engine_picker::check(&b, "metadata", "extundelete").is_sensitive());
                obj::<gtk::DropDown>(&b, "mode").set_selected(2); // Full
                engine_picker::check(&b, "carver", "photorec").set_active(true);
                assert!(obj::<gtk::CheckButton>(&b, "low_memory").is_sensitive());
                assert!(!checked(&b, "low_memory"));
                engine_picker::check(&b, "carver", "foremost").set_active(true);
                assert!(!obj::<gtk::CheckButton>(&b, "low_memory").is_sensitive());
                assert_eq!(engine_picker::selected(&b, "carver"), vec!["foremost"]);
                obj::<gtk::DropDown>(&b, "filesystem").set_selected(3); // ext4
                engine_picker::check(&b, "metadata", "extundelete").set_active(true);
                assert!(!engine_picker::check(&b, "metadata", "auto").is_active());
                assert!(obj::<gtk::Entry>(&b, "ext_super").is_sensitive());
                assert!(!obj::<gtk::Entry>(&b, "ext_transaction").is_sensitive());
                engine_picker::check(&b, "metadata", "tsk").set_active(true);
                assert_eq!(
                    engine_picker::selected(&b, "metadata"),
                    vec!["extundelete", "tsk"]
                );
                engine_picker::check(&b, "metadata", "auto").set_active(true);
                assert_eq!(engine_picker::selected(&b, "metadata"), vec!["auto"]);
                obj::<gtk::DropDown>(&b, "filesystem").set_selected(0); // detected ext2

                // Simulate missing tools, including a previously selected one.
                // Disabled rows must reject mouse/keyboard activation as well as
                // a programmatic toggle, without silently changing the choice.
                let installed = ui.custom_versions.borrow().as_ref().unwrap().clone();
                let mut missing = installed.clone();
                for name in ["foremost", "safecopy", "tsk_recover"] {
                    missing.remove(name);
                }
                ui.custom_versions.replace(Some(missing));
                scan::context(&ui, &b);
                assert!(!obj::<gtk::Button>(&b, "confirm").is_sensitive());
                obj::<gtk::Button>(&b, "confirm").emit_clicked();
                assert!(!ui.busy());
                assert!(!workspace.exists());
                engine_picker::check(&b, "carver", "photorec").set_active(true);
                for (id, name) in [
                    ("carver", "foremost"),
                    ("metadata", "tsk_recover"),
                    ("clone_engine", "safecopy"),
                ] {
                    let check = engine_picker::check(&b, id, name);
                    assert!(!check.get_sensitive());
                    assert!(
                        check.tooltip_text().unwrap().as_str()
                            == tr("Not installed. See Help → Backend availability.")
                    );
                    check.set_active(true);
                    assert!(!check.is_active());
                    assert!(!engine_picker::selected(&b, id).iter().any(|s| s == name));
                }
                obj::<gtk::CheckButton>(&b, "clone").set_active(true);
                assert!(obj::<gtk::CheckButton>(&b, "dd_direct").is_sensitive());
                obj::<gtk::CheckButton>(&b, "clone").set_active(false);
                assert!(!obj::<gtk::CheckButton>(&b, "dd_direct").is_sensitive());
                ui.custom_versions.replace(Some(installed));
                scan::context(&ui, &b);
                engine_picker::check(&b, "metadata", "tsk_recover").set_active(true);
                engine_picker::check(&b, "metadata", "tsk").set_active(true);
                assert_eq!(
                    engine_picker::selected(&b, "metadata"),
                    vec!["tsk_recover", "tsk"]
                );
                obj::<gtk::DropDown>(&b, "mode").set_selected(0); // Quick
                obj::<gtk::CheckButton>(&b, "existing").set_active(true);
                assert!(!obj::<gtk::MenuButton>(&b, "carver").is_sensitive());
                assert!(!obj::<gtk::Entry>(&b, "ext_super").is_sensitive());
                assert!(obj::<gtk::Entry>(&b, "directory_inode").is_sensitive());
                assert!(obj::<gtk::Button>(&b, "confirm").is_sensitive());
                assert!(!workspace.exists(), "Editing options must not start a scan");
                obj::<gtk::Button>(&b, "confirm").emit_clicked();
                assert!(ui.busy(), "Start scan must execute directly");
                step = 2;
            }
            2 if !ui.busy() && ui.directory.borrow().is_some() => {
                let store = Store::open(ui.directory.borrow().as_ref().unwrap()).unwrap();
                assert!(store.manifest.complete, "{:?}", store.manifest.errors);
                assert!(store.count().unwrap() > 0);
                assert_eq!(
                    store.manifest.options.metadata_sequence,
                    vec!["tsk_recover", "tsk"]
                );
                ui.custom_scan();
                let b = ui.dialog.borrow().clone().unwrap();
                assert_eq!(
                    engine_picker::selected(&b, "metadata"),
                    vec!["tsk_recover", "tsk"]
                );
                obj::<gtk::Window>(&b, "dialog").close();
                println!(
                    "Advanced GTK workflow passed: checked engine dropdowns, disabled unavailable choices, selection order, context sensitivity, direct start and reopened settings."
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
