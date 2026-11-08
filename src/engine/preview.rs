//! Bounded preview data. Decoders never run in the GTK/application process.
use super::{Error, Result, fail, jobs, process::Control, store::Store};
use gtk::glib;
use image::ImageDecoder;
use std::{
    collections::VecDeque,
    fs::{File, Metadata, OpenOptions},
    io::{Read, Write},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt},
        process::CommandExt,
    },
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub const MAX_EDGE: u32 = 1024;
const MAX_PIXELS: u64 = 64 * 1024 * 1024;
const MAX_ENCODED: u64 = 8 * 1024 * 1024;
const MAX_OUTPUT: usize = MAX_EDGE as usize * MAX_EDGE as usize * 4;
const CACHE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: glib::Bytes,
}

#[derive(Clone, Debug)]
pub enum Content {
    Image(Image),
    Text(String),
    Binary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    device: u64,
    inode: u64,
    length: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl Stamp {
    fn new(meta: &Metadata) -> Result<Self> {
        if !meta.is_file() || meta.len() > MAX_ENCODED {
            return fail("Preview requires a regular file of at most 8 MiB.");
        }
        Ok(Self {
            device: meta.dev(),
            inode: meta.ino(),
            length: meta.len(),
            modified: (meta.mtime(), meta.mtime_nsec()),
            changed: (meta.ctime(), meta.ctime_nsec()),
        })
    }
}

#[derive(Default)]
pub struct Cache {
    entries: VecDeque<(Stamp, Image)>,
    bytes: usize,
}
impl Cache {
    fn get(&mut self, stamp: &Stamp) -> Option<Image> {
        let index = self.entries.iter().position(|(key, _)| key == stamp)?;
        let entry = self.entries.remove(index)?;
        let image = entry.1.clone();
        self.entries.push_back(entry);
        Some(image)
    }
    fn insert(&mut self, stamp: Stamp, image: Image) {
        while self.entries.len() >= 8 || self.bytes + image.pixels.len() > CACHE_BYTES {
            let Some((_, old)) = self.entries.pop_front() else {
                break;
            };
            self.bytes -= old.pixels.len();
        }
        self.bytes += image.pixels.len();
        self.entries.push_back((stamp, image));
    }
}

pub fn load(
    directory: &Path,
    id: i64,
    image: bool,
    control: &Control,
    cache: &mut Cache,
) -> Result<Content> {
    // Hold the shared session lock through extraction and decoding, including cache hits.
    let _session = Store::open(directory)?;
    let path = jobs::preview(directory, id, control)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let stamp = Stamp::new(&file.metadata()?)?;
    control.checkpoint()?;
    if image {
        if let Some(image) = cache.get(&stamp) {
            return Ok(Content::Image(image));
        }
        let decoded = decode(&file, control)?;
        control.checkpoint()?;
        if Stamp::new(&file.metadata()?)? != stamp {
            return fail("The preview file changed while it was being read.");
        }
        cache.insert(stamp, decoded.clone());
        Ok(Content::Image(decoded))
    } else {
        let mut bytes = Vec::new();
        file.take(65536).read_to_end(&mut bytes)?;
        control.checkpoint()?;
        Ok(match std::str::from_utf8(&bytes) {
            Ok(text) if !bytes.contains(&0) => Content::Text(text.into()),
            _ => Content::Binary,
        })
    }
}

fn parse_image(bytes: Vec<u8>) -> Result<Image> {
    if bytes.len() < 12 || &bytes[..4] != b"ODRP" {
        return fail("Invalid image preview response.");
    }
    let width = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let height = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    if width == 0
        || height == 0
        || width > MAX_EDGE
        || height > MAX_EDGE
        || bytes.len() != 12 + width as usize * height as usize * 4
    {
        return fail("Image preview exceeds its pixel limit or is incomplete.");
    }
    Ok(Image {
        width,
        height,
        pixels: glib::Bytes::from_owned(bytes[12..].to_vec()),
    })
}

fn decode(file: &File, control: &Control) -> Result<Image> {
    control.checkpoint()?;
    // Keep AppImage's library environment: this is our own executable, not a host engine.
    let child = Command::new(std::env::current_exe()?)
        .arg("--decode-preview")
        .env("GIO_USE_VFS", "local")
        .stdin(Stdio::from(file.try_clone()?))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    read_decoder(child, control, Duration::from_secs(8))
}

fn read_decoder(mut child: Child, control: &Control, timeout: Duration) -> Result<Image> {
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let output = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(MAX_OUTPUT as u64 + 13)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let errors = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.take(4096).read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    });
    let start = Instant::now();
    let status = loop {
        if let Err(error) = control.checkpoint() {
            break Err(error);
        }
        if start.elapsed() > timeout {
            break fail("Image preview timed out. Recover the file to inspect it externally.");
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => break Err(error.into()),
        }
    };
    if status.is_err() {
        // Reap the helper and terminate any descendants holding its output pipes.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        let _ = child.wait();
    }
    let bytes = output
        .join()
        .map_err(|_| Error::Message("Image preview reader stopped.".into()))?;
    let errors = errors.join().unwrap_or_default();
    if !status?.success() {
        return fail(format!(
            "Image preview unavailable or exceeds its limits. Recover the file to inspect it externally. {}",
            errors.trim()
        ));
    }
    parse_image(bytes?)
}

fn thumbnail_size(width: i32, height: i32) -> Result<(i32, i32)> {
    if width <= 0
        || height <= 0
        || width > 32768
        || height > 32768
        || width as u64 * height as u64 > MAX_PIXELS
    {
        return fail("Image dimensions exceed the 64-megapixel preview limit.");
    }
    let edge = width.max(height).max(MAX_EDGE as i32) as i64;
    Ok((
        (width as i64 * MAX_EDGE as i64 / edge).max(1) as i32,
        (height as i64 * MAX_EDGE as i64 / edge).max(1) as i32,
    ))
}

pub fn decoder_main() -> Result<()> {
    super::process::confine_preview_decoder()?;
    let mut encoded = Vec::new();
    std::io::stdin()
        .lock()
        .take(MAX_ENCODED + 1)
        .read_to_end(&mut encoded)?;
    let rgba = thumbnail(encoded)?;
    let mut output = std::io::stdout().lock();
    output.write_all(b"ODRP")?;
    output.write_all(&rgba.width().to_le_bytes())?;
    output.write_all(&rgba.height().to_le_bytes())?;
    output.write_all(rgba.as_raw())?;
    Ok(())
}

fn thumbnail(encoded: Vec<u8>) -> Result<image::RgbaImage> {
    if encoded.len() as u64 > MAX_ENCODED {
        return fail("Encoded image exceeds 8 MiB.");
    }
    let error = |e: image::ImageError| Error::Message(e.to_string());
    let mut reader =
        image::ImageReader::new(std::io::Cursor::new(encoded)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(32768);
    limits.max_image_height = Some(32768);
    limits.max_alloc = Some(384 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(error)?;
    let (width, height) = decoder.dimensions();
    let (width, height) = thumbnail_size(width as i32, height as i32)?;
    if decoder.total_bytes() > 256 * 1024 * 1024 {
        return fail("Decoded image exceeds the preview memory limit.");
    }
    let orientation = decoder.orientation().map_err(error)?;
    let decoded = image::DynamicImage::from_decoder(decoder).map_err(error)?;
    let mut thumbnail = decoded.thumbnail(width as u32, height as u32);
    drop(decoded);
    thumbnail.apply_orientation(orientation);
    Ok(thumbnail.into_rgba8())
}

#[cfg(test)]
#[path = "preview_tests.rs"]
mod tests;
