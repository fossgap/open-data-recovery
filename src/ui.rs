use crate::model::{self, Filter, Kind, Phase, ScanMode, Session, Snapshot, Source, SourceKind};
use gtk::{gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeSet,
    rc::Rc,
    time::Duration,
};

const WINDOW: &str = include_str!("../ui/window.ui");
const SOURCE_ROW: &str = include_str!("../ui/source-row.ui");
const DISK_ROW: &str = include_str!("../ui/disk-row.ui");
const PARTITION_ROW: &str = include_str!("../ui/partition-row.ui");
const SOURCE_SECTION: &str = include_str!("../ui/source-section.ui");
const FILE_ROW: &str = include_str!("../ui/file-row.ui");
const FOLDER_ROW: &str = include_str!("../ui/folder-row.ui");
const EVENT_ROW: &str = include_str!("../ui/event-row.ui");
const RECOVERY: &str = include_str!("../ui/recovery.ui");
const ADD_SOURCE: &str = include_str!("../ui/add-source.ui");
const INFO: &str = include_str!("../ui/info.ui");

struct Workspace {
    sources: Vec<Source>,
    session: Session,
    snapshots: Vec<Snapshot>,
    filter: Filter,
    query: String,
    collapsed: BTreeSet<String>,
    collapsed_disks: BTreeSet<usize>,
    focused: Option<usize>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            sources: model::sources(),
            session: Session::example(0),
            snapshots: vec![Snapshot::capture(
                "Archive · full scan".into(),
                &Session::example(0),
            )],
            filter: Filter::All,
            query: String::new(),
            collapsed: ["Projects".into(), "Lost files".into()]
                .into_iter()
                .collect(),
            focused: Some(0),
            collapsed_disks: [8].into_iter().collect(),
        }
    }
}

#[derive(Clone)]
enum RowKey {
    Folder(String),
    File(usize),
}

struct Ui {
    builder: gtk::Builder,
    state: RefCell<Workspace>,
    row_keys: RefCell<Vec<RowKey>>,
    source_keys: RefCell<Vec<Option<usize>>>,
    source_maps: RefCell<Vec<(gtk::ListBoxRow, gtk::DrawingArea, Vec<usize>)>>,
    rendering: Cell<bool>,
    event_count: Cell<usize>,
    dialog: RefCell<Option<gtk::Builder>>,
}

fn object<T: IsA<glib::Object>>(builder: &gtk::Builder, id: &str) -> T {
    builder
        .object(id)
        .unwrap_or_else(|| panic!("Missing or incorrectly typed GtkBuilder object: {id}"))
}

impl Ui {
    fn widget<T: IsA<glib::Object>>(&self, id: &str) -> T {
        object(&self.builder, id)
    }
    fn window(&self) -> gtk::ApplicationWindow {
        self.widget("window")
    }
    fn label(&self, id: &str, text: &str) {
        self.widget::<gtk::Label>(id).set_text(text);
    }

    fn action(self: &Rc<Self>, name: &str, handler: fn(&Rc<Self>)) {
        let action = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(self);
        action.connect_activate(move |_, _| {
            if let Some(ui) = weak.upgrade() {
                handler(&ui);
            }
        });
        self.window().add_action(&action);
    }

    fn enable(&self, name: &str, enabled: bool) {
        self.window()
            .lookup_action(name)
            .unwrap()
            .downcast::<gio::SimpleAction>()
            .unwrap()
            .set_enabled(enabled);
    }

    fn connect(self: &Rc<Self>, app: &gtk::Application) {
        self.action("start", Self::start);
        self.action("pause", Self::pause);
        self.action("cancel", Self::cancel);
        self.action("save", Self::save);
        self.action("recover", Self::recover);
        self.action("add-source", Self::add_source);
        self.action("custom-scan", Self::custom_scan);
        self.action("saved-scans", Self::saved_scans);
        self.action("backends", |ui| ui.info("Demo mode", "The demonstration runs no recovery tools. Launch without --demo to discover devices, inspect installed backends, and perform real recovery."));
        self.action("reset", Self::reset);
        self.action("search", |ui| {
            ui.widget::<gtk::SearchEntry>("search").grab_focus();
        });
        self.action("about", |ui| ui.info("Open Data Recovery", "A native Linux recovery workspace.\n\nThis interactive design prototype uses Rust, GTK4, and declarative GtkBuilder layouts. Every source, file, preview, and destination is sample data. Saved scans live only in memory and disappear when the app closes.\n\nNo recovery backends, source access, disk writes, or permission prompts run in this prototype. The README requests layout validation before full implementation.\n\nVersion 0.1.0 · Design preview"));
        self.action("shortcuts", |ui| ui.info("Keyboard shortcuts", "Ctrl+O    Add a sample source\nCtrl+S    Save a demo snapshot\nCtrl+F    Search files\nCtrl+R    Start scan\nCtrl+P    Toggle file preview\nCtrl+L    Toggle activity log\nCtrl+Q    Quit\n\nTab / Shift+Tab    Move between controls\nSpace    Toggle a focused checkbox\nEnter    Expand or collapse a focused folder"));

        for (name, panel) in [("preview", "preview_panel"), ("events", "events_panel")] {
            let action = gio::SimpleAction::new_stateful(name, None, &true.to_variant());
            let weak = Rc::downgrade(self);
            action.connect_activate(move |action, _| {
                if let Some(ui) = weak.upgrade() {
                    let visible = !action
                        .state()
                        .and_then(|state| state.get::<bool>())
                        .unwrap_or(true);
                    action.set_state(&visible.to_variant());
                    ui.widget::<gtk::Box>(panel).set_visible(visible);
                }
            });
            self.window().add_action(&action);
        }
        let quit = gio::SimpleAction::new("quit", None);
        let weak_app = app.downgrade();
        quit.connect_activate(move |_, _| {
            if let Some(app) = weak_app.upgrade() {
                app.quit();
            }
        });
        app.add_action(&quit);
        for (action, key) in [
            ("win.add-source", "<Primary>o"),
            ("win.save", "<Primary>s"),
            ("win.search", "<Primary>f"),
            ("win.start", "<Primary>r"),
            ("win.custom-scan", "<Primary><Shift>r"),
            ("win.saved-scans", "<Primary><Shift>s"),
            ("win.preview", "<Primary>p"),
            ("win.events", "<Primary>l"),
            ("app.quit", "<Primary>q"),
        ] {
            app.set_accels_for_action(action, &[key]);
        }

        let weak = Rc::downgrade(self);
        self.widget::<gtk::ListBox>("sources")
            .connect_row_selected(move |_, row| {
                if let (Some(ui), Some(row)) = (weak.upgrade(), row) {
                    if !ui.rendering.get() {
                        let id = ui
                            .source_keys
                            .borrow()
                            .get(row.index() as usize)
                            .copied()
                            .flatten();
                        if let Some(id) = id {
                            ui.select_source(id);
                        }
                    }
                }
            });
        let weak = Rc::downgrade(self);
        self.widget::<gtk::ListBox>("sources")
            .connect_row_activated(move |_, row| {
                if let Some(ui) = weak.upgrade() {
                    let id = ui
                        .source_keys
                        .borrow()
                        .get(row.index() as usize)
                        .copied()
                        .flatten();
                    if let Some(id) = id {
                        ui.toggle_disk(id);
                    }
                }
            });
        let weak = Rc::downgrade(self);
        self.widget::<gtk::ListBox>("snapshots")
            .connect_row_activated(move |_, row| {
                if let Some(ui) = weak.upgrade() {
                    ui.open_snapshot(row.index() as usize);
                }
            });
        let weak = Rc::downgrade(self);
        self.widget::<gtk::SearchEntry>("search")
            .connect_search_changed(move |entry| {
                if let Some(ui) = weak.upgrade() {
                    if ui.rendering.get() {
                        return;
                    }
                    ui.state.borrow_mut().query = entry.text().to_string();
                    ui.render_files();
                }
            });
        for (name, filter) in [
            ("filter_all", Filter::All),
            ("filter_images", Filter::Images),
            ("filter_documents", Filter::Documents),
            ("filter_other", Filter::Other),
        ] {
            let weak = Rc::downgrade(self);
            self.widget::<gtk::ToggleButton>(name)
                .connect_toggled(move |button| {
                    if let Some(ui) = weak.upgrade() {
                        if ui.rendering.get() || !button.is_active() {
                            return;
                        }
                        ui.state.borrow_mut().filter = filter;
                        ui.render_files();
                    }
                });
        }
        let weak = Rc::downgrade(self);
        self.widget::<gtk::ListBox>("files")
            .connect_row_selected(move |_, row| {
                if let Some(ui) = weak.upgrade() {
                    if ui.rendering.get() {
                        return;
                    }
                    let id =
                        row.and_then(|row| match ui.row_keys.borrow().get(row.index() as usize) {
                            Some(RowKey::File(id)) => Some(*id),
                            _ => None,
                        });
                    ui.state.borrow_mut().focused = id;
                    ui.render_preview();
                }
            });
        let weak = Rc::downgrade(self);
        self.widget::<gtk::ListBox>("files")
            .connect_row_activated(move |_, row| {
                if let Some(ui) = weak.upgrade() {
                    let key = ui.row_keys.borrow().get(row.index() as usize).cloned();
                    if let Some(RowKey::Folder(folder)) = key {
                        ui.toggle_folder(&folder);
                    }
                }
            });
        let weak = Rc::downgrade(self);
        self.widget::<gtk::CheckButton>("select_all")
            .connect_toggled(move |button| {
                if let Some(ui) = weak.upgrade() {
                    if ui.rendering.get() {
                        return;
                    }
                    let mut state = ui.state.borrow_mut();
                    let ids: Vec<_> = state
                        .session
                        .found
                        .iter()
                        .filter(|entry| model::matches(entry, state.filter, &state.query))
                        .map(|entry| entry.id)
                        .collect();
                    for id in ids {
                        state.session.select(id, button.is_active());
                    }
                    drop(state);
                    ui.render_files();
                }
            });
        let weak = Rc::downgrade(self);
        self.widget::<gtk::DropDown>("scan_mode")
            .connect_selected_notify(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.render_hint();
                }
            });
        let weak = Rc::downgrade(self);
        self.widget::<gtk::Button>("clear_events")
            .connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    clear_list(&ui.widget("events"));
                    ui.event_count.set(0);
                    ui.label("event_count", "0 events");
                }
            });
    }

    fn render_sources(self: &Rc<Self>) {
        self.rendering.set(true);
        let list: gtk::ListBox = self.widget("sources");
        clear_list(&list);
        self.source_keys.borrow_mut().clear();
        self.source_maps.borrow_mut().clear();
        let state = self.state.borrow();
        for (id, source) in state
            .sources
            .iter()
            .enumerate()
            .filter(|(_, source)| source.kind == SourceKind::Disk)
        {
            let builder = gtk::Builder::from_string(DISK_ROW);
            object::<gtk::Label>(&builder, "name").set_text(&source.name);
            object::<gtk::Label>(&builder, "detail")
                .set_text(&format!("{} · {}", source.path, source.filesystem));
            object::<gtk::Label>(&builder, "capacity").set_text(source.capacity);
            object::<gtk::Image>(&builder, "icon").set_icon_name(Some(source.icon));
            let expand: gtk::Button = object(&builder, "expand");
            let collapsed = state.collapsed_disks.contains(&id);
            expand.set_icon_name(if collapsed {
                "pan-end-symbolic"
            } else {
                "pan-down-symbolic"
            });
            expand.set_tooltip_text(Some(&format!(
                "{} partitions on {}",
                if collapsed { "Expand" } else { "Collapse" },
                source.name
            )));
            let weak = Rc::downgrade(self);
            expand.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.toggle_disk(id);
                }
            });
            let children = model::disk_children(&state.sources, id);
            let selected = state.session.source;
            let row: gtk::ListBoxRow = object(&builder, "row");
            if children.contains(&selected) {
                row.add_css_class("contains-selected");
            }
            row.set_tooltip_text(Some(&format!(
                "{} · {} · {}\nSelect to scan the whole sample disk. Expand to select a partition.",
                source.name, source.path, source.capacity
            )));
            let map: gtk::DrawingArea = object(&builder, "layout");
            map.set_tooltip_text(Some(
                &children
                    .iter()
                    .map(|&child| {
                        let source = &state.sources[child];
                        format!(
                            "{} · {} · {}",
                            source.name, source.filesystem, source.capacity
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ));
            let segments: Vec<_> = children
                .iter()
                .map(|&child| {
                    let source = &state.sources[child];
                    (
                        source.size_mib,
                        source.kind == SourceKind::Unallocated,
                        child,
                    )
                })
                .collect();
            let weak = Rc::downgrade(self);
            map.set_draw_func(move |area, context, width, height| {
                let selected = weak.upgrade().map(|ui| ui.state.borrow().session.source);
                let segments: Vec<_> = segments
                    .iter()
                    .map(|&(size, unallocated, child)| {
                        (
                            size,
                            unallocated,
                            selected == Some(child) || selected == Some(id),
                        )
                    })
                    .collect();
                draw_disk_map(area, context, width, height, &segments)
            });
            self.source_maps
                .borrow_mut()
                .push((row.clone(), map, children.clone()));
            self.source_keys.borrow_mut().push(Some(id));
            list.append(&row);
            if collapsed {
                continue;
            }
            for child in children {
                let source = &state.sources[child];
                let builder = gtk::Builder::from_string(PARTITION_ROW);
                object::<gtk::Label>(&builder, "name").set_text(&source.name);
                object::<gtk::Label>(&builder, "capacity").set_text(source.capacity);
                object::<gtk::Label>(&builder, "detail").set_text(&if source.kind
                    == SourceKind::Unallocated
                {
                    "Unallocated · carve only".into()
                } else {
                    format!(
                        "{} · {}{}",
                        source.path.trim_start_matches("/dev/"),
                        source.filesystem,
                        if source.mounted { " · mounted" } else { "" }
                    )
                });
                let row: gtk::ListBoxRow = object(&builder, "row");
                row.set_tooltip_text(Some(&format!(
                    "{} · {} · {}\n{} on {}\nSample source — no device access",
                    source.name,
                    source.path,
                    source.capacity,
                    source.detail,
                    state.sources[id].name
                )));
                self.source_keys.borrow_mut().push(Some(child));
                list.append(&row);
            }
        }
        let standalone: Vec<_> = state
            .sources
            .iter()
            .enumerate()
            .filter(|(_, source)| source.parent.is_none() && source.kind != SourceKind::Disk)
            .collect();
        if !standalone.is_empty() {
            let builder = gtk::Builder::from_string(SOURCE_SECTION);
            self.source_keys.borrow_mut().push(None);
            list.append(&object::<gtk::ListBoxRow>(&builder, "row"));
        }
        for (id, source) in standalone {
            let builder = gtk::Builder::from_string(SOURCE_ROW);
            object::<gtk::Label>(&builder, "name").set_text(&source.name);
            object::<gtk::Label>(&builder, "detail").set_text(&source.detail);
            object::<gtk::Image>(&builder, "icon").set_icon_name(Some(source.icon));
            let row: gtk::ListBoxRow = object(&builder, "row");
            row.set_tooltip_text(Some(&format!(
                "{} · {}\nSample source — no device access",
                source.path, source.capacity
            )));
            row.add_css_class("standalone-source");
            self.source_keys.borrow_mut().push(Some(id));
            list.append(&row);
        }
        let selected = self
            .source_keys
            .borrow()
            .iter()
            .position(|&id| id == Some(state.session.source));
        list.select_row(
            selected
                .and_then(|index| list.row_at_index(index as i32))
                .as_ref(),
        );
        self.rendering.set(false);
    }

    fn refresh_source_highlights(&self) {
        let selected = self.state.borrow().session.source;
        for (row, map, children) in self.source_maps.borrow().iter() {
            if children.contains(&selected) {
                row.add_css_class("contains-selected");
            } else {
                row.remove_css_class("contains-selected");
            }
            map.queue_draw();
        }
    }

    fn toggle_disk(self: &Rc<Self>, id: usize) {
        let mut state = self.state.borrow_mut();
        if state.session.phase.busy()
            || state
                .sources
                .get(id)
                .is_none_or(|source| source.kind != SourceKind::Disk)
        {
            return;
        }
        if !state.collapsed_disks.remove(&id) {
            state.collapsed_disks.insert(id);
        }
        drop(state);
        self.render_sources();
    }

    fn render_snapshots(&self) {
        let list: gtk::ListBox = self.widget("snapshots");
        clear_list(&list);
        for snapshot in &self.state.borrow().snapshots {
            let builder = gtk::Builder::from_string(SOURCE_ROW);
            object::<gtk::Label>(&builder, "name").set_text(&snapshot.title);
            object::<gtk::Label>(&builder, "detail").set_text(&format!(
                "{} files · {}% · in memory",
                snapshot.session.found.len(),
                snapshot.session.progress
            ));
            object::<gtk::Image>(&builder, "icon")
                .set_icon_name(Some("document-open-recent-symbolic"));
            list.append(&object::<gtk::ListBoxRow>(&builder, "row"));
        }
    }

    fn render_hint(&self) {
        let state = self.state.borrow();
        let source = &state.sources[state.session.source];
        let mode = ScanMode::from_index(self.widget::<gtk::DropDown>("scan_mode").selected());
        self.label("mode_hint", if source.kind == SourceKind::Unallocated {
            "Unallocated space: search file signatures. Original names and folders may be lost."
        } else if source.carve_only {
            "This filesystem supports best-effort carving only. Original names may be lost."
        } else if source.mounted {
            "Live source: mounted filesystems can change during a scan. Clone-first is available."
        } else { mode.description() });
    }

    fn render_status(&self) {
        let state = self.state.borrow();
        let session = &state.session;
        let source = &state.sources[session.source];
        self.label("source_title", &source.name);
        self.label(
            "source_detail",
            &format!(
                "{}   ·   {}   ·   {}{}   ·   Sample source",
                source.path,
                source.filesystem,
                source.capacity,
                source
                    .parent
                    .map(|id| format!("   ·   on {}", state.sources[id].path))
                    .unwrap_or_default()
            ),
        );
        self.widget::<gtk::Image>("source_icon")
            .set_icon_name(Some(source.icon));
        self.label(
            "source_state",
            if source.mounted {
                "Read-only source · mounted"
            } else {
                "Read-only source"
            },
        );
        self.label(
            "status_text",
            &format!(
                "{} · {} · {} candidates",
                session.stage(),
                session.mode.name(),
                session.found.len()
            ),
        );
        let progress: gtk::ProgressBar = self.widget("progress");
        progress.set_fraction(session.progress as f64 / 100.0);
        progress.set_text(Some(&format!("{}%", session.progress)));
        for (class, active) in [
            ("paused", session.phase == Phase::Paused),
            ("cancelled", session.phase == Phase::Cancelled),
        ] {
            if active {
                progress.add_css_class(class);
            } else {
                progress.remove_css_class(class);
            }
        }
        self.label("progress_text", &format!("{}%", session.progress));
        self.widget::<gtk::Image>("status_icon")
            .set_icon_name(Some(match session.phase {
                Phase::Complete => "object-select-symbolic",
                Phase::Paused => "media-playback-pause-symbolic",
                Phase::Cancelled => "process-stop-symbolic",
                _ => "view-refresh-symbolic",
            }));
        let busy = session.phase.busy();
        self.enable("start", !busy);
        self.enable("pause", busy);
        self.enable("cancel", busy);
        self.enable("save", session.phase != Phase::Ready);
        self.enable("reset", !busy);
        self.enable("add-source", !busy);
        self.widget::<gtk::ListBox>("sources").set_sensitive(!busy);
        self.widget::<gtk::ListBox>("snapshots")
            .set_sensitive(!busy);
        self.widget::<gtk::DropDown>("scan_mode")
            .set_sensitive(!busy && !source.carve_only);
        self.widget::<gtk::CheckButton>("clone_first")
            .set_sensitive(!busy);
        self.widget::<gtk::Button>("start")
            .set_label(if session.phase == Phase::Ready {
                "Start scan"
            } else {
                "Scan again"
            });
        self.widget::<gtk::Button>("pause")
            .set_label(if session.phase == Phase::Paused {
                "Resume"
            } else {
                "Pause"
            });
        drop(state);
        self.render_selection();
        self.render_hint();
    }

    fn render_selection(&self) {
        let state = self.state.borrow();
        let session = &state.session;
        let count = session.selected.len();
        self.label(
            "selection_summary",
            &if count == 0 {
                "No files selected".into()
            } else {
                format!(
                    "{count} {} selected · {}",
                    if count == 1 { "file" } else { "files" },
                    model::size(session.selected_bytes())
                )
            },
        );
        self.enable("recover", count > 0 && !session.phase.busy());
    }

    fn render_files(self: &Rc<Self>) {
        self.rendering.set(true);
        let list: gtk::ListBox = self.widget("files");
        clear_list(&list);
        self.row_keys.borrow_mut().clear();
        let (session, filter, query, collapsed, focused) = {
            let state = self.state.borrow();
            (
                state.session.clone(),
                state.filter,
                state.query.clone(),
                state.collapsed.clone(),
                state.focused,
            )
        };
        let matching: Vec<_> = session
            .found
            .iter()
            .filter(|entry| model::matches(entry, filter, &query))
            .collect();
        let mut focused_row = None;
        for folder in model::FOLDERS {
            let children: Vec<_> = matching
                .iter()
                .filter(|entry| entry.folder == folder)
                .copied()
                .collect();
            if children.is_empty() {
                continue;
            }
            let builder = gtk::Builder::from_string(FOLDER_ROW);
            object::<gtk::Label>(&builder, "name").set_text(folder);
            object::<gtk::Label>(&builder, "count").set_text(&format!("{}", children.len()));
            let check: gtk::CheckButton = object(&builder, "check");
            let selected = children
                .iter()
                .filter(|entry| session.selected.contains(&entry.id))
                .count();
            check.set_active(selected == children.len());
            check.set_inconsistent(selected > 0 && selected < children.len());
            let ids: Vec<_> = children.iter().map(|entry| entry.id).collect();
            let weak = Rc::downgrade(self);
            check.connect_toggled(move |check| {
                if let Some(ui) = weak.upgrade() {
                    if ui.rendering.get() {
                        return;
                    }
                    for id in &ids {
                        ui.state.borrow_mut().session.select(*id, check.is_active());
                    }
                    ui.render_files();
                }
            });
            let expand: gtk::Button = object(&builder, "expand");
            expand.set_icon_name(if collapsed.contains(folder) {
                "pan-end-symbolic"
            } else {
                "pan-down-symbolic"
            });
            expand.set_tooltip_text(Some(&format!(
                "{} {folder}",
                if collapsed.contains(folder) {
                    "Expand"
                } else {
                    "Collapse"
                }
            )));
            let weak = Rc::downgrade(self);
            expand.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.toggle_folder(folder);
                }
            });
            self.row_keys
                .borrow_mut()
                .push(RowKey::Folder(folder.into()));
            list.append(&object::<gtk::ListBoxRow>(&builder, "row"));
            if collapsed.contains(folder) && query.is_empty() {
                continue;
            }
            for entry in children {
                let builder = gtk::Builder::from_string(FILE_ROW);
                object::<gtk::Label>(&builder, "name").set_text(entry.name);
                object::<gtk::Label>(&builder, "size").set_text(&model::size(entry.bytes));
                let status: gtk::Label = object(&builder, "status");
                status.set_text(entry.status());
                if entry.carved {
                    status.add_css_class("carved");
                }
                object::<gtk::Image>(&builder, "icon").set_icon_name(Some(entry.icon()));
                let check: gtk::CheckButton = object(&builder, "check");
                check.set_active(session.selected.contains(&entry.id));
                check.set_tooltip_text(Some(&format!("Select {} for recovery", entry.name)));
                let id = entry.id;
                let weak = Rc::downgrade(self);
                check.connect_toggled(move |check| {
                    if let Some(ui) = weak.upgrade() {
                        if ui.rendering.get() {
                            return;
                        }
                        ui.state.borrow_mut().session.select(id, check.is_active());
                        // Defer rebuilding so the emitting widget remains alive during its signal.
                        let weak = Rc::downgrade(&ui);
                        glib::idle_add_local_once(move || {
                            if let Some(ui) = weak.upgrade() {
                                ui.render_files();
                            }
                        });
                    }
                });
                self.row_keys.borrow_mut().push(RowKey::File(id));
                let row: gtk::ListBoxRow = object(&builder, "row");
                row.set_tooltip_text(Some(&format!(
                    "{}/{} · {}\nDiscovery does not guarantee recovery.",
                    entry.folder, entry.name, entry.modified
                )));
                list.append(&row);
                if focused == Some(id) {
                    focused_row = Some(row);
                }
            }
        }
        list.select_row(focused_row.as_ref());
        if focused_row.is_none() {
            self.state.borrow_mut().focused = None;
        }
        let selected = matching
            .iter()
            .filter(|entry| session.selected.contains(&entry.id))
            .count();
        let select_all: gtk::CheckButton = self.widget("select_all");
        select_all.set_active(!matching.is_empty() && selected == matching.len());
        select_all.set_inconsistent(selected > 0 && selected < matching.len());
        select_all.set_sensitive(!matching.is_empty());
        self.label("result_count", &format!("{}", session.found.len()));
        self.label(
            "file_summary",
            &format!(
                "{} of {} files · {} · Sample results",
                matching.len(),
                session.found.len(),
                model::size(matching.iter().map(|entry| entry.bytes).sum())
            ),
        );
        self.widget::<gtk::Box>("empty_state")
            .set_visible(matching.is_empty());
        let (title, detail) = if !query.is_empty() || filter != Filter::All {
            (
                "No matching files",
                "Try another search or choose All files.",
            )
        } else if session.phase.busy() {
            (
                "Looking for your files…",
                "Candidates will appear as the simulated scan progresses.",
            )
        } else if session.phase == Phase::Cancelled {
            (
                "Scan cancelled",
                "Start another scan to find sample candidates.",
            )
        } else {
            (
                "Ready when you are",
                "Choose a scan mode above, then start a sample scan.",
            )
        };
        self.label("empty_title", title);
        self.label("empty_detail", detail);
        self.rendering.set(false);
        self.render_selection();
        self.render_preview();
    }

    fn render_preview(&self) {
        let state = self.state.borrow();
        let entry = state
            .focused
            .and_then(|id| state.session.found.iter().find(|entry| entry.id == id));
        let picture: gtk::Picture = self.widget("preview_picture");
        picture.set_visible(entry.is_some_and(|entry| entry.kind == Kind::Image));
        if let Some(entry) = entry {
            self.label("preview_name", entry.name);
            self.label("preview_path", &format!("/{}/{}", entry.folder, entry.name));
            self.label("preview_text", entry.preview);
            self.label("preview_metadata", &format!("Size                 {}\nModified         {}\nDiscovery       {}\nIntegrity          Not verified", model::size(entry.bytes), entry.modified, entry.status()));
            self.widget::<gtk::Image>("preview_icon")
                .set_icon_name(Some(entry.icon()));
        } else {
            self.label("preview_name", "Choose a file");
            self.label("preview_path", "");
            self.label(
                "preview_text",
                "Select a candidate to inspect its contents and metadata.",
            );
            self.label("preview_metadata", "");
            self.widget::<gtk::Image>("preview_icon")
                .set_icon_name(Some("document-properties-symbolic"));
        }
    }

    fn toggle_folder(self: &Rc<Self>, folder: &str) {
        let mut state = self.state.borrow_mut();
        if !state.collapsed.remove(folder) {
            state.collapsed.insert(folder.into());
        }
        drop(state);
        self.render_files();
    }

    fn clear_filters(&self) {
        self.rendering.set(true);
        self.widget::<gtk::SearchEntry>("search").set_text("");
        self.widget::<gtk::ToggleButton>("filter_all")
            .set_active(true);
        let mut state = self.state.borrow_mut();
        state.query.clear();
        state.filter = Filter::All;
        state.focused = None;
        self.rendering.set(false);
    }

    fn select_source(self: &Rc<Self>, index: usize) {
        let mut state = self.state.borrow_mut();
        if state.session.phase.busy()
            || index >= state.sources.len()
            || index == state.session.source
        {
            return;
        }
        state.session = Session::ready(index);
        if state.sources[index].carve_only {
            state.session.mode = ScanMode::Deep;
        }
        let mode = state.session.mode;
        let name = state.sources[index].name.clone();
        drop(state);
        self.clear_filters();
        self.widget::<gtk::DropDown>("scan_mode")
            .set_selected(mode.index());
        self.widget::<gtk::CheckButton>("clone_first")
            .set_active(false);
        self.render_files();
        self.render_status();
        self.refresh_source_highlights();
        self.event(
            &format!("Selected {name}. Sample source only; no device opened."),
            false,
        );
    }

    fn start(self: &Rc<Self>) {
        let mut mode = ScanMode::from_index(self.widget::<gtk::DropDown>("scan_mode").selected());
        let clone_first = self.widget::<gtk::CheckButton>("clone_first").is_active();
        let mut state = self.state.borrow_mut();
        if state.sources[state.session.source].carve_only {
            mode = ScanMode::Deep;
        }
        if !state.session.start(mode, clone_first) {
            return;
        }
        drop(state);
        self.clear_filters();
        self.render_files();
        self.render_status();
        self.event(
            &format!(
                "{} started (simulation).{}",
                mode.name(),
                if clone_first {
                    " Simulating clone-first before scanning."
                } else {
                    " Source access disabled."
                }
            ),
            false,
        );
    }

    fn tick(self: &Rc<Self>) {
        let (changed, completed) = {
            let mut state = self.state.borrow_mut();
            if state.session.phase != Phase::Scanning {
                return;
            }
            let changed = state.session.tick();
            (changed, state.session.phase == Phase::Complete)
        };
        if changed {
            self.render_files();
        }
        self.render_status();
        if completed {
            let count = self.state.borrow().session.found.len();
            self.event(&format!("Simulation complete. {count} sample candidates found. Review and select files to continue."), false);
        }
    }

    fn pause(self: &Rc<Self>) {
        if self.state.borrow_mut().session.pause_or_resume() {
            self.render_status();
            self.event(
                if self.state.borrow().session.phase == Phase::Paused {
                    "Simulated scan paused. Progress and candidates retained."
                } else {
                    "Simulated scan resumed."
                },
                false,
            );
        }
    }

    fn cancel(self: &Rc<Self>) {
        if self.state.borrow_mut().session.cancel() {
            self.render_status();
            self.render_files();
            self.event(
                "Simulated scan cancelled. Partial candidates remain available.",
                true,
            );
        }
    }

    fn save(self: &Rc<Self>) {
        let mut state = self.state.borrow_mut();
        if state.session.phase == Phase::Ready {
            return;
        }
        let title = format!(
            "{} · #{}",
            state.sources[state.session.source].name,
            state.snapshots.len() + 1
        );
        let snapshot = Snapshot::capture(title, &state.session);
        state.snapshots.push(snapshot);
        drop(state);
        self.render_snapshots();
        self.event(
            "Snapshot saved in memory for this demo session. Nothing was written to disk.",
            false,
        );
    }

    fn open_snapshot(self: &Rc<Self>, index: usize) {
        let mut state = self.state.borrow_mut();
        if state.session.phase.busy() {
            return;
        }
        let Some(snapshot) = state.snapshots.get(index).cloned() else {
            return;
        };
        state.session = snapshot.session;
        if let Some(parent) = state.sources[state.session.source].parent {
            state.collapsed_disks.remove(&parent);
        }
        let mode = state.session.mode;
        let clone_first = state.session.clone_first;
        drop(state);
        self.clear_filters();
        self.widget::<gtk::DropDown>("scan_mode")
            .set_selected(mode.index());
        self.widget::<gtk::CheckButton>("clone_first")
            .set_active(clone_first);
        self.render_sources();
        self.render_files();
        self.render_status();
        self.event(
            "Reopened in-memory snapshot. An unfinished scan reopens paused.",
            false,
        );
    }

    fn reset(self: &Rc<Self>) {
        if self.state.borrow().session.phase.busy() {
            return;
        }
        *self.state.borrow_mut() = Workspace::default();
        self.clear_filters();
        self.state.borrow_mut().focused = Some(0);
        self.widget::<gtk::DropDown>("scan_mode").set_selected(2);
        self.widget::<gtk::CheckButton>("clone_first")
            .set_active(false);
        self.render_sources();
        self.render_snapshots();
        self.render_files();
        self.render_status();
        self.event("Demo reset to the bundled Archive SSD scan.", false);
    }

    fn event(&self, message: &str, warning: bool) {
        let builder = gtk::Builder::from_string(EVENT_ROW);
        let time = glib::DateTime::now_local()
            .and_then(|time| time.format("%H:%M:%S"))
            .map(|text| text.to_string())
            .unwrap_or_else(|_| "--:--:--".into());
        object::<gtk::Label>(&builder, "time").set_text(&time);
        object::<gtk::Label>(&builder, "message").set_text(message);
        let row: gtk::ListBoxRow = object(&builder, "row");
        row.set_tooltip_text(Some(message));
        if warning {
            row.add_css_class("warning");
            object::<gtk::Image>(&builder, "icon").set_icon_name(Some("dialog-warning-symbolic"));
        }
        let list: gtk::ListBox = self.widget("events");
        list.prepend(&row);
        let count = (self.event_count.get() + 1).min(200);
        self.event_count.set(count);
        // Even a long-running demo retains a bounded event list.
        if let Some(row) = list.row_at_index(200) {
            list.remove(&row);
        }
        self.label("event_count", &format!("{count} events"));
    }

    fn show_dialog(&self, xml: &str) -> gtk::Builder {
        if let Some(previous) = self.dialog.borrow_mut().take() {
            object::<gtk::Window>(&previous, "dialog").destroy();
        }
        let builder = gtk::Builder::from_string(xml);
        let dialog: gtk::Window = object(&builder, "dialog");
        dialog.set_transient_for(Some(&self.window()));
        dialog.set_destroy_with_parent(true);
        let escape = gtk::EventControllerKey::new();
        let weak_dialog = dialog.downgrade();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if let Some(dialog) = weak_dialog.upgrade() {
                    dialog.close();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        dialog.add_controller(escape);
        let weak = dialog.downgrade();
        object::<gtk::Button>(&builder, "close").connect_clicked(move |_| {
            if let Some(dialog) = weak.upgrade() {
                dialog.close();
            }
        });
        self.dialog.replace(Some(builder.clone()));
        dialog.present();
        builder
    }

    fn info(&self, title: &str, body: &str) {
        let builder = self.show_dialog(INFO);
        object::<gtk::Window>(&builder, "dialog").set_title(Some(title));
        object::<gtk::Label>(&builder, "title").set_text(title);
        object::<gtk::Label>(&builder, "body").set_text(body);
    }

    fn custom_scan(self: &Rc<Self>) {
        let b = self.show_dialog(include_str!("../ui/scan-options.ui"));
        object::<gtk::CheckButton>(&b, "consolidate").set_sensitive(false);
        object::<gtk::CheckButton>(&b, "consolidate").set_tooltip_text(Some(
            "Content consolidation is available for real scans; this demo uses sample data.",
        ));
        object::<gtk::DropDown>(&b, "mode")
            .set_selected(self.widget::<gtk::DropDown>("scan_mode").selected());
        object::<gtk::Entry>(&b, "workspace").set_text("/demo/recovery-workspace");
        object::<gtk::Entry>(&b, "workspace").set_sensitive(false);
        object::<gtk::Button>(&b, "browse").set_visible(false);
        object::<gtk::Label>(&b, "validation")
            .set_text("Demo settings only. No devices are opened or files written.");
        let weak = Rc::downgrade(self);
        let dialog = b.downgrade();
        object::<gtk::Button>(&b, "confirm").connect_clicked(move |_| {
            if let (Some(ui), Some(b)) = (weak.upgrade(), dialog.upgrade()) {
                if let Err(error) = crate::engine::types::normalize_extensions(
                    &object::<gtk::Entry>(&b, "extensions").text(),
                ) {
                    object::<gtk::Label>(&b, "validation").set_text(&error.to_string());
                    return;
                }
                ui.widget::<gtk::DropDown>("scan_mode")
                    .set_selected(object::<gtk::DropDown>(&b, "mode").selected());
                ui.widget::<gtk::CheckButton>("clone_first")
                    .set_active(object::<gtk::CheckButton>(&b, "clone").is_active());
                object::<gtk::Window>(&b, "dialog").close();
                ui.start();
            }
        });
    }
    fn saved_scans(self: &Rc<Self>) {
        let b = self.show_dialog(include_str!("../ui/saved-scans.ui"));
        object::<gtk::Entry>(&b, "workspace").set_text("Demo snapshots · in memory");
        object::<gtk::Entry>(&b, "workspace").set_sensitive(false);
        for id in ["browse", "refresh", "resume", "compare", "delete"] {
            object::<gtk::Button>(&b, id).set_visible(false);
        }
        let list: gtk::ListBox = object(&b, "scans");
        for snapshot in &self.state.borrow().snapshots {
            let row = gtk::Builder::from_string(SOURCE_ROW);
            object::<gtk::Label>(&row, "name").set_text(&snapshot.title);
            object::<gtk::Label>(&row, "detail").set_text(&format!(
                "{} · {} sample candidates",
                snapshot.session.mode.name(),
                snapshot.session.found.len()
            ));
            object::<gtk::Image>(&row, "icon").set_icon_name(Some("document-open-recent-symbolic"));
            list.append(&object::<gtk::ListBoxRow>(&row, "row"));
        }
        object::<gtk::Label>(&b,"details").set_text("Demo snapshots disappear when the app closes. Production scans are saved automatically to SQLite and can retain a clone.");
        let weak = Rc::downgrade(self);
        let dialog = b.downgrade();
        object::<gtk::Button>(&b, "confirm").connect_clicked(move |_| {
            if let (Some(ui), Some(b)) = (weak.upgrade(), dialog.upgrade()) {
                if let Some(row) = object::<gtk::ListBox>(&b, "scans").selected_row() {
                    let index = row.index() as usize;
                    object::<gtk::Window>(&b, "dialog").close();
                    ui.open_snapshot(index);
                } else {
                    object::<gtk::Label>(&b, "validation").set_text("Select a saved scan.");
                }
            }
        });
    }

    fn recover(self: &Rc<Self>) {
        let state = self.state.borrow();
        if state.session.selected.is_empty() || state.session.phase.busy() {
            return;
        }
        let count = state.session.selected.len();
        let bytes = state.session.selected_bytes();
        let name = state.sources[state.session.source].name.clone();
        drop(state);
        let builder = self.show_dialog(RECOVERY);
        object::<gtk::Entry>(&builder, "destination_path")
            .parent()
            .unwrap()
            .set_visible(false);
        object::<gtk::Button>(&builder, "browse").set_visible(false);
        object::<gtk::Label>(&builder, "summary").set_text(&format!("Review {count} selected files ({}) from {name}.\nThe source must stay read-only throughout recovery.", model::size(bytes)));
        let confirm: gtk::Button = object(&builder, "confirm");
        let validation: gtk::Label = object(&builder, "validation");
        let confirm_weak = confirm.downgrade();
        object::<gtk::DropDown>(&builder, "destination").connect_selected_notify(move |destination| {
            let result = model::demo_destination(destination.selected());
            validation.set_text(match result { Ok(()) => "Sample destination is on a separate drive with enough space. Real device checks are not implemented in this prototype.", Err(message) => message });
            if let Some(confirm) = confirm_weak.upgrade() { confirm.set_sensitive(result.is_ok()); }
        });
        let weak = Rc::downgrade(self);
        let destination: gtk::DropDown = object(&builder, "destination");
        let result_label: gtk::Label = object(&builder, "result");
        let close: gtk::Button = object(&builder, "close");
        confirm.connect_clicked(move |confirm| {
            if model::demo_destination(destination.selected()).is_err() { return; }
            if let Some(ui) = weak.upgrade() {
                if ui.state.borrow().session.phase.busy() { return; }
                result_label.set_text(&format!("Simulation complete: {count} files ({}) would be recovered. No files were written or hashes computed.", model::size(bytes)));
                result_label.set_visible(true);
                confirm.set_sensitive(false);
                confirm.set_label("Simulation complete");
                destination.set_sensitive(false);
                close.set_label("Done");
                ui.event(&format!("Recovery simulation reviewed: {count} files, {}. No files written.", model::size(bytes)), false);
            }
        });
    }

    fn add_source(self: &Rc<Self>) {
        if self.state.borrow().session.phase.busy() {
            return;
        }
        let builder = self.show_dialog(ADD_SOURCE);
        let name: gtk::Entry = object(&builder, "name");
        let confirm: gtk::Button = object(&builder, "confirm");
        let confirm_weak = confirm.downgrade();
        name.connect_changed(move |name| {
            if let Some(confirm) = confirm_weak.upgrade() {
                confirm.set_sensitive(!name.text().trim().is_empty());
            }
        });
        let kind: gtk::DropDown = object(&builder, "kind");
        let weak = Rc::downgrade(self);
        let dialog: gtk::Window = object(&builder, "dialog");
        dialog.set_default_widget(Some(&confirm));
        let weak_dialog = dialog.downgrade();
        confirm.connect_clicked(move |_| {
            if name.text().trim().is_empty() {
                return;
            }
            if let Some(ui) = weak.upgrade() {
                if ui.state.borrow().session.phase.busy() {
                    return;
                }
                let (detail, path, filesystem, mounted, carve_only, icon) = match kind.selected() {
                    1 => (
                        "E01 image · NTFS",
                        "/demo/added.E01",
                        "NTFS",
                        false,
                        false,
                        "drive-optical-symbolic",
                    ),
                    2 => (
                        "Mounted folder · ext4",
                        "/demo/mounted/folder",
                        "ext4",
                        true,
                        false,
                        "folder-symbolic",
                    ),
                    3 => (
                        "Unlocked mapper · btrfs",
                        "/dev/mapper/demo",
                        "btrfs",
                        true,
                        false,
                        "drive-harddisk-symbolic",
                    ),
                    4 => (
                        "Sample volume · XFS",
                        "/dev/demo-xfs",
                        "XFS",
                        false,
                        true,
                        "drive-harddisk-symbolic",
                    ),
                    5 => (
                        "Raw image · HFS+",
                        "/demo/hfs.dd",
                        "HFS+",
                        false,
                        true,
                        "drive-optical-symbolic",
                    ),
                    _ => (
                        "Raw image · ext4",
                        "/demo/added.dd",
                        "ext4",
                        false,
                        false,
                        "drive-optical-symbolic",
                    ),
                };
                let index = ui.state.borrow().sources.len();
                ui.state.borrow_mut().sources.push(Source {
                    name: name.text().trim().into(),
                    detail: detail.into(),
                    path: path.into(),
                    filesystem,
                    capacity: "119.2 GiB",
                    icon,
                    mounted,
                    carve_only,
                    kind: match kind.selected() {
                        2 => SourceKind::Folder,
                        3 => SourceKind::Mapper,
                        4 => SourceKind::Partition,
                        _ => SourceKind::Image,
                    },
                    parent: None,
                    size_mib: 122061,
                });
                ui.select_source(index);
                ui.render_sources();
                if let Some(dialog) = weak_dialog.upgrade() {
                    dialog.close();
                }
            }
        });
    }
}

fn clear_list(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

type MapSegment = (u64, bool, bool);

/// Schematic size map. Tiny partitions get a minimum width so they remain visible.
fn draw_disk_map(
    area: &gtk::DrawingArea,
    context: &gtk::cairo::Context,
    width: i32,
    height: i32,
    segments: &[MapSegment],
) {
    if segments.is_empty() || width < 1 || height < 1 {
        return;
    }
    let gap = 2.0;
    let available = (width as f64 - gap * (segments.len() - 1) as f64).max(0.0);
    let minimum = (available / segments.len() as f64).min(3.0);
    let proportional = available - minimum * segments.len() as f64;
    let total = segments
        .iter()
        .map(|segment| segment.0 as f64)
        .sum::<f64>()
        .max(1.0);
    let (foreground, accent) = crate::appearance::map_colors(area);
    let mut x = 0.0;
    for &(size, unallocated, selected) in segments {
        let segment_width = minimum + proportional * size as f64 / total;
        crate::appearance::source(
            context,
            if selected { &accent } else { &foreground },
            if selected {
                1.0
            } else if unallocated {
                0.1
            } else {
                0.35
            },
        );
        context.rectangle(x, 0.0, segment_width, height as f64);
        let _ = context.fill();
        if unallocated {
            let _ = context.save();
            context.rectangle(x, 0.0, segment_width, height as f64);
            context.clip();
            crate::appearance::source(context, &foreground, 0.45);
            context.set_line_width(1.0);
            let mut stripe = -(height as f64);
            while stripe < segment_width {
                context.move_to(x + stripe, height as f64);
                context.line_to(x + stripe + height as f64, 0.0);
                stripe += 5.0;
            }
            let _ = context.stroke();
            let _ = context.restore();
        }
        if selected {
            crate::appearance::source(context, &foreground, 1.0);
            context.set_line_width(1.5);
            context.rectangle(
                x + 0.75,
                0.75,
                (segment_width - 1.5).max(0.0),
                (height as f64 - 1.5).max(0.0),
            );
            let _ = context.stroke();
        }
        x += segment_width + gap;
    }
}

pub fn activate(app: &gtk::Application, smoke_test: bool) {
    let css = gtk::CssProvider::new();
    css.load_from_data(include_str!("../ui/layout.css"));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("GTK display"),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let ui = Rc::new(Ui {
        builder: gtk::Builder::from_string(WINDOW),
        state: RefCell::new(Workspace::default()),
        row_keys: RefCell::new(vec![]),
        source_keys: RefCell::new(vec![]),
        source_maps: RefCell::new(vec![]),
        rendering: Cell::new(false),
        event_count: Cell::new(0),
        dialog: RefCell::new(None),
    });
    ui.window().set_application(Some(app));
    ui.connect(app);
    let bytes = glib::Bytes::from_static(include_bytes!("../ui/sample-landscape.svg"));
    let stream = gio::MemoryInputStream::from_bytes(&bytes);
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_stream(&stream, gio::Cancellable::NONE)
        .expect("Bundled sample illustration");
    ui.widget::<gtk::Picture>("preview_picture")
        .set_paintable(Some(&gtk::gdk::Texture::for_pixbuf(&pixbuf)));
    ui.render_sources();
    ui.render_snapshots();
    ui.render_files();
    ui.render_status();
    ui.event(
        "Loaded a sample full scan of Archive SSD. 14 candidate files across 4 folders.",
        false,
    );
    ui.event(
        "Sources, previews, and destinations are fictional. No device access or recovery writes.",
        false,
    );
    ui.window().present();

    // The window owns the controller until destruction; signal handlers only hold weak references.
    let lifetime = Rc::new(RefCell::new(Some(ui.clone())));
    ui.window().connect_destroy(move |_| {
        lifetime.borrow_mut().take();
    });
    if smoke_test {
        let weak = Rc::downgrade(&ui);
        glib::timeout_add_local_once(Duration::from_millis(200), move || {
            if let Some(ui) = weak.upgrade() {
                smoke::run(&ui);
            }
        });
    } else {
        let weak = Rc::downgrade(&ui);
        glib::timeout_add_local(Duration::from_millis(160), move || {
            if let Some(ui) = weak.upgrade() {
                ui.tick();
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
    }
}

mod smoke {
    use super::*;

    fn action(ui: &Ui, name: &str) {
        ui.window().lookup_action(name).unwrap().activate(None);
    }

    pub fn run(ui: &Rc<Ui>) {
        let source_count = model::sources().len();
        let sources: gtk::ListBox = ui.widget("sources");
        let disk_row = sources.row_at_index(0).unwrap();
        sources.emit_by_name::<()>("row-activated", &[&disk_row]);
        assert_eq!(ui.state.borrow().session.source, 0);
        assert!(!ui.source_keys.borrow().contains(&Some(0)));
        ui.open_snapshot(0);
        assert!(ui.source_keys.borrow().contains(&Some(0)));
        let home_row = ui
            .source_keys
            .borrow()
            .iter()
            .position(|&id| id == Some(1))
            .unwrap();
        sources.select_row(sources.row_at_index(home_row as i32).as_ref());
        assert_eq!(ui.state.borrow().session.source, 1);
        assert!(
            ui.widget::<gtk::Label>("source_detail")
                .text()
                .contains("on /dev/nvme0n1")
        );
        sources.select_row(sources.row_at_index(0).as_ref());
        assert_eq!(ui.state.borrow().session.source, 6);
        assert_eq!(ui.state.borrow().sources[6].kind, SourceKind::Disk);
        ui.open_snapshot(0);
        ui.state.borrow_mut().focused = Some(0);
        ui.render_files();
        assert_eq!(ui.state.borrow().session.found.len(), 14);
        assert_eq!(
            ui.widget::<gtk::Label>("preview_name").text(),
            "Field notes.txt"
        );
        ui.widget::<gtk::ToggleButton>("filter_images")
            .set_active(true);
        assert!(
            ui.row_keys
                .borrow()
                .iter()
                .any(|key| matches!(key, RowKey::File(4)))
        );
        assert!(
            !ui.row_keys
                .borrow()
                .iter()
                .any(|key| matches!(key, RowKey::File(0)))
        );
        ui.widget::<gtk::CheckButton>("select_all").set_active(true);
        assert_eq!(ui.state.borrow().session.selected.len(), 5);
        ui.widget::<gtk::ToggleButton>("filter_all")
            .set_active(true);
        assert_eq!(ui.state.borrow().session.selected.len(), 5);
        assert!(
            ui.widget::<gtk::CheckButton>("select_all")
                .is_inconsistent()
        );
        let search: gtk::SearchEntry = ui.widget("search");
        search.set_text("no-such-file");
        search.emit_by_name::<()>("search-changed", &[]);
        assert!(ui.widget::<gtk::Box>("empty_state").is_visible());
        assert!(ui.row_keys.borrow().is_empty());
        assert_eq!(ui.state.borrow().session.selected.len(), 5);
        search.set_text("Alpine");
        search.emit_by_name::<()>("search-changed", &[]);
        let list: gtk::ListBox = ui.widget("files");
        list.select_row(list.row_at_index(1).as_ref());
        assert!(ui.widget::<gtk::Picture>("preview_picture").is_visible());
        assert_eq!(
            ui.widget::<gtk::Label>("preview_name").text(),
            "Alpine morning.jpg"
        );
        search.set_text("");
        search.emit_by_name::<()>("search-changed", &[]);
        let folder = list.row_at_index(0).unwrap();
        list.emit_by_name::<()>("row-activated", &[&folder]);
        assert!(ui.state.borrow().collapsed.contains("Documents"));
        let folder = list.row_at_index(0).unwrap();
        list.emit_by_name::<()>("row-activated", &[&folder]);
        assert!(!ui.state.borrow().collapsed.contains("Documents"));
        action(ui, "recover");
        let dialog = ui.dialog.borrow().as_ref().unwrap().clone();
        let destination: gtk::DropDown = object(&dialog, "destination");
        let confirm: gtk::Button = object(&dialog, "confirm");
        assert!(!confirm.is_sensitive());
        for index in [2, 3] {
            destination.set_selected(index);
            assert!(!confirm.is_sensitive());
        }
        destination.set_selected(1);
        assert!(confirm.is_sensitive());
        confirm.emit_clicked();
        assert!(object::<gtk::Label>(&dialog, "result").is_visible());
        assert!(!destination.is_sensitive());
        object::<gtk::Window>(&dialog, "dialog").close();

        action(ui, "start");
        assert_eq!(ui.state.borrow().session.phase, Phase::Scanning);
        assert!(!ui.widget::<gtk::ListBox>("sources").is_sensitive());
        for _ in 0..10 {
            ui.tick();
        }
        action(ui, "pause");
        let progress = ui.state.borrow().session.progress;
        ui.tick();
        assert_eq!(ui.state.borrow().session.progress, progress);
        assert_eq!(
            ui.widget::<gtk::Label>("progress_text").text(),
            format!("{progress}%")
        );
        assert!(
            ui.widget::<gtk::Label>("status_text")
                .text()
                .contains("paused")
        );
        assert_eq!(
            ui.widget::<gtk::Button>("pause").label().as_deref(),
            Some("Resume")
        );
        action(ui, "save");
        action(ui, "cancel");
        ui.open_snapshot(1);
        assert_eq!(ui.state.borrow().session.phase, Phase::Paused);
        action(ui, "pause");
        for _ in 0..50 {
            ui.tick();
        }
        assert_eq!(ui.state.borrow().session.phase, Phase::Complete);
        assert_eq!(ui.state.borrow().session.found.len(), 14);

        action(ui, "add-source");
        let dialog = ui.dialog.borrow().as_ref().unwrap().clone();
        object::<gtk::Entry>(&dialog, "name").set_text("Smoke test image");
        object::<gtk::DropDown>(&dialog, "kind").set_selected(4);
        object::<gtk::Button>(&dialog, "confirm").emit_clicked();
        assert_eq!(ui.state.borrow().sources.len(), source_count + 1);
        assert_eq!(ui.state.borrow().session.mode, ScanMode::Deep);
        assert!(!ui.widget::<gtk::DropDown>("scan_mode").is_sensitive());
        ui.widget::<gtk::CheckButton>("clone_first")
            .set_active(true);
        action(ui, "start");
        for _ in 0..10 {
            ui.tick();
        }
        assert!(ui.state.borrow().session.found.is_empty());
        for _ in 0..40 {
            ui.tick();
        }
        assert_eq!(ui.state.borrow().session.found.len(), 4);
        action(ui, "preview");
        assert!(!ui.widget::<gtk::Box>("preview_panel").is_visible());
        action(ui, "preview");
        action(ui, "events");
        assert!(!ui.widget::<gtk::Box>("events_panel").is_visible());
        action(ui, "events");
        action(ui, "reset");
        assert_eq!(ui.state.borrow().session.found.len(), 14);
        assert_eq!(ui.state.borrow().sources.len(), source_count);
        assert!(ui.state.borrow().session.selected.is_empty());
        println!(
            "GTK smoke test passed: disk/partition selection, hierarchy expansion, progress text, search, empty state, image preview, folder expansion, filters, selection, destination guards, scan controls, snapshots, source addition, clone-first, carving scope, panels, reset."
        );
        ui.window().close();
    }
}
