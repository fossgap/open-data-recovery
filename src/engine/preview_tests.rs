use super::*;

#[test]
fn thumbnails_decode_supported_formats_and_preserve_png_alpha() {
    let rgb = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        20,
        10,
        image::Rgb([64, 128, 192]),
    ));
    for format in [
        image::ImageFormat::Png,
        image::ImageFormat::Jpeg,
        image::ImageFormat::Gif,
        image::ImageFormat::Bmp,
        image::ImageFormat::WebP,
        image::ImageFormat::Tiff,
    ] {
        let mut encoded = std::io::Cursor::new(Vec::new());
        rgb.write_to(&mut encoded, format).unwrap();
        let decoded = thumbnail(encoded.into_inner()).unwrap();
        assert_eq!(decoded.dimensions(), (20, 10), "{format:?}");
        assert_eq!(decoded.get_pixel(0, 0)[3], 255);
    }
    let rgba = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        3,
        2,
        image::Rgba([17, 31, 51, 63]),
    ));
    let mut encoded = std::io::Cursor::new(Vec::new());
    rgba.write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    assert_eq!(
        *thumbnail(encoded.into_inner()).unwrap().get_pixel(0, 0),
        image::Rgba([17, 31, 51, 63])
    );
}

#[test]
fn jpeg_preview_applies_exif_orientation() {
    let rgb = image::DynamicImage::new_rgb8(20, 10);
    let mut jpeg = std::io::Cursor::new(Vec::new());
    rgb.write_to(&mut jpeg, image::ImageFormat::Jpeg).unwrap();
    // Little-endian TIFF IFD containing Orientation = 6 (rotate 90 degrees).
    let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut encoded = vec![0xff, 0xd8, 0xff, 0xe1];
    encoded.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    encoded.extend_from_slice(exif);
    encoded.extend_from_slice(&jpeg.into_inner()[2..]);
    assert_eq!(thumbnail(encoded).unwrap().dimensions(), (10, 20));
}

fn sleeping_decoder() -> Child {
    Command::new("sh")
        .args(["-c", "sleep 30"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .unwrap()
}

#[test]
fn obsolete_decoder_is_cancelled_and_its_process_group_reaped() {
    let child = sleeping_decoder();
    let pid = child.id();
    let control = Control::default();
    let cancelling = control.clone();
    let cancel = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        cancelling.cancel();
    });
    let start = Instant::now();
    assert!(matches!(
        read_decoder(child, &control, Duration::from_secs(8)),
        Err(Error::Cancelled)
    ));
    cancel.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
}

#[test]
fn stalled_decoder_times_out_without_waiting_for_its_output() {
    let start = Instant::now();
    let error = read_decoder(
        sleeping_decoder(),
        &Control::default(),
        Duration::from_millis(30),
    )
    .unwrap_err();
    assert!(error.to_string().contains("timed out"));
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn large_previews_are_scaled_and_excessive_dimensions_rejected() {
    assert_eq!(thumbnail_size(8192, 8192).unwrap(), (1024, 1024));
    assert_eq!(thumbnail_size(6000, 4000).unwrap(), (1024, 682));
    assert_eq!(thumbnail_size(5, 9).unwrap(), (5, 9));
    for (width, height) in [(0, 10), (-1, 2), (8193, 8193), (100000, 1)] {
        assert!(thumbnail_size(width, height).is_err());
    }
}

#[test]
fn decoder_protocol_rejects_truncation_overflow_and_extra_bytes() {
    let mut response = b"ODRP".to_vec();
    response.extend_from_slice(&1u32.to_le_bytes());
    response.extend_from_slice(&1u32.to_le_bytes());
    response.extend_from_slice(&[17, 31, 51, 255]);
    let image = parse_image(response.clone()).unwrap();
    assert_eq!(image.pixels.as_ref(), &[17, 31, 51, 255]);
    assert!(parse_image(response[..15].into()).is_err());
    let mut extra = response.clone();
    extra.push(0);
    assert!(parse_image(extra).is_err());
    response[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(parse_image(response).is_err());
}

#[test]
fn thumbnail_cache_invalidates_changed_files_and_evicts_old_entries() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("image");
    std::fs::write(&file, b"first").unwrap();
    let first = Stamp::new(&std::fs::metadata(&file).unwrap()).unwrap();
    let image = Image {
        width: 1,
        height: 1,
        pixels: glib::Bytes::from_owned(vec![1, 2, 3, 255]),
    };
    let mut cache = Cache::default();
    cache.insert(first.clone(), image.clone());
    assert!(cache.get(&first).is_some());
    std::fs::write(&file, b"different").unwrap();
    let changed = Stamp::new(&std::fs::metadata(&file).unwrap()).unwrap();
    assert!(cache.get(&changed).is_none());
    for inode in 100..108 {
        cache.insert(
            Stamp {
                inode,
                ..first.clone()
            },
            image.clone(),
        );
    }
    assert!(cache.get(&first).is_none());
    assert_eq!(cache.entries.len(), 8);
    assert_eq!(cache.bytes, 8 * 4);
}
