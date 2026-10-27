//! Production GTK controller. All blocking discovery, database and engine work runs off the UI thread.
mod partitions;
mod partitions_smoke;
use crate::{
    engine::{
        self, adapters, discovery,
        jobs::{self, Job},
        sessions::Deletion,
        sources,
        store::{self, Store},
        types::*,
    },
    model,
};
use gtk::{gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc::{self, Receiver, Sender},
    time::Duration,
};

type Task = Box<dyn FnOnce() -> Message + Send>;
enum Message {
    RecoveryValidation(u64, String, bool, engine::Result<String>),
    Findings(u64, i64, engine::Result<Vec<Candidate>>),
    Sources(engine::Result<Vec<SourceDescriptor>>),
    Added(u64, engine::Result<SourceDescriptor>),
    Page(u64, engine::Result<Page>),
    Opened(u64, PathBuf, engine::Result<(Manifest, Page)>),
    DeletePrepared(u64, engine::Result<Deletion>),
    Deleted(PathBuf, bool, engine::Result<()>),
    PartitionReport(u64, engine::Result<discovery::Report>),
    Selected(engine::Result<()>),
    Preview(u64, engine::Result<PathBuf>),
    Saved(u64, engine::Result<Vec<SavedScan>>),
    Info(String, engine::Result<String>),
}
pub struct Live {
    builder: gtk::Builder,
    dialog: RefCell<Option<gtk::Builder>>,
    dialog_generation: Cell<u64>,
    finding_cursor: Cell<i64>,
    sources: RefCell<Vec<SourceDescriptor>>,
    selected: Cell<Option<usize>>,
    source_rows: RefCell<Vec<usize>>,
    collapsed_disks: RefCell<BTreeSet<PathBuf>>,
    collapsed: RefCell<BTreeSet<String>>,
    directory: RefCell<Option<PathBuf>>,
    view_source: RefCell<Option<SourceDescriptor>>,
    options: RefCell<ScanOptions>,
    page: RefCell<Page>,
    query: RefCell<Query>,
    files: RefCell<Vec<Option<i64>>>,
    job: RefCell<Option<Job>>,
    deleting: Cell<bool>,
    partition_search: RefCell<Option<discovery::SearchJob>>,
    partition_report: RefCell<Option<discovery::Report>>,
    partition_source: RefCell<Option<SourceDescriptor>>,
    partition_generation: Cell<u64>,
    close_partition_search: Cell<bool>,
    paused: Cell<bool>,
    pulse: Cell<bool>,
    rendering: Cell<bool>,
    generation: Cell<u64>,
    view_generation: Cell<u64>,
    preview_generation: Cell<u64>,
    saved: RefCell<Vec<SavedScan>>,
    quitting: Cell<bool>,
    tasks: Sender<Task>,
    messages: Receiver<Message>,
}
fn obj<T: IsA<glib::Object>>(b: &gtk::Builder, id: &str) -> T {
    b.object(id)
        .unwrap_or_else(|| panic!("Missing GtkBuilder object {id}"))
}
fn clear(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}
fn text(b: &gtk::Builder, id: &str, value: &str) {
    obj::<gtk::Label>(b, id).set_text(value);
}
fn value(b: &gtk::Builder, id: &str) -> String {
    obj::<gtk::Entry>(b, id).text().trim().into()
}
fn checked(b: &gtk::Builder, id: &str) -> bool {
    obj::<gtk::CheckButton>(b, id).is_active()
}
fn choice(b: &gtk::Builder, id: &str) -> String {
    obj::<gtk::DropDown>(b, id)
        .selected_item()
        .and_downcast::<gtk::StringObject>()
        .map(|s| s.string().to_string())
        .unwrap_or_default()
}
fn mode(index: u32) -> Mode {
    match index {
        0 => Mode::Quick,
        1 => Mode::Deep,
        _ => Mode::Full,
    }
}
fn index(mode: Mode) -> u32 {
    match mode {
        Mode::Quick => 0,
        Mode::Deep => 1,
        Mode::Full => 2,
    }
}
fn date(seconds: i64) -> String {
    glib::DateTime::from_unix_local(seconds)
        .and_then(|d| d.format("%Y-%m-%d %H:%M"))
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "Unknown".into())
}
fn file_icon(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" => "image-x-generic-symbolic",
        "txt" | "md" | "pdf" | "csv" | "docx" => "text-x-generic-symbolic",
        _ => "application-x-generic-symbolic",
    }
}

impl Live {
    fn widget<T: IsA<glib::Object>>(&self, id: &str) -> T {
        obj(&self.builder, id)
    }
    fn window(&self) -> gtk::ApplicationWindow {
        self.widget("window")
    }
    fn label(&self, id: &str, value: &str) {
        text(&self.builder, id, value);
    }
    fn submit(&self, task: impl FnOnce() -> Message + Send + 'static) {
        let _ = self.tasks.send(Box::new(task));
    }
    fn busy(&self) -> bool {
        self.job.borrow().is_some()
            || self.deleting.get()
            || self.partition_search.borrow().is_some()
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
    fn enable(&self, name: &str, active: bool) {
        if let Some(action) = self
            .window()
            .lookup_action(name)
            .and_downcast::<gio::SimpleAction>()
        {
            action.set_enabled(active);
        }
    }
    fn connect(self: &Rc<Self>, app: &gtk::Application) {
        for (name, handler) in [
            ("start", Self::start as fn(&Rc<Self>)),
            ("custom-scan", Self::custom_scan),
            ("saved-scans", Self::saved_scans),
            ("discover-partitions", Self::find_partitions),
            ("save", Self::saved_scans),
            ("add-source", Self::add_source),
            ("reset", Self::refresh),
            ("recover", Self::recovery),
            ("pause", Self::pause),
            ("cancel", Self::cancel),
        ] {
            self.action(name, handler);
        }
        self.action("search", |ui| {
            ui.widget::<gtk::SearchEntry>("search").grab_focus();
        });
        self.action("about",|ui|ui.info("Open Data Recovery","Native Linux recovery workspace · 0.1.0\n\nRead-only source access, isolated recovery engines, persistent SQLite scans, and verified recovery to validated output storage.\n\nSource discovery does not unmount live filesystems. Recovery results depend on filesystem damage and the installed backend versions."));
        self.action("shortcuts",|ui|ui.info("Keyboard shortcuts","Ctrl+O  Add source\nCtrl+R  Start default scan\nCtrl+S  Saved scans\nCtrl+F  Search files\nCtrl+P  Toggle preview\nCtrl+L  Toggle activity\nCtrl+Q  Quit\n\nSelection controls apply to the current page. Selections persist across pages and filters."));
        self.action("backends", |ui| {
            ui.submit(|| {
                Message::Info(
                    "Backend availability".into(),
                    Ok(adapters::versions()
                        .into_iter()
                        .map(|(name, version)| format!("{name}: {version}"))
                        .collect::<Vec<_>>()
                        .join("\n")),
                )
            });
        });
        for (name, panel) in [("preview", "preview_panel"), ("events", "events_panel")] {
            let action = gio::SimpleAction::new_stateful(name, None, &true.to_variant());
            let weak = Rc::downgrade(self);
            action.connect_activate(move |action, _| {
                if let Some(ui) = weak.upgrade() {
                    let visible = !action.state().and_then(|s| s.get::<bool>()).unwrap_or(true);
                    action.set_state(&visible.to_variant());
                    ui.widget::<gtk::Box>(panel).set_visible(visible);
                }
            });
            self.window().add_action(&action);
        }
        let quit = gio::SimpleAction::new("quit", None);
        let weak = Rc::downgrade(self);
        quit.connect_activate(move |_, _| {
            if let Some(ui) = weak.upgrade() {
                ui.window().close();
            }
        });
        app.add_action(&quit);
        for (action, key) in [
            ("win.add-source", "<Primary>o"),
            ("win.start", "<Primary>r"),
            ("win.custom-scan", "<Primary><Shift>r"),
            ("win.saved-scans", "<Primary><Shift>s"),
            ("win.saved-scans", "<Primary>s"),
            ("win.search", "<Primary>f"),
            ("win.preview", "<Primary>p"),
            ("win.events", "<Primary>l"),
            ("app.quit", "<Primary>q"),
        ] {
            app.set_accels_for_action(action, &[key]);
        }
        let weak = Rc::downgrade(self);
        self.window().connect_close_request(move |_| {
            if let Some(ui) = weak.upgrade() {
                if ui.busy() {
                    ui.quitting.set(true);
                    ui.cancel();
                    ui.label(
                        "status_text",
                        if ui.deleting.get() {
                            "Finishing saved-scan deletion before closing…"
                        } else {
                            "Stopping the running job and saving partial results…"
                        },
                    );
                    return glib::Propagation::Stop;
                }
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        self.widget::<gtk::ListBox>("sources")
            .connect_row_selected(move |_, row| {
                if let (Some(ui), Some(row)) = (weak.upgrade(), row) {
                    if ui.rendering.get() || ui.busy() {
                        return;
                    }
                    let selected = ui.source_rows.borrow().get(row.index() as usize).copied();
                    ui.selected.set(selected);
                    ui.render_sources();
                    ui.controls();
                }
            });
        let weak = Rc::downgrade(self);
        self.widget::<gtk::SearchEntry>("search")
            .connect_search_changed(move |entry| {
                if let Some(ui) = weak.upgrade() {
                    ui.query.borrow_mut().text = entry.text().to_string();
                    ui.query.borrow_mut().offset = 0;
                    ui.reload();
                }
            });
        for (id, kind) in [
            ("filter_all", ""),
            ("filter_images", "image"),
            ("filter_documents", "document"),
            ("filter_other", "other"),
        ] {
            let weak = Rc::downgrade(self);
            self.widget::<gtk::ToggleButton>(id)
                .connect_toggled(move |button| {
                    if let Some(ui) = weak.upgrade() {
                        if button.is_active() {
                            ui.query.borrow_mut().kind = kind.into();
                            ui.query.borrow_mut().offset = 0;
                            ui.reload();
                        }
                    }
                });
        }
        for (id, next) in [("next_page", true), ("previous_page", false)] {
            let weak = Rc::downgrade(self);
            self.widget::<gtk::Button>(id).connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    let mut query = ui.query.borrow_mut();
                    query.offset = if next {
                        query.offset + 250
                    } else {
                        query.offset.saturating_sub(250)
                    };
                    drop(query);
                    ui.reload();
                }
            });
        }
        let weak = Rc::downgrade(self);
        self.widget::<gtk::CheckButton>("select_all")
            .connect_toggled(move |button| {
                if let Some(ui) = weak.upgrade() {
                    if !ui.rendering.get() {
                        let ids = ui.page.borrow().files.iter().map(|f| f.id).collect();
                        ui.select(ids, button.is_active());
                    }
                }
            });
        let weak = Rc::downgrade(self);
        self.widget::<gtk::ListBox>("files")
            .connect_row_selected(move |_, row| {
                if let (Some(ui), Some(row)) = (weak.upgrade(), row) {
                    if ui.rendering.get() {
                        return;
                    }
                    let id = ui
                        .files
                        .borrow()
                        .get(row.index() as usize)
                        .copied()
                        .flatten();
                    if let Some(id) = id {
                        ui.preview(id);
                    }
                }
            });
        let weak = Rc::downgrade(self);
        self.widget::<gtk::Button>("clear_events")
            .connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    clear(&ui.widget("events"));
                    ui.label("event_count", "0 events");
                }
            });
        let weak = Rc::downgrade(self);
        self.widget::<gtk::ListBox>("snapshots")
            .connect_row_activated(move |_, row| {
                if let Some(ui) = weak.upgrade() {
                    let dir = ui
                        .saved
                        .borrow()
                        .get(row.index() as usize)
                        .map(|s| s.directory.clone());
                    if let Some(dir) = dir {
                        ui.open(dir);
                    }
                }
            });
        let weak = Rc::downgrade(self);
        self.widget::<gtk::DropDown>("scan_mode")
            .connect_selected_notify(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.controls();
                }
            });
    }
    fn refresh(self: &Rc<Self>) {
        if self.busy() {
            return;
        }
        self.label("status_text", "Discovering disks and partitions…");
        self.submit(|| Message::Sources(sources::discover()));
    }
    fn controls(&self) {
        let busy = self.busy();
        let sources = self.sources.borrow();
        let source = self.selected.get().and_then(|id| sources.get(id));
        for name in ["start", "custom-scan"] {
            self.enable(name, !busy && source.is_some());
        }
        for name in [
            "add-source",
            "reset",
            "saved-scans",
            "save",
            "discover-partitions",
        ] {
            self.enable(name, !busy);
        }
        self.enable("pause", self.job.borrow().is_some());
        self.enable("cancel", self.job.borrow().is_some());
        self.enable("recover", !busy && self.page.borrow().selected > 0);
        self.widget::<gtk::ListBox>("sources").set_sensitive(!busy);
        self.widget::<gtk::ListBox>("snapshots")
            .set_sensitive(!busy);
        self.widget::<gtk::DropDown>("scan_mode")
            .set_sensitive(!busy);
        self.widget::<gtk::CheckButton>("clone_first")
            .set_sensitive(!busy);
        self.widget::<gtk::MenuButton>("scan_dropdown")
            .set_sensitive(!busy);
        self.widget::<gtk::Button>("start").set_label("Scan");
        self.widget::<gtk::Button>("pause")
            .set_label(if self.paused.get() { "Resume" } else { "Pause" });
        self.widget::<gtk::CheckButton>("select_all")
            .set_sensitive(!busy && !self.page.borrow().files.is_empty());
        if let Some(source) = source {
            let different = self
                .view_source
                .borrow()
                .as_ref()
                .is_some_and(|view| !view.same_target(source));
            self.label(
                "source_title",
                &if different {
                    format!("Next scan: {}", source.name)
                } else {
                    source.name.clone()
                },
            );
            self.label(
                "source_detail",
                &format!(
                    "{} · {} · {}{}",
                    source.path.display(),
                    if source.filesystem.is_empty() {
                        "Disk / unknown filesystem"
                    } else {
                        &source.filesystem
                    },
                    model::size(source.target_size()),
                    source
                        .partition
                        .as_ref()
                        .map(|p| format!(" · discovered range at byte {}", p.offset_bytes))
                        .unwrap_or_default()
                        + &source
                            .scope
                            .as_ref()
                            .map(|s| format!(" · folder {s}"))
                            .unwrap_or_default()
                ),
            );
            self.label(
                "source_state",
                if source.mounted {
                    "Read-only source · mounted"
                } else {
                    "Read-only source"
                },
            );
            self.label(
                "mode_hint",
                if source.mounted {
                    "Live filesystem: files can change during a scan. Clone-first is available."
                } else {
                    match mode(self.widget::<gtk::DropDown>("scan_mode").selected()) {
                        Mode::Quick => "Search filesystem metadata. Original names may survive.",
                        Mode::Deep => {
                            "Search file contents. Original names and folders may be lost."
                        }
                        Mode::Full => {
                            "Search filesystem metadata, then carve for additional files."
                        }
                    }
                },
            );
        } else {
            self.label("source_title", "Choose a source");
            self.label(
                "source_detail",
                "Add an image, or select a discovered disk or partition.",
            );
            self.label(
                "mode_hint",
                "Sources are opened read-only. Images and output folders may share a drive; devices require a separate drive.",
            );
        }
        let page = self.page.borrow();
        let result_source: gtk::Label = self.widget("results_source");
        let view_source = self.view_source.borrow();
        result_source.set_visible(view_source.is_some());
        if let Some(source) = view_source.as_ref() {
            let caption = format!("Results from {} · {}", source.name, source.path.display());
            result_source.set_text(&caption);
            result_source.set_tooltip_text(Some(&caption));
        }
        self.label(
            "selection_summary",
            &format!(
                "{} selected · {}",
                page.selected,
                model::size(page.selected_bytes)
            ),
        );
    }
    fn render_sources(self: &Rc<Self>) {
        self.rendering.set(true);
        let list: gtk::ListBox = self.widget("sources");
        clear(&list);
        self.source_rows.borrow_mut().clear();
        let sources = self.sources.borrow().clone();
        let selected = self.selected.get();
        let mut order = Vec::new();
        fn walk(
            sources: &[SourceDescriptor],
            i: usize,
            order: &mut Vec<usize>,
            collapsed: &BTreeSet<PathBuf>,
        ) {
            if order.contains(&i) {
                return;
            }
            order.push(i);
            if sources[i].partition.is_none() && !collapsed.contains(&sources[i].path) {
                for (j, s) in sources.iter().enumerate() {
                    if s.parent_path.as_ref() == Some(&sources[i].path) {
                        walk(sources, j, order, collapsed);
                    }
                }
            }
        }
        for (i, s) in sources.iter().enumerate() {
            if s.parent_path.as_ref().is_none_or(|p| {
                !sources
                    .iter()
                    .any(|s| s.partition.is_none() && &s.path == p)
            }) {
                walk(&sources, i, &mut order, &self.collapsed_disks.borrow());
            }
        }
        for i in order {
            let source = &sources[i];
            let children: Vec<_> = sources
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    source.partition.is_none() && s.parent_path.as_ref() == Some(&source.path)
                })
                .map(|(i, s)| (i, s.clone()))
                .collect();
            let disk = !children.is_empty()
                || (source.kind == SourceType::Block && source.parent_path.is_none());
            let xml = if disk {
                include_str!("../ui/disk-row.ui")
            } else if source.parent_path.is_some() {
                include_str!("../ui/partition-row.ui")
            } else {
                include_str!("../ui/source-row.ui")
            };
            let b = gtk::Builder::from_string(xml);
            text(&b, "name", &source.name);
            text(
                &b,
                "detail",
                &format!(
                    "{} · {}{}",
                    source.path.display(),
                    if source.filesystem.is_empty() {
                        "Disk"
                    } else {
                        &source.filesystem
                    },
                    source
                        .partition
                        .as_ref()
                        .map(|p| format!(" · byte {}", p.offset_bytes))
                        .unwrap_or_else(|| if source.mounted {
                            " · mounted".into()
                        } else {
                            String::new()
                        })
                ),
            );
            if let Some(label) = b.object::<gtk::Label>("capacity") {
                label.set_text(&model::size(source.target_size()));
            }
            if let Some(icon) = b.object::<gtk::Image>("icon") {
                icon.set_icon_name(Some(if source.kind == SourceType::Image {
                    "drive-optical-symbolic"
                } else {
                    "drive-harddisk-symbolic"
                }));
            }
            let row: gtk::ListBoxRow = obj(&b, "row");
            if disk {
                let parent_selected = selected
                    .is_some_and(|id| id == i || children.iter().any(|(child, _)| *child == id));
                if parent_selected {
                    row.add_css_class("contains-selected");
                }
                let expand: gtk::Button = obj(&b, "expand");
                expand.set_icon_name(if self.collapsed_disks.borrow().contains(&source.path) {
                    "pan-end-symbolic"
                } else {
                    "pan-down-symbolic"
                });
                let weak = Rc::downgrade(self);
                let path = source.path.clone();
                expand.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        if !ui.collapsed_disks.borrow_mut().remove(&path) {
                            ui.collapsed_disks.borrow_mut().insert(path.clone());
                        }
                        ui.render_sources();
                    }
                });
                let size = source.size.max(1);
                let segments = children.clone();
                obj::<gtk::DrawingArea>(&b, "layout").set_draw_func(
                    move |area, ctx, width, height| {
                        let (foreground, accent) = crate::appearance::map_colors(area);
                        crate::appearance::source(ctx, &foreground, 0.1);
                        let _ = ctx.paint();
                        for (j, s) in &segments {
                            let start = s.target_offset() as f64 / size as f64 * width as f64;
                            let width =
                                (s.target_size() as f64 / size as f64 * width as f64).max(2.0);
                            let active = selected == Some(*j);
                            if active {
                                crate::appearance::source(ctx, &accent, 1.0);
                            } else {
                                crate::appearance::source(ctx, &foreground, 0.35);
                            }
                            ctx.rectangle(start, 0.0, width - 1.0, height as f64);
                            let _ = ctx.fill();
                        }
                        if segments.is_empty() {
                            crate::appearance::source(ctx, &accent, 0.65);
                            ctx.rectangle(0.0, 0.0, width as f64, height as f64);
                            let _ = ctx.fill();
                        }
                    },
                );
            }
            list.append(&row);
            self.source_rows.borrow_mut().push(i);
            if selected == Some(i) {
                list.select_row(Some(&row));
            }
        }
        self.rendering.set(false);
    }
    fn reload(&self) {
        let Some(dir) = self.directory.borrow().clone() else {
            return;
        };
        let query = self.query.borrow().clone();
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.submit(move || {
            Message::Page(generation, Store::open(&dir).and_then(|s| s.page(&query)))
        });
    }
    fn invalidate_view(&self) {
        self.generation.set(self.generation.get() + 1);
        self.view_generation.set(self.view_generation.get() + 1);
        self.preview_generation
            .set(self.preview_generation.get() + 1);
        self.widget::<gtk::Picture>("preview_picture")
            .set_visible(false);
        self.label("preview_name", "Choose a file");
        self.label("preview_path", "");
        self.label("preview_metadata", "");
        self.label(
            "preview_text",
            "Select a candidate to inspect its contents and metadata.",
        );
    }
    fn select(&self, ids: Vec<i64>, selected: bool) {
        if self.busy() {
            return;
        }
        if let Some(dir) = self.directory.borrow().clone() {
            self.submit(move || {
                Message::Selected(Store::open(&dir).and_then(|s| s.select(&ids, selected)))
            });
        }
    }
    fn render_files(self: &Rc<Self>) {
        self.rendering.set(true);
        let list: gtk::ListBox = self.widget("files");
        clear(&list);
        self.files.borrow_mut().clear();
        let page = self.page.borrow().clone();
        let mut folders: BTreeMap<String, Vec<Candidate>> = BTreeMap::new();
        for file in &page.files {
            let folder = Path::new(&file.path)
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "/".into());
            folders.entry(folder).or_default().push(file.clone());
        }
        for (folder, files) in folders {
            let b = gtk::Builder::from_string(include_str!("../ui/folder-row.ui"));
            text(&b, "name", &folder);
            text(&b, "count", &format!("{} on this page", files.len()));
            let selected = files.iter().filter(|f| f.selected).count();
            let check: gtk::CheckButton = obj(&b, "check");
            check.set_active(selected == files.len());
            check.set_inconsistent(selected > 0 && selected < files.len());
            check.set_sensitive(!self.busy());
            let ids = files.iter().map(|f| f.id).collect::<Vec<_>>();
            let weak = Rc::downgrade(self);
            check.connect_toggled(move |button| {
                if let Some(ui) = weak.upgrade() {
                    if !ui.rendering.get() {
                        ui.select(ids.clone(), button.is_active());
                    }
                }
            });
            let collapsed = self.collapsed.borrow().contains(&folder);
            let expand: gtk::Button = obj(&b, "expand");
            expand.set_icon_name(if collapsed {
                "pan-end-symbolic"
            } else {
                "pan-down-symbolic"
            });
            let weak = Rc::downgrade(self);
            let key = folder.clone();
            expand.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    if !ui.collapsed.borrow_mut().remove(&key) {
                        ui.collapsed.borrow_mut().insert(key.clone());
                    }
                    ui.render_files();
                }
            });
            list.append(&obj::<gtk::ListBoxRow>(&b, "row"));
            self.files.borrow_mut().push(None);
            if collapsed {
                continue;
            }
            for file in files {
                let b = gtk::Builder::from_string(include_str!("../ui/file-row.ui"));
                text(
                    &b,
                    "name",
                    Path::new(&file.path)
                        .file_name()
                        .unwrap_or_default()
                        .to_str()
                        .unwrap_or(&file.path),
                );
                text(&b, "size", &model::size(file.bytes));
                text(
                    &b,
                    "status",
                    if file.carved {
                        "Carved"
                    } else if file.deleted {
                        "Deleted"
                    } else {
                        "Existing"
                    },
                );
                obj::<gtk::Image>(&b, "icon").set_icon_name(Some(file_icon(&file.path)));
                let check: gtk::CheckButton = obj(&b, "check");
                check.set_active(file.selected);
                check.set_sensitive(!self.busy());
                let id = file.id;
                let weak = Rc::downgrade(self);
                check.connect_toggled(move |button| {
                    if let Some(ui) = weak.upgrade() {
                        if !ui.rendering.get() {
                            ui.select(vec![id], button.is_active());
                        }
                    }
                });
                let row: gtk::ListBoxRow = obj(&b, "row");
                let findings: gtk::Button = obj(&b, "findings");
                findings.set_visible(
                    page.consolidated && (file.findings > 1 || file.consolidation_error.is_some()),
                );
                findings.set_label(&if file.consolidation_error.is_some() {
                    "Not verified".into()
                } else {
                    format!("{} findings…", file.findings)
                });
                let weak = Rc::downgrade(self);
                let group = file.clone();
                findings.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        ui.show_findings(group.clone());
                    }
                });
                if page.consolidated {
                    check.set_tooltip_text(Some("Select this group; recover one copy by default"));
                }
                row.set_tooltip_text(Some(&format!(
                    "{}\n{} · {}{}",
                    file.path,
                    file.backend,
                    file.backend_version,
                    if page.consolidated {
                        format!(
                            "\n{} findings · {} recorded paths{}",
                            file.findings,
                            file.paths,
                            file.consolidation_error
                                .as_ref()
                                .map(|e| format!("\nNot verified: {e}"))
                                .unwrap_or_default()
                        )
                    } else {
                        String::new()
                    }
                )));
                list.append(&row);
                self.files.borrow_mut().push(Some(file.id));
            }
        }
        let selected = page.files.iter().filter(|f| f.selected).count();
        let all: gtk::CheckButton = self.widget("select_all");
        all.set_active(!page.files.is_empty() && selected == page.files.len());
        all.set_inconsistent(selected > 0 && selected < page.files.len());
        all.set_tooltip_text(Some(
            "Select matching files on this page; selection persists across pages",
        ));
        self.label("result_count", &page.total.to_string());
        self.label(
            "file_summary",
            &if page.consolidated {
                format!(
                    "{} matching groups · {} groups from {} findings · Saved automatically",
                    page.matched, page.total, page.findings
                )
            } else {
                format!(
                    "{} matching · {} total · Results and selections are saved automatically",
                    page.matched, page.total
                )
            },
        );
        let offset = self.query.borrow().offset;
        self.widget::<gtk::Box>("pagination")
            .set_visible(page.matched > 250);
        self.label(
            "page_info",
            &format!(
                "{}–{} of {}",
                if page.matched == 0 { 0 } else { offset + 1 },
                offset + page.files.len(),
                page.matched
            ),
        );
        self.widget::<gtk::Button>("previous_page")
            .set_sensitive(offset > 0);
        self.widget::<gtk::Button>("next_page")
            .set_sensitive((offset + 250) < page.matched as usize);
        self.widget::<gtk::Box>("empty_state")
            .set_visible(page.files.is_empty());
        self.label(
            "empty_title",
            if self.busy() {
                "Looking for your files…"
            } else {
                "No candidates to display"
            },
        );
        self.label(
            "empty_detail",
            "Choose a source and start a scan, open a saved scan, or adjust your filters.",
        );
        self.rendering.set(false);
        self.controls();
    }
    fn preview(&self, id: i64) {
        let Some(file) = self
            .page
            .borrow()
            .files
            .iter()
            .find(|f| f.id == id)
            .cloned()
        else {
            return;
        };
        let Some(dir) = self.directory.borrow().clone() else {
            return;
        };
        self.widget::<gtk::Picture>("preview_picture")
            .set_visible(false);
        self.label(
            "preview_name",
            Path::new(&file.path)
                .file_name()
                .unwrap_or_default()
                .to_str()
                .unwrap_or(&file.path),
        );
        self.label("preview_path", &file.path);
        self.label("preview_text", "Loading preview…");
        self.label("preview_metadata",&format!("Size                 {}\nModified         {}\nEngine             {}\nAddress           {}\nByte range       {} + {}\nFindings           {} · {} recorded paths\nContent            {}\nRecovery integrity  Not verified",model::size(file.bytes),date(file.modified),file.backend,file.locator,file.offset_bytes,file.length_bytes,file.findings,file.paths,
            if file.sha256.is_some() { "Verified for consolidation" } else { "Not compared" }));
        self.widget::<gtk::Image>("preview_icon")
            .set_icon_name(Some(file_icon(&file.path)));
        let generation = self.preview_generation.get() + 1;
        self.preview_generation.set(generation);
        self.submit(move || Message::Preview(generation, jobs::preview(&dir, id)));
    }
    fn show_findings(self: &Rc<Self>, group: Candidate) {
        let b = self.show(include_str!("../ui/findings.ui"));
        text(&b,"summary",&format!("{}\n{} findings · {} recorded paths\n{}",group.path,group.findings,group.paths,
            group.consolidation_error.as_ref().map(|e|format!("Kept separate: {e}")).unwrap_or_else(||
                "Contents were verified as identical. Recovery saves one copy by default; enable Recover every recorded path to restore all names.".into())));
        for (button, first) in [("first", true), ("next", false)] {
            let weak = Rc::downgrade(self);
            let id = group.id;
            obj::<gtk::Button>(&b, button).connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.load_findings(id, if first { 0 } else { ui.finding_cursor.get() });
                }
            });
        }
        self.load_findings(group.id, 0);
    }
    fn load_findings(&self, id: i64, after: i64) {
        let Some(directory) = self.directory.borrow().clone() else {
            return;
        };
        if let Some(b) = self.dialog.borrow().as_ref() {
            obj::<gtk::TextView>(b, "body")
                .buffer()
                .set_text("Loading findings…");
            obj::<gtk::Button>(b, "next").set_sensitive(false);
            obj::<gtk::Button>(b, "first").set_sensitive(false);
        }
        let generation = self.dialog_generation.get();
        self.submit(move || {
            Message::Findings(
                generation,
                after,
                Store::open(&directory).and_then(|s| s.findings(id, after)),
            )
        });
    }
    fn event(&self, message: &str) {
        let b = gtk::Builder::from_string(include_str!("../ui/event-row.ui"));
        text(
            &b,
            "time",
            &glib::DateTime::now_local()
                .and_then(|d| d.format("%H:%M:%S"))
                .map(|s| s.to_string())
                .unwrap_or_default(),
        );
        text(&b, "message", message);
        let row: gtk::ListBoxRow = obj(&b, "row");
        row.set_tooltip_text(Some(message));
        let list: gtk::ListBox = self.widget("events");
        list.prepend(&row);
        if let Some(row) = list.row_at_index(200) {
            list.remove(&row);
        }
        self.label("event_count", "Recent events");
    }
    fn show(&self, xml: &str) -> gtk::Builder {
        if let Some(old) = self.dialog.borrow_mut().take() {
            obj::<gtk::Window>(&old, "dialog").destroy();
        }
        self.dialog_generation.set(self.dialog_generation.get() + 1);
        let b = gtk::Builder::from_string(xml);
        let window: gtk::Window = obj(&b, "dialog");
        window.set_transient_for(Some(&self.window()));
        window.set_destroy_with_parent(true);
        let escape = gtk::EventControllerKey::new();
        let weak_dialog = window.downgrade();
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
        window.add_controller(escape);
        let weak = window.downgrade();
        obj::<gtk::Button>(&b, "close").connect_clicked(move |_| {
            if let Some(w) = weak.upgrade() {
                w.close();
            }
        });
        self.dialog.replace(Some(b.clone()));
        window.present();
        b
    }
    fn info(&self, title: &str, body: &str) {
        let b = self.show(include_str!("../ui/info.ui"));
        obj::<gtk::Window>(&b, "dialog").set_title(Some(title));
        text(&b, "title", title);
        text(&b, "body", body);
    }
    fn browse(b: &gtk::Builder, button: &str, entry: &str, chooser: &str) {
        let chooser: gtk::FileChooserNative = obj(b, chooser);
        chooser.set_transient_for(Some(&obj::<gtk::Window>(b, "dialog")));
        let target: gtk::Entry = obj(b, entry);
        let weak = target.downgrade();
        chooser.connect_response(move |chooser, response| {
            if response == gtk::ResponseType::Accept {
                if let (Some(target), Some(path)) =
                    (weak.upgrade(), chooser.file().and_then(|f| f.path()))
                {
                    target.set_text(&path.to_string_lossy());
                }
            }
            chooser.hide();
        });
        let weak = chooser.downgrade();
        obj::<gtk::Button>(b, button).connect_clicked(move |_| {
            if let Some(chooser) = weak.upgrade() {
                chooser.show();
            }
        });
    }
    fn add_source(self: &Rc<Self>) {
        let b = self.show(include_str!("../ui/source.ui"));
        Self::browse(&b, "browse", "path", "chooser");
        Self::browse(&b, "folder", "path", "folder_chooser");
        let weak = Rc::downgrade(self);
        let dialog = b.downgrade();
        obj::<gtk::Button>(&b, "confirm").connect_clicked(move |button| {
            let Some(dialog) = dialog.upgrade() else {
                return;
            };
            if let Some(ui) = weak.upgrade() {
                let path = PathBuf::from(value(&dialog, "path"));
                if !path.is_absolute() {
                    text(&dialog, "validation", "Enter an absolute source path.");
                    return;
                }
                button.set_sensitive(false);
                text(&dialog, "validation", "Inspecting source…");
                let generation = ui.dialog_generation.get();
                ui.submit(move || Message::Added(generation, sources::inspect(&path)));
            }
        });
    }
    fn start(self: &Rc<Self>) {
        let options = ScanOptions {
            mode: mode(self.widget::<gtk::DropDown>("scan_mode").selected()),
            clone_first: self.widget::<gtk::CheckButton>("clone_first").is_active(),
            workspace: self.options.borrow().workspace.clone(),
            ..Default::default()
        };
        self.begin(options);
    }
    fn begin(self: &Rc<Self>, options: ScanOptions) {
        if self.busy() {
            return;
        }
        let Some(source) = self
            .selected
            .get()
            .and_then(|i| self.sources.borrow().get(i).cloned())
        else {
            return;
        };
        self.options.replace(options.clone());
        self.view_source.replace(Some(source.clone()));
        self.invalidate_view();
        self.directory.replace(None);
        self.page.replace(Page::default());
        self.query.borrow_mut().offset = 0;
        self.paused.set(false);
        self.pulse.set(true);
        self.label("progress_text", "");
        self.label("status_text", "Preparing scan…");
        self.event(&format!(
            "Requested {:?} scan of {}",
            options.mode,
            source.path.display()
        ));
        self.job.replace(Some(jobs::start_scan(source, options)));
        self.render_files();
        self.controls();
    }
    fn custom_scan(self: &Rc<Self>) {
        let b = self.show(include_str!("../ui/scan-options.ui"));
        let options = self.options.borrow().clone();
        obj::<gtk::DropDown>(&b, "mode")
            .set_selected(self.widget::<gtk::DropDown>("scan_mode").selected());
        obj::<gtk::Entry>(&b, "workspace").set_text(&options.workspace.to_string_lossy());
        obj::<gtk::CheckButton>(&b, "clone")
            .set_active(self.widget::<gtk::CheckButton>("clone_first").is_active());
        obj::<gtk::CheckButton>(&b, "consolidate").set_active(options.consolidate);
        if let Some(partition) = self
            .selected
            .get()
            .and_then(|i| self.sources.borrow().get(i)?.partition.clone())
        {
            for (id, value) in [
                ("offset", partition.offset_bytes.to_string()),
                ("length", partition.length_bytes.to_string()),
            ] {
                let entry: gtk::Entry = obj(&b, id);
                entry.set_text(&value);
                entry.set_sensitive(false);
                entry.set_tooltip_text(Some("The discovered partition defines this range. It is retained in the workspace before scanning."));
            }
        }
        Self::browse(&b, "browse", "workspace", "chooser");
        let weak = Rc::downgrade(self);
        let dialog = b.downgrade();
        obj::<gtk::Button>(&b, "confirm").connect_clicked(move |_| {
            let Some(dialog) = dialog.upgrade() else {
                return;
            };
            if let Some(ui) = weak.upgrade() {
                match scan_options(&dialog) {
                    Ok(options) => {
                        obj::<gtk::Window>(&dialog, "dialog").close();
                        ui.widget::<gtk::DropDown>("scan_mode")
                            .set_selected(index(options.mode));
                        ui.begin(options);
                    }
                    Err(error) => text(&dialog, "validation", &error.to_string()),
                }
            }
        });
    }
    fn pause(self: &Rc<Self>) {
        if let Some(job) = self.job.borrow().as_ref() {
            self.paused.set(!self.paused.get());
            job.control.pause(self.paused.get());
            self.label(
                "status_text",
                if self.paused.get() {
                    "Paused · source and partial results retained"
                } else {
                    "Resuming…"
                },
            );
            if self.paused.get() {
                self.widget::<gtk::ProgressBar>("progress")
                    .add_css_class("paused");
            } else {
                self.widget::<gtk::ProgressBar>("progress")
                    .remove_css_class("paused");
            }
        }
        self.controls();
    }
    fn cancel(self: &Rc<Self>) {
        if let Some(search) = self.partition_search.borrow().as_ref() {
            search.control.cancel();
        }
        if let Some(job) = self.job.borrow().as_ref() {
            job.control.cancel();
            self.label("status_text", "Cancelling · saving partial results…");
        }
    }
    fn recovery(self: &Rc<Self>) {
        let Some(dir) = self.directory.borrow().clone() else {
            return;
        };
        let b = self.show(include_str!("../ui/recovery.ui"));
        obj::<gtk::DropDown>(&b, "destination").set_visible(false);
        obj::<gtk::Window>(&b, "dialog").set_default_size(590, 680);
        let page = self.page.borrow().clone();
        obj::<gtk::CheckButton>(&b, "all_paths").set_visible(page.consolidated);
        let recovery_source = self
            .view_source
            .borrow()
            .as_ref()
            .map(|source| format!("From: {} · {}\n", source.name, source.path.display()))
            .unwrap_or_default();
        text(
            &b,
            "summary",
            &format!(
                "{recovery_source}{} selected {} · {}\n{}Each output path, error, and SHA-256 is recorded with this scan.",
                page.selected,
                if page.consolidated { "groups" } else { "files" },
                model::size(page.selected_bytes),
                if page.consolidated {
                    "One copy per group. "
                } else {
                    ""
                }
            ),
        );
        let dialog = b.downgrade();
        obj::<gtk::CheckButton>(&b,"all_paths").connect_toggled(move |button| {
            if let Some(b)=dialog.upgrade() {
                text(&b,"summary",&if button.is_active() { format!("{recovery_source}{} recorded paths · {}\nEach distinct recorded path will receive a copy.",page.selected_paths,model::size(page.selected_path_bytes)) }
                    else { format!("{recovery_source}{} selected groups · {}\nOne copy per group; all findings remain saved.",page.selected,model::size(page.selected_bytes)) });
                obj::<gtk::Entry>(&b,"destination_path").emit_by_name::<()>("changed",&[]);
            }
        });
        text(
            &b,
            "notice",
            if self
                .view_source
                .borrow()
                .as_ref()
                .is_some_and(SourceDescriptor::is_image_file)
            {
                "The destination may share the image's drive, but must be outside the saved scan folder and must not contain the source image or its segments."
            } else {
                "Keep the destination outside the saved scan folder and on a different physical drive from the original source."
            },
        );
        Self::browse(&b, "browse", "destination_path", "chooser");
        obj::<gtk::Button>(&b, "confirm").set_label("Recover files");
        obj::<gtk::Button>(&b, "confirm").set_sensitive(false);
        let weak = Rc::downgrade(self);
        let dialog = b.downgrade();
        let session = dir.clone();
        obj::<gtk::Entry>(&b, "destination_path").connect_changed(move |entry| {
            let entered = entry.text().trim().to_string();
            let (Some(ui), Some(b)) = (weak.upgrade(), dialog.upgrade()) else {
                return;
            };
            obj::<gtk::Button>(&b, "confirm").set_sensitive(false);
            text(
                &b,
                "validation",
                "Checking destination identity and free space…",
            );
            let generation = ui.dialog_generation.get();
            let all_paths = checked(&b, "all_paths");
            let weak = Rc::downgrade(&ui);
            let session = session.clone();
            let dialog = b.downgrade();
            glib::timeout_add_local_once(Duration::from_millis(350), move || {
                let (Some(ui), Some(b)) = (weak.upgrade(), dialog.upgrade()) else {
                    return;
                };
                if generation != ui.dialog_generation.get()
                    || value(&b, "destination_path") != entered
                    || checked(&b, "all_paths") != all_paths
                {
                    return;
                }
                ui.submit(move || {
                    let result = Store::open(&session).and_then(|store| {
                        let options = RecoveryOptions {
                            destination: PathBuf::from(&entered),
                            restore_all_paths: all_paths,
                            ..Default::default()
                        };
                        jobs::validate_recovery(&store, &options)?;
                        let existing = engine::safety::existing_parent(&options.destination)?;
                        let free = engine::safety::free_bytes(&existing)?;
                        Ok(format!(
                            "Destination verified · {} available",
                            model::size(free)
                        ))
                    });
                    Message::RecoveryValidation(generation, entered, all_paths, result)
                });
            });
        });
        let weak = Rc::downgrade(self);
        let dialog = b.downgrade();
        obj::<gtk::Button>(&b, "confirm").connect_clicked(move |_| {
            let Some(dialog) = dialog.upgrade() else {
                return;
            };
            if let Some(ui) = weak.upgrade() {
                let options = RecoveryOptions {
                    destination: PathBuf::from(value(&dialog, "destination_path")),
                    preserve_tree: checked(&dialog, "preserve_tree"),
                    conflict: if obj::<gtk::DropDown>(&dialog, "conflict").selected() == 0 {
                        Conflict::Rename
                    } else {
                        Conflict::Skip
                    },
                    preserve_timestamps: checked(&dialog, "timestamps"),
                    verify_after_write: checked(&dialog, "verify"),
                    continue_on_error: checked(&dialog, "continue_errors"),
                    restore_all_paths: checked(&dialog, "all_paths"),
                };
                if !options.destination.is_absolute() {
                    text(
                        &dialog,
                        "validation",
                        "Choose an absolute destination directory path.",
                    );
                    return;
                }
                obj::<gtk::Window>(&dialog, "dialog").close();
                ui.job
                    .replace(Some(jobs::start_recovery(dir.clone(), options)));
                ui.pulse.set(true);
                ui.label("progress_text", "");
                ui.label("status_text", "Validating recovery destination…");
                ui.render_files();
            }
        });
    }
    fn saved_scans(self: &Rc<Self>) {
        if self.busy() {
            return;
        }
        let b = self.show(include_str!("../ui/saved-scans.ui"));
        obj::<gtk::Entry>(&b, "workspace")
            .set_text(&self.options.borrow().workspace.to_string_lossy());
        Self::browse(&b, "browse", "workspace", "chooser");
        let weak = Rc::downgrade(self);
        let dialog = b.downgrade();
        obj::<gtk::Button>(&b, "refresh").connect_clicked(move |_| {
            let Some(dialog) = dialog.upgrade() else {
                return;
            };
            if let Some(ui) = weak.upgrade() {
                ui.load_saved(PathBuf::from(value(&dialog, "workspace")));
            }
        });
        let weak = Rc::downgrade(self);
        let dialog = b.downgrade();
        obj::<gtk::ListBox>(&b, "scans").connect_row_selected(move |_, row| {
            let Some(dialog) = dialog.upgrade() else {
                return;
            };
            obj::<gtk::Button>(&dialog, "delete").set_sensitive(row.is_some());
            if let (Some(ui), Some(row)) = (weak.upgrade(), row) {
                if let Some(scan) = ui.saved.borrow().get(row.index() as usize) {
                    text(
                        &dialog,
                        "details",
                        &format!(
                            "{}\n{} · {:?} · {} candidates\nSource: {}\nSaved: {}\nEngines: {}{}",
                            scan.directory.display(),
                            scan.manifest.state,
                            scan.manifest.options.mode,
                            scan.manifest.candidate_count,
                            scan.manifest.source.path.display(),
                            date(scan.manifest.updated_at as i64),
                            scan.manifest
                                .engines
                                .iter()
                                .map(|(name, version)| format!("{name} ({version})"))
                                .collect::<Vec<_>>()
                                .join(", "),
                            if scan.manifest.clone.is_some() {
                                "\nRetained clone: original source is not required for recovery."
                            } else {
                                ""
                            }
                        ),
                    );
                }
            }
        });
        let weak = Rc::downgrade(self);
        let dialog = b.downgrade();
        obj::<gtk::Button>(&b, "delete").connect_clicked(move |button| {
            let (Some(ui), Some(dialog)) = (weak.upgrade(), dialog.upgrade()) else {
                return;
            };
            if ui.busy() {
                return;
            }
            let Some(row) = obj::<gtk::ListBox>(&dialog, "scans").selected_row() else {
                return;
            };
            let Some(scan) = ui.saved.borrow().get(row.index() as usize).cloned() else {
                return;
            };
            button.set_sensitive(false);
            text(&dialog, "validation", "Checking the selected saved scan…");
            let generation = ui.dialog_generation.get();
            ui.submit(move || Message::DeletePrepared(generation, Deletion::prepare(scan)));
        });
        for (id, operation) in [("confirm", 0), ("resume", 1), ("compare", 2)] {
            let weak = Rc::downgrade(self);
            let dialog = b.downgrade();
            obj::<gtk::Button>(&b,id).connect_clicked(move |_|{let Some(dialog)=dialog.upgrade()else{return;};if let Some(ui)=weak.upgrade(){let Some(row)=obj::<gtk::ListBox>(&dialog,"scans").selected_row()else{text(&dialog,"validation","Select a saved scan first.");return;};let Some(scan)=ui.saved.borrow().get(row.index()as usize).cloned()else{return;};match operation{0=>{obj::<gtk::Window>(&dialog,"dialog").close();ui.open(scan.directory);},1=>{obj::<gtk::Window>(&dialog,"dialog").close();ui.restart(scan);},_=>{let Some(current)=ui.directory.borrow().clone()else{text(&dialog,"validation","Open a scan in the workspace first, then select another scan to compare.");return;};ui.submit(move||Message::Info("Saved scan comparison".into(),Store::open(&current).and_then(|a|Store::open(&scan.directory).and_then(|b|store::compare_recorded(&a,&b)))));}}}});
        }
        let workspace = self.options.borrow().workspace.clone();
        self.load_saved(workspace);
    }
    fn load_saved(&self, root: PathBuf) {
        let generation = self.dialog_generation.get();
        self.options.borrow_mut().workspace = root.clone();
        self.submit(move || Message::Saved(generation, store::list(&root)));
    }
    fn open(&self, dir: PathBuf) {
        if self.busy() {
            return;
        }
        self.invalidate_view();
        let generation = self.view_generation.get();
        self.submit(move || {
            let result = Store::open(&dir).and_then(|s| {
                let page = s.page(&Query {
                    limit: 250,
                    ..Default::default()
                })?;
                Ok((s.manifest, page))
            });
            Message::Opened(generation, dir, result)
        });
    }
    fn confirm_delete(self: &Rc<Self>, plan: Deletion) {
        let b = self.show(include_str!("../ui/delete-scan.ui"));
        text(
            &b,
            "details",
            &format!(
                "{}\nSaved: {}\n\n{}",
                plan.scan.manifest.source.name,
                date(plan.scan.manifest.updated_at as i64),
                plan.scan.directory.display()
            ),
        );
        let weak = Rc::downgrade(self);
        obj::<gtk::Window>(&b, "dialog").connect_close_request(move |_| {
            if let Some(ui) = weak.upgrade() {
                if ui.deleting.get() {
                    return glib::Propagation::Stop;
                }
                let weak = Rc::downgrade(&ui);
                glib::idle_add_local_once(move || {
                    if let Some(ui) = weak.upgrade() {
                        ui.saved_scans();
                    }
                });
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        let dialog = b.downgrade();
        obj::<gtk::Button>(&b, "confirm").connect_clicked(move |button| {
            let (Some(ui), Some(dialog)) = (weak.upgrade(), dialog.upgrade()) else {
                return;
            };
            if ui.busy() {
                return;
            }
            ui.deleting.set(true);
            button.set_sensitive(false);
            obj::<gtk::Button>(&dialog, "close").set_sensitive(false);
            text(&dialog, "validation", "Deleting the saved-scan folder…");
            let plan = plan.clone();
            ui.submit(move || {
                let result = plan.execute();
                Message::Deleted(
                    plan.scan.directory.clone(),
                    !plan.scan.directory.exists(),
                    result,
                )
            });
            ui.controls();
        });
        obj::<gtk::Button>(&b, "close").grab_focus();
    }
    fn restart(self: &Rc<Self>, scan: SavedScan) {
        if self.busy() {
            return;
        }
        self.view_source.replace(Some(scan.manifest.source.clone()));
        self.options.replace(scan.manifest.options.clone());
        self.invalidate_view();
        self.directory.replace(None);
        self.page.replace(Page::default());
        self.job.replace(Some(jobs::restart(scan.directory)));
        self.pulse.set(true);
        self.label("progress_text", "");
        self.label("status_text", "Starting a new attempt from the saved scan…");
        self.render_files();
    }
    fn tick(self: &Rc<Self>) {
        self.poll_partition_search();
        let events: Vec<_> = self
            .job
            .borrow()
            .as_ref()
            .map(|job| job.events.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            match event {
                Event::Message(message) => self.event(&message),
                Event::Session(dir) => {
                    self.directory.replace(Some(dir));
                }
                Event::Candidates(count) => {
                    self.label("result_count", &count.to_string());
                    self.reload();
                }
                Event::Stage { text, fraction } => {
                    self.label("status_text", &text);
                    self.pulse.set(fraction.is_none());
                    if let Some(fraction) = fraction {
                        self.widget::<gtk::ProgressBar>("progress")
                            .set_fraction(fraction.clamp(0.0, 1.0));
                        self.label("progress_text", &format!("{:.0}%", fraction * 100.0));
                    } else {
                        self.label("progress_text", "");
                    }
                }
                Event::Finished {
                    directory,
                    cancelled,
                    error,
                } => {
                    self.job.replace(None);
                    self.pulse.set(false);
                    self.paused.set(false);
                    self.widget::<gtk::ProgressBar>("progress")
                        .remove_css_class("paused");
                    let status = if cancelled {
                        "Cancelled · partial results retained"
                    } else if error.is_some() {
                        "Stopped with errors · inspect Activity"
                    } else {
                        "Scan complete · results saved"
                    };
                    self.label("status_text", status);
                    self.event(status);
                    if let Some(error) = error {
                        self.event(&error);
                        self.info(
                            if cancelled {
                                "Job cancelled"
                            } else {
                                "Scan / recovery details"
                            },
                            &error,
                        );
                    } else if !cancelled {
                        self.widget::<gtk::ProgressBar>("progress")
                            .set_fraction(1.0);
                        self.label("progress_text", "100%");
                    }
                    if !directory.as_os_str().is_empty() {
                        self.directory.replace(Some(directory));
                        self.reload();
                    }
                    self.controls();
                    if self.quitting.get() {
                        self.window().close();
                    }
                }
                Event::Recovered {
                    successful,
                    failed,
                    skipped,
                } => {
                    self.job.replace(None);
                    self.pulse.set(false);
                    self.label("progress_text", "100%");
                    let summary = format!(
                        "Recovery finished · {successful} saved · {failed} failed · {skipped} skipped"
                    );
                    self.label("status_text", &summary);
                    self.event(&summary);
                    self.info("Recovery results",&format!("{summary}\n\nOutput paths, hashes, verification results, and individual errors are in this scan's events.log."));
                    self.controls();
                    if self.quitting.get() {
                        self.window().close();
                    }
                }
            }
        }
        while let Ok(message) = self.messages.try_recv() {
            match message {
                Message::PartitionReport(generation, result) => {
                    if generation == self.dialog_generation.get() && !self.busy() {
                        match result {
                            Ok(report) => self.show_partition_report(report),
                            Err(error) => {
                                if let Some(b) = self.dialog.borrow().as_ref() {
                                    text(b, "validation", &error.to_string());
                                }
                            }
                        }
                    }
                }
                Message::Findings(generation, after, result) => {
                    if generation == self.dialog_generation.get() {
                        if let Some(b) = self.dialog.borrow().as_ref() {
                            let mut next = false;
                            let body = match result {
                                Ok(files) => {
                                    self.finding_cursor
                                        .set(files.last().map_or(after, |f| f.id));
                                    next = files.len() == 100;
                                    if files.is_empty() {
                                        "No more findings.".into()
                                    } else {
                                        files.iter().map(|f|format!("{}\n{} · {} · {}\nAddress: {} · Range: {} + {}\n",f.path,f.backend,f.backend_version,model::size(f.bytes),f.locator,f.offset_bytes,f.length_bytes)).collect::<Vec<_>>().join("\n")
                                    }
                                }
                                Err(error) => error.to_string(),
                            };
                            obj::<gtk::TextView>(b, "body").buffer().set_text(&body);
                            obj::<gtk::Button>(b, "next").set_sensitive(next);
                            obj::<gtk::Button>(b, "first").set_sensitive(after > 0);
                        }
                    }
                }
                Message::RecoveryValidation(generation, path, all_paths, result) => {
                    if generation == self.dialog_generation.get() {
                        if let Some(b) = self.dialog.borrow().as_ref() {
                            if b.object::<gtk::Entry>("destination_path")
                                .is_some_and(|entry| entry.text().trim() == path)
                                && checked(b, "all_paths") == all_paths
                            {
                                obj::<gtk::Button>(b, "confirm").set_sensitive(result.is_ok());
                                text(
                                    b,
                                    "validation",
                                    &result.unwrap_or_else(|error| error.to_string()),
                                );
                            }
                        }
                    }
                }
                Message::Sources(result) => match result {
                    Ok(sources) => {
                        let previous = self
                            .selected
                            .get()
                            .and_then(|id| self.sources.borrow().get(id).cloned());
                        let images = self
                            .sources
                            .borrow()
                            .iter()
                            .filter(|s| s.kind != SourceType::Block || s.partition.is_some())
                            .cloned()
                            .collect::<Vec<_>>();
                        self.sources.replace(sources);
                        self.sources.borrow_mut().extend(images);
                        let selected = previous.as_ref().and_then(|old| {
                            self.sources
                                .borrow()
                                .iter()
                                .position(|s| s.same_target(old))
                        });
                        if selected.is_none() && self.directory.borrow().is_some() {
                            if let Some(old) = previous {
                                let id = self.sources.borrow().len();
                                self.sources.borrow_mut().push(old);
                                self.selected.set(Some(id));
                            }
                        } else {
                            self.selected.set(
                                selected
                                    .or_else(|| (!self.sources.borrow().is_empty()).then_some(0)),
                            );
                        }
                        self.render_sources();
                        self.controls();
                        self.label(
                            "status_text",
                            if self.directory.borrow().is_some() {
                                "Sources refreshed · current scan kept open"
                            } else {
                                "Ready · choose a source and scan mode"
                            },
                        );
                    }
                    Err(e) => self.info("Source discovery", &e.to_string()),
                },
                Message::Added(generation, result) => match result {
                    Ok(source) => {
                        let index = self.sources.borrow().len();
                        self.sources.borrow_mut().push(source);
                        if generation != self.dialog_generation.get() || self.busy() {
                            self.render_sources();
                            continue;
                        }
                        self.selected.set(Some(index));
                        if let Some(dialog) = self.dialog.borrow().as_ref() {
                            obj::<gtk::Window>(dialog, "dialog").close();
                        }
                        self.render_sources();
                        self.controls();
                    }
                    Err(e) => {
                        if generation != self.dialog_generation.get() {
                            self.event(&e.to_string());
                            continue;
                        }
                        if let Some(dialog) = self.dialog.borrow().as_ref() {
                            if let Some(label) = dialog.object::<gtk::Label>("validation") {
                                label.set_text(&e.to_string());
                                obj::<gtk::Button>(dialog, "confirm").set_sensitive(true);
                            }
                        }
                    }
                },
                Message::Page(generation, result) => {
                    if generation == self.generation.get() {
                        match result {
                            Ok(page) => {
                                self.page.replace(page);
                                self.render_files();
                            }
                            Err(error) => self.event(&error.to_string()),
                        }
                    }
                }
                Message::Selected(result) => {
                    if let Err(error) = result {
                        self.info("Selection", &error.to_string());
                    }
                    self.reload();
                }
                Message::Opened(generation, dir, result) => {
                    if generation != self.view_generation.get() || self.deleting.get() {
                        continue;
                    }
                    match result {
                        Ok((manifest, page)) => {
                            self.invalidate_view();
                            let id = self
                                .sources
                                .borrow()
                                .iter()
                                .position(|s| s.same_target(&manifest.source));
                            let id = id.unwrap_or_else(|| {
                                let id = self.sources.borrow().len();
                                self.sources.borrow_mut().push(manifest.source.clone());
                                id
                            });
                            self.selected.set(Some(id));
                            self.directory.replace(Some(dir.clone()));
                            self.view_source.replace(Some(manifest.source.clone()));
                            self.options.replace(manifest.options.clone());
                            self.page.replace(page);
                            self.query.replace(Query {
                                limit: 250,
                                ..Default::default()
                            });
                            self.widget::<gtk::SearchEntry>("search").set_text("");
                            self.widget::<gtk::ToggleButton>("filter_all")
                                .set_active(true);
                            self.widget::<gtk::DropDown>("scan_mode")
                                .set_selected(index(manifest.options.mode));
                            self.label(
                                "status_text",
                                &format!("Saved scan · {} · {}", manifest.state, dir.display()),
                            );
                            self.label(
                                "progress_text",
                                if manifest.complete { "100%" } else { "" },
                            );
                            self.widget::<gtk::ProgressBar>("progress")
                                .set_fraction(if manifest.complete { 1.0 } else { 0.0 });
                            self.render_sources();
                            self.render_files();
                        }
                        Err(error) => self.info("Open saved scan", &error.to_string()),
                    }
                }
                Message::DeletePrepared(generation, result) => {
                    if generation != self.dialog_generation.get() || self.busy() {
                        continue;
                    }
                    match result {
                        Ok(plan) => self.confirm_delete(plan),
                        Err(error) => {
                            if let Some(dialog) = self.dialog.borrow().as_ref() {
                                text(dialog, "validation", &error.to_string());
                                obj::<gtk::Button>(dialog, "delete").set_sensitive(true);
                            }
                        }
                    }
                }
                Message::Deleted(directory, removed, result) => {
                    self.deleting.set(false);
                    let current = self.directory.borrow().as_ref() == Some(&directory);
                    if removed && current {
                        self.invalidate_view();
                        self.directory.replace(None);
                        self.view_source.replace(None);
                        self.page.replace(Page::default());
                        self.query.borrow_mut().offset = 0;
                        self.pulse.set(false);
                        self.widget::<gtk::ProgressBar>("progress")
                            .set_fraction(0.0);
                        self.label("progress_text", "");
                        self.render_files();
                    }
                    match result {
                        Ok(()) => {
                            self.label("status_text", "Saved scan deleted");
                            self.event(&format!("Deleted saved scan: {}", directory.display()));
                            if !self.quitting.get() {
                                self.saved_scans();
                            }
                        }
                        Err(error) => {
                            self.event(&error.to_string());
                            self.info("Could not delete saved scan", &error.to_string());
                            let root = self.options.borrow().workspace.clone();
                            self.load_saved(root);
                        }
                    }
                    self.controls();
                    if self.quitting.get() {
                        self.window().close();
                    }
                }
                Message::Preview(generation, result) => {
                    if generation == self.preview_generation.get() {
                        match result {
                            Ok(path) => {
                                if file_icon(path.to_str().unwrap_or(""))
                                    == "image-x-generic-symbolic"
                                {
                                    let picture: gtk::Picture = self.widget("preview_picture");
                                    picture.set_filename(Some(&path));
                                    picture.set_visible(true);
                                    self.label(
                                        "preview_text",
                                        "Image preview · recovery integrity has not been verified.",
                                    );
                                } else {
                                    use std::io::Read;
                                    let mut bytes = Vec::new();
                                    if let Ok(file) = std::fs::File::open(path) {
                                        let _ = file.take(65536).read_to_end(&mut bytes);
                                    }
                                    self.label("preview_text",if bytes.contains(&0){"Binary content. Recover the file to view it in its native application."}else{std::str::from_utf8(&bytes).unwrap_or("Binary content. Recover the file to inspect it.")});
                                }
                            }
                            Err(error) => self.label("preview_text", &error.to_string()),
                        }
                    }
                }
                Message::Saved(generation, result) => match result {
                    Ok(scans) => {
                        self.saved.replace(scans);
                        let sidebar: gtk::ListBox = self.widget("snapshots");
                        clear(&sidebar);
                        let dialog = self
                            .dialog
                            .borrow()
                            .clone()
                            .filter(|_| generation == self.dialog_generation.get());
                        let list = dialog
                            .as_ref()
                            .and_then(|d| d.object::<gtk::ListBox>("scans"));
                        if let Some(list) = &list {
                            clear(list);
                        }
                        for scan in self.saved.borrow().iter() {
                            for list in [Some(sidebar.clone()), list.clone()].into_iter().flatten()
                            {
                                let b =
                                    gtk::Builder::from_string(include_str!("../ui/source-row.ui"));
                                text(&b, "name", &scan.manifest.source.name);
                                text(
                                    &b,
                                    "detail",
                                    &format!(
                                        "{} · {} {} · {}",
                                        scan.manifest.state,
                                        scan.manifest.candidate_count,
                                        if scan.manifest.consolidation.is_some() {
                                            "groups"
                                        } else {
                                            "files"
                                        },
                                        date(scan.manifest.updated_at as i64)
                                    ),
                                );
                                obj::<gtk::Image>(&b, "icon")
                                    .set_icon_name(Some("document-open-recent-symbolic"));
                                list.append(&obj::<gtk::ListBoxRow>(&b, "row"));
                            }
                        }
                        if let Some(dialog) = dialog {
                            if let Some(label) = dialog.object::<gtk::Label>("validation") {
                                label.set_text(if self.saved.borrow().is_empty() {
                                    "No saved scans in this workspace."
                                } else {
                                    "Select a scan to open, compare, scan again, or delete."
                                });
                            }
                        }
                    }
                    Err(error) => self.info("Saved scans", &error.to_string()),
                },
                Message::Info(title, result) => {
                    self.info(&title, &result.unwrap_or_else(|error| error.to_string()))
                }
            }
        }
        if self.pulse.get() && !self.paused.get() {
            self.widget::<gtk::ProgressBar>("progress").pulse();
        }
    }
}

fn scan_options(b: &gtk::Builder) -> engine::Result<ScanOptions> {
    fn number(b: &gtk::Builder, id: &str) -> engine::Result<u64> {
        value(b, id)
            .parse()
            .map_err(|_| engine::Error::Message(format!("{id}: enter a nonnegative whole number.")))
    }
    fn optional(b: &gtk::Builder, id: &str) -> engine::Result<Option<u64>> {
        if value(b, id).is_empty() {
            Ok(None)
        } else {
            number(b, id).map(Some)
        }
    }
    let retries = number(b, "retries")?;
    if retries > 20 {
        return engine::fail("Retry passes must be between 0 and 20.");
    }
    let mut options = ScanOptions {
        mode: mode(obj::<gtk::DropDown>(b, "mode").selected()),
        workspace: PathBuf::from(value(b, "workspace")),
        extensions: engine::types::normalize_extensions(&value(b, "extensions"))?,
        include_existing: checked(b, "existing"),
        consolidate: checked(b, "consolidate"),
        clone_first: checked(b, "clone"),
        clone_backend: choice(b, "clone_engine"),
        retry_passes: retries as u32,
        metadata_backend: choice(b, "metadata"),
        carver: choice(b, "carver"),
        keep_corrupted: checked(b, "corrupted"),
        low_memory: checked(b, "low_memory"),
        offset_bytes: number(b, "offset")?,
        length_bytes: optional(b, "length")?,
        minimum_size: number(b, "minimum")?,
        maximum_size: optional(b, "maximum")?,
        timeout_seconds: number(b, "timeout")?,
        btrfs_root: optional(b, "btrfs_root")?,
    };
    options.validate()?;
    Ok(options)
}

pub fn activate(app: &gtk::Application, smoke: bool) {
    let css = gtk::CssProvider::new();
    css.load_from_data(include_str!("../ui/layout.css"));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("GTK display"),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let (tasks, rx) = mpsc::channel::<Task>();
    let (tx, messages) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(task) = rx.recv() {
            if tx.send(task()).is_err() {
                break;
            }
        }
    });
    let ui = Rc::new(Live {
        builder: gtk::Builder::from_string(include_str!("../ui/window.ui")),
        dialog: RefCell::new(None),
        dialog_generation: Cell::new(0),
        finding_cursor: Cell::new(0),
        sources: RefCell::new(vec![]),
        selected: Cell::new(None),
        source_rows: RefCell::new(vec![]),
        collapsed_disks: RefCell::new(BTreeSet::new()),
        collapsed: RefCell::new(BTreeSet::new()),
        directory: RefCell::new(None),
        view_source: RefCell::new(None),
        options: RefCell::new(ScanOptions::default()),
        page: RefCell::new(Page::default()),
        query: RefCell::new(Query {
            limit: 250,
            ..Default::default()
        }),
        files: RefCell::new(vec![]),
        job: RefCell::new(None),
        deleting: Cell::new(false),
        partition_search: RefCell::new(None),
        partition_report: RefCell::new(None),
        partition_source: RefCell::new(None),
        partition_generation: Cell::new(0),
        close_partition_search: Cell::new(false),
        paused: Cell::new(false),
        pulse: Cell::new(false),
        rendering: Cell::new(false),
        generation: Cell::new(0),
        view_generation: Cell::new(0),
        preview_generation: Cell::new(0),
        saved: RefCell::new(vec![]),
        quitting: Cell::new(false),
        tasks,
        messages,
    });
    ui.window().set_application(Some(app));
    ui.window().set_title(Some("Open Data Recovery"));
    ui.widget::<gtk::Box>("demo_banner").set_visible(false);
    ui.label(
        "snapshot_notice",
        "Scans are saved automatically in your workspace.",
    );
    ui.widget::<gtk::Label>("demo_indicator").set_visible(false);
    ui.label("progress_text", "0%");
    ui.widget::<gtk::ProgressBar>("progress").set_fraction(0.0);
    ui.widget::<gtk::Picture>("preview_picture")
        .set_visible(false);
    ui.widget::<gtk::Button>("add_source")
        .set_tooltip_text(Some("Add an image, device or mounted folder"));
    ui.widget::<gtk::Button>("save_snapshot")
        .set_tooltip_text(Some("Browse saved scans"));
    ui.connect(app);
    ui.render_files();
    ui.refresh();
    ui.load_saved(engine::types::default_workspace());
    ui.event("Source access is read-only. Image files and output folders may share a drive; device sources require a separate drive.");
    ui.window().present();
    let lifetime = Rc::new(RefCell::new(Some(ui.clone())));
    ui.window().connect_destroy(move |_| {
        lifetime.borrow_mut().take();
    });
    let weak = Rc::downgrade(&ui);
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if let Some(ui) = weak.upgrade() {
            ui.tick();
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });
    if smoke {
        if partitions_smoke::run(&ui) {
            return;
        }
        if let (Some(session), Some(destination)) = (
            std::env::var_os("ODR_SMOKE_SESSION"),
            std::env::var_os("ODR_SMOKE_DESTINATION"),
        ) {
            let session = PathBuf::from(session);
            let destination = PathBuf::from(destination);
            let inspect_consolidation = std::env::var_os("ODR_SMOKE_CONSOLIDATION").is_some();
            let all_paths = std::env::var_os("ODR_SMOKE_ALL_PATHS").is_some();
            let other_source = std::env::var_os("ODR_SMOKE_OTHER_SOURCE").map(PathBuf::from);
            ui.open(session.clone());
            let weak = Rc::downgrade(&ui);
            let mut step = 0;
            let mut ticks = 0;
            let mut switched_sources = false;
            let mut selected_before_switch = 0;
            let mut preview_before_switch = String::new();
            let mut checked_reopen = false;
            let mut reopen_generation = 0;
            glib::timeout_add_local(Duration::from_millis(100), move || {
                let Some(ui) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                ticks += 1;
                assert!(
                    ticks < 400,
                    "GUI recovery workflow stalled: {}",
                    ui.widget::<gtk::Label>("status_text").text()
                );
                match step {
                    0 if ui.directory.borrow().as_ref() == Some(&session)
                        && !ui.page.borrow().files.is_empty() =>
                    {
                        if other_source.is_some() && !checked_reopen {
                            // A pending old-page refresh must neither cancel an
                            // explicit open nor overwrite its new result view.
                            ui.open(session.clone());
                            reopen_generation = ui.view_generation.get();
                            ui.reload();
                            checked_reopen = true;
                            step = 30;
                            return glib::ControlFlow::Continue;
                        }
                        if inspect_consolidation {
                            let group = ui
                                .page
                                .borrow()
                                .files
                                .iter()
                                .find(|f| f.findings > 1)
                                .cloned()
                                .expect("consolidated group");
                            assert!(ui.page.borrow().consolidated);
                            ui.show_findings(group);
                            step = 10;
                            return glib::ControlFlow::Continue;
                        }
                        ui.widget::<gtk::CheckButton>("select_all").set_active(true);
                        step = 1;
                    }
                    30 if ui.view_generation.get() > reopen_generation => {
                        step = 0;
                    }
                    10 => {
                        let b = ui.dialog.borrow().clone().unwrap();
                        let buffer = obj::<gtk::TextView>(&b, "body").buffer();
                        let body = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
                        if body.contains("Address:") {
                            assert!(body.contains("recovered.txt") && body.contains("alias.txt"));
                            obj::<gtk::Window>(&b, "dialog").close();
                            ui.widget::<gtk::CheckButton>("select_all").set_active(true);
                            step = 1;
                        }
                    }
                    1 if ui.page.borrow().selected > 0 => {
                        if other_source.is_some() && !switched_sources {
                            let id = ui.page.borrow().files.first().unwrap().id;
                            ui.preview(id);
                            ui.widget::<gtk::SearchEntry>("search").set_text("txt");
                            step = 11;
                            return glib::ControlFlow::Continue;
                        }
                        ui.recovery();
                        let b = ui.dialog.borrow().clone().unwrap();
                        if all_paths {
                            assert!(obj::<gtk::CheckButton>(&b, "all_paths").is_visible());
                            obj::<gtk::CheckButton>(&b, "all_paths").set_active(true);
                        }
                        obj::<gtk::Entry>(&b, "destination_path")
                            .set_text(&destination.to_string_lossy());
                        step = 2;
                    }
                    11 if ui
                        .widget::<gtk::Label>("preview_text")
                        .text()
                        .contains("Verified recovery") =>
                    {
                        selected_before_switch = ui.page.borrow().selected;
                        preview_before_switch =
                            ui.widget::<gtk::Label>("preview_text").text().to_string();
                        ui.add_source();
                        let b = ui.dialog.borrow().clone().unwrap();
                        obj::<gtk::Entry>(&b, "path")
                            .set_text(&other_source.as_ref().unwrap().to_string_lossy());
                        obj::<gtk::Button>(&b, "confirm").emit_clicked();
                        step = 12;
                    }
                    12 if ui
                        .selected
                        .get()
                        .and_then(|id| ui.sources.borrow().get(id).map(|s| s.path.clone()))
                        .as_ref()
                        == other_source.as_ref() =>
                    {
                        assert_eq!(ui.directory.borrow().as_ref(), Some(&session));
                        assert_eq!(ui.page.borrow().selected, selected_before_switch);
                        assert_eq!(ui.query.borrow().text, "txt");
                        assert_eq!(
                            ui.widget::<gtk::Label>("preview_text").text().as_str(),
                            preview_before_switch
                        );
                        let original = other_source.as_ref().unwrap().with_file_name("source.img");
                        assert_eq!(ui.view_source.borrow().as_ref().unwrap().path, original);
                        assert!(
                            ui.widget::<gtk::Label>("source_title")
                                .text()
                                .starts_with("Next scan:")
                        );
                        // Exercise real sidebar row clicks in both directions.
                        let other = ui.selected.get().unwrap();
                        let first = ui
                            .sources
                            .borrow()
                            .iter()
                            .position(|s| s.path == original)
                            .unwrap();
                        for source in [first, other] {
                            let row_index = ui
                                .source_rows
                                .borrow()
                                .iter()
                                .position(|id| *id == source)
                                .unwrap();
                            let list: gtk::ListBox = ui.widget("sources");
                            let row = list.row_at_index(row_index as i32).unwrap();
                            list.select_row(Some(&row));
                            assert_eq!(ui.directory.borrow().as_ref(), Some(&session));
                            assert_eq!(ui.page.borrow().selected, selected_before_switch);
                            assert_eq!(ui.query.borrow().text, "txt");
                            assert_eq!(
                                ui.widget::<gtk::Label>("preview_text").text().as_str(),
                                preview_before_switch
                            );
                        }
                        switched_sources = true;
                        step = 1;
                    }
                    2 => {
                        let b = ui.dialog.borrow().clone().unwrap();
                        let button: gtk::Button = obj(&b, "confirm");
                        if button.is_sensitive() {
                            button.emit_clicked();
                            step = 3;
                        }
                    }
                    3 if !ui.busy() => {
                        let file = ui.page.borrow().files.first().unwrap().path.clone();
                        assert!(destination.join(file).is_file());
                        assert!(
                            std::fs::read_to_string(session.join("events.log"))
                                .unwrap()
                                .contains("\"verified\":true")
                        );
                        println!(
                            "Production GTK recovery workflow passed: saved scan, selection, destination validation, options, verified output."
                        );
                        if other_source.is_some() {
                            ui.saved_scans();
                            step = 20;
                            return glib::ControlFlow::Continue;
                        }
                        ui.window().close();
                        return glib::ControlFlow::Break;
                    }
                    20 | 22 => {
                        let Some(b) = ui.dialog.borrow().clone() else {
                            return glib::ControlFlow::Continue;
                        };
                        if let Some(list) = b.object::<gtk::ListBox>("scans") {
                            let index = ui
                                .saved
                                .borrow()
                                .iter()
                                .position(|s| s.directory == session);
                            if let Some(row) = index.and_then(|i| list.row_at_index(i as i32)) {
                                list.select_row(Some(&row));
                                obj::<gtk::Button>(&b, "delete").emit_clicked();
                                step += 1;
                            }
                        }
                    }
                    21 | 23 => {
                        let b = ui.dialog.borrow().clone().unwrap();
                        if obj::<gtk::Window>(&b, "dialog").title().as_deref()
                            == Some("Delete saved scan?")
                        {
                            assert!(
                                obj::<gtk::Label>(&b, "details")
                                    .text()
                                    .contains(session.to_str().unwrap())
                            );
                            assert!(session.join("manifest.json").exists());
                            if step == 21 {
                                obj::<gtk::Button>(&b, "close").emit_clicked();
                            } else {
                                obj::<gtk::Button>(&b, "confirm").emit_clicked();
                            }
                            step += 1;
                        }
                    }
                    24 if !ui.busy()
                        && !session.exists()
                        && ui.saved.borrow().iter().all(|s| s.directory != session) =>
                    {
                        assert!(ui.directory.borrow().is_none());
                        assert_eq!(ui.page.borrow().total, 0);
                        assert_eq!(ui.page.borrow().selected, 0);
                        assert!(ui.view_source.borrow().is_none());
                        assert!(destination.join("recovered.txt").is_file());
                        assert!(
                            other_source
                                .as_ref()
                                .unwrap()
                                .with_file_name("source.img")
                                .is_file()
                        );
                        println!(
                            "GTK saved-scan management passed: source switching preserved selection/search/preview, recovery used the open scan, cancellation kept the folder, and confirmed deletion removed it without deleting recovered files."
                        );
                        ui.window().close();
                        return glib::ControlFlow::Break;
                    }
                    _ => {}
                }
                glib::ControlFlow::Continue
            });
            return;
        }
        let weak = Rc::downgrade(&ui);
        glib::timeout_add_local_once(Duration::from_millis(700), move || {
            if let Some(ui) = weak.upgrade() {
                ui.custom_scan();
                let b = ui.dialog.borrow().clone().unwrap();
                assert!(scan_options(&b).is_ok());
                assert!(!scan_options(&b).unwrap().consolidate);
                obj::<gtk::CheckButton>(&b, "consolidate").set_active(true);
                assert!(scan_options(&b).unwrap().consolidate);
                assert!(
                    obj::<gtk::MenuButton>(&ui.builder, "scan_dropdown")
                        .menu_model()
                        .is_some()
                );
                ui.saved_scans();
                assert!(
                    ui.dialog
                        .borrow()
                        .as_ref()
                        .unwrap()
                        .object::<gtk::ListBox>("scans")
                        .is_some()
                );
                ui.find_partitions();
                assert!(
                    ui.dialog
                        .borrow()
                        .as_ref()
                        .unwrap()
                        .object::<gtk::ListBox>("partitions")
                        .is_some()
                );
                ui.add_source();
                ui.info("Smoke check", "Production dialogs loaded successfully.");
                println!("Production GTK smoke check passed");
                ui.window().close();
            }
        });
    }
}
