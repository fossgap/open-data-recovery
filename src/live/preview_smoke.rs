//! Production preview/refresh regression on a disposable saved scan.
use super::*;

pub(super) fn run(ui: &Rc<Live>) -> bool {
    let Some(session) = std::env::var_os("ODR_SMOKE_PREVIEWS") else {
        return false;
    };
    ui.open(PathBuf::from(session));
    let weak = Rc::downgrade(ui);
    let mut step = 0;
    let began = Instant::now();
    let mut requested = Instant::now();
    let mut beats = 0;
    let mut max_gap = Duration::ZERO;
    let mut previous = Instant::now();
    let mut large_id = 0;
    let mut text_id = 0;
    let mut broken_id = 0;
    let mut excessive_id = 0;
    let mut refresh_generation = 0;
    glib::timeout_add_local(Duration::from_millis(10), move || {
        let Some(ui) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        assert!(
            began.elapsed() < Duration::from_secs(30),
            "Preview workflow stalled at {step}"
        );
        let now = Instant::now();
        if step == 1 {
            max_gap = max_gap.max(now.duration_since(previous));
            beats += 1;
        }
        previous = now;
        let picture = ui.widget::<gtk::Picture>("preview_picture");
        let body = ui.widget::<gtk::Label>("preview_text").text();
        match step {
            0 if !ui.page.borrow().files.is_empty() => {
                let page = ui.page.borrow();
                let id = |name: &str| {
                    page.files
                        .iter()
                        .find(|file| file.path.ends_with(name))
                        .unwrap()
                        .id
                };
                large_id = id("large.png");
                text_id = id("message.txt");
                broken_id = id("broken.png");
                excessive_id = id("excessive.png");
                drop(page);
                ui.preview(large_id);
                requested = Instant::now();
                step = 1;
            }
            1 if picture.is_visible() => {
                let texture = picture
                    .paintable()
                    .unwrap()
                    .downcast::<gtk::gdk::Texture>()
                    .unwrap();
                assert_eq!((texture.width(), texture.height()), (1024, 1024));
                let mut pixels = vec![0; 1024 * 1024 * 4];
                texture.download(&mut pixels, 1024 * 4);
                // Texture.download uses Cairo's native premultiplied format (BGRA on x86).
                assert_eq!(&pixels[..4], &[192, 128, 64, 255]);
                assert!(
                    beats >= 3,
                    "GTK heartbeat stopped while preview was loading"
                );
                assert!(
                    max_gap < Duration::from_millis(150),
                    "GTK stalled for {max_gap:?}"
                );
                println!(
                    "Preview: 8192x8192 -> 1024x1024; latency={:?}; heartbeats={beats}; max_gap={max_gap:?}",
                    requested.elapsed()
                );
                // Repeated selection returns the cached pixels, then text replaces the image.
                ui.preview(large_id);
                step = 2;
            }
            2 if picture.is_visible() => {
                for _ in 0..100 {
                    ui.preview(large_id);
                }
                ui.preview(text_id);
                step = 3;
            }
            3 if body.contains("Preview worker stays responsive") => {
                assert!(!picture.is_visible());
                assert!(picture.paintable().is_none());
                ui.preview(broken_id);
                step = 4;
            }
            4 if body.contains("Image preview unavailable") => {
                assert!(!picture.is_visible());
                ui.preview(excessive_id);
                step = 5;
            }
            5 if body.contains("Image preview unavailable") => {
                assert!(!picture.is_visible());
                // Only one page read is submitted now; all further changes collapse
                // into a single latest-query read after that in-flight task completes.
                for index in 0..1000 {
                    ui.query.borrow_mut().text = if index == 999 {
                        "message.txt"
                    } else {
                        "large.png"
                    }
                    .into();
                    ui.reload();
                }
                assert!(ui.page_loading.get() && ui.page_dirty.get());
                refresh_generation = ui.generation.get();
                step = 6;
            }
            6 if !ui.page_loading.get() && !ui.page_dirty.get() => {
                assert_eq!(ui.generation.get(), refresh_generation);
                assert_eq!(ui.page.borrow().files.len(), 1);
                assert!(ui.page.borrow().files[0].path.ends_with("message.txt"));
                ui.preview(text_id);
                ui.cancel_preview();
                assert!(!ui.preview_loading.get());
                assert_eq!(
                    ui.widget::<gtk::Label>("preview_text").text(),
                    tr("Cancelled")
                );
                ui.preview(text_id);
                ui.invalidate_view();
                requested = Instant::now();
                step = 7;
            }
            7 if requested.elapsed() > Duration::from_millis(300) => {
                assert_eq!(
                    ui.widget::<gtk::Label>("preview_name").text(),
                    tr("Choose a file")
                );
                assert!(!picture.is_visible());
                println!(
                    "GTK preview workflow passed: bounded image, heartbeat, rapid selection, text, malformed/oversized images, latest query and invalidated view."
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
