use super::*;
#[test]
fn invalid_options_cannot_overflow_canvases_or_inject_filters() {
    assert!(validate_options(
        &Options::Pack {
            columns: 256,
            frame_width: 4096,
            frame_height: 4096,
            padding: 256,
            fit: Fit::Contain
        },
        256
    )
    .is_err());
    assert!(validate_options(
        &Options::Split {
            columns: 257,
            rows: 1,
            padding: 0,
            order: Order::RowMajor
        },
        1
    )
    .is_err());
    assert!(validate_options(&Options::Stitch { cuts: vec![] }, 2).is_err());
    assert!(validate_options(
        &Options::Icons {
            sizes: vec![512],
            fit: Fit::Contain,
            background: Background::Transparent
        },
        1
    )
    .is_err());
    assert!(validate_options(
        &Options::Icons {
            sizes: vec![32, 32],
            fit: Fit::Contain,
            background: Background::Transparent
        },
        1
    )
    .is_err());
    assert!(color("red;movie=http://private").is_err());
    assert!(color("#12345g").is_err());
    assert_eq!(color("#10aBff").unwrap(), [16, 171, 255]);
    assert!(serde_json::from_value::<Options>(json!({"kind":"pack","columns":1,"frameWidth":32,"frameHeight":32,"padding":0,"fit":"contain","filter":"movie=secret"})).is_err());
}
#[test]
fn atlas_dimensions_include_exact_outer_and_between_frame_padding() {
    assert_eq!(pack_dimensions(3, 2, 16, 24, 2).unwrap(), (38, 54));
    assert!(pixel_bytes(100000, 1).is_err());
    assert!(pixel_bytes(10000, 10000).is_err());
}
fn fixture_item(path: &Path) -> LibraryItem {
    let mut item = library::index_file(path).unwrap();
    item.kind = "image".into();
    item
}
fn solid(width: u32, height: u32, color: [u8; 4]) -> Rgba {
    let mut image = Rgba::blank(width, height).unwrap();
    for pixel in image.bytes.chunks_exact_mut(4) {
        pixel.copy_from_slice(&color);
    }
    image
}
fn row_pattern(start: u32, height: u32) -> Rgba {
    let mut image = Rgba::blank(64, height).unwrap();
    for y in 0..height {
        for x in 0..64 {
            let seed = (start + y).wrapping_mul(1103515245).wrapping_add(x * 12345);
            let offset = ((y * 64 + x) * 4) as usize;
            image.bytes[offset..offset + 4].copy_from_slice(&[
                (seed >> 17) as u8,
                (seed >> 9) as u8,
                (seed >> 1) as u8,
                255,
            ]);
        }
    }
    image
}
#[tokio::test]
#[ignore = "Requires deliberately prepared FFmpeg in NABNAB_TEST_ENGINES; uses synthetic local PNGs only."]
async fn real_image_assets_verify_png_alpha_atlas_coordinates_icon_entries_and_originals() {
    let engines = PathBuf::from(
        std::env::var_os("NABNAB_TEST_ENGINES")
            .expect("Set NABNAB_TEST_ENGINES to the managed FFmpeg directory."),
    );
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let work = std::env::temp_dir().join(format!(
        "nabnab-image-assets-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&work).unwrap();
    let (_sender, cancel) = watch::channel(false);
    let a = write_png(
        &engines,
        &solid(64, 96, [255, 0, 0, 128]),
        &work,
        "red-alpha.png",
        cancel.clone(),
        0,
    )
    .await
    .unwrap();
    let b = write_png(
        &engines,
        &solid(64, 96, [0, 255, 0, 255]),
        &work,
        "green.png",
        cancel.clone(),
        0,
    )
    .await
    .unwrap();
    let originals = [fs::read(&a).unwrap(), fs::read(&b).unwrap()];
    let items = vec![fixture_item(&a), fixture_item(&b)];
    let stage = work.join("pack");
    fs::create_dir(&stage).unwrap();
    let packed = execute(
        &engines,
        &items,
        &stage,
        &Options::Pack {
            columns: 2,
            frame_width: 64,
            frame_height: 96,
            padding: 2,
            fit: Fit::Contain,
        },
        cancel.clone(),
        0,
        |_| {},
    )
    .await
    .unwrap();
    assert!(fs::read(packed).unwrap().starts_with(b"PK\x03\x04"));
    let atlas: serde_json::Value =
        serde_json::from_slice(&fs::read(stage.join("atlas.json")).unwrap()).unwrap();
    assert_eq!(atlas["width"], 134);
    assert_eq!(atlas["height"], 100);
    assert_eq!(
        atlas["frames"][0]["frame"],
        json!({"x":2,"y":2,"width":64,"height":96})
    );
    assert_eq!(atlas["frames"][1]["frame"]["x"], 68);
    let decoded = decode(
        &engines,
        &stage.join("atlas.png"),
        134,
        100,
        None,
        &stage,
        "verify",
        cancel.clone(),
        0,
    )
    .await
    .unwrap();
    assert_eq!(&decoded.bytes[..4], &[0, 0, 0, 0]);
    let red = ((2 * 134 + 2) * 4) as usize;
    assert_eq!(&decoded.bytes[red..red + 4], &[255, 0, 0, 128]);
    let green = ((2 * 134 + 68) * 4) as usize;
    assert_eq!(&decoded.bytes[green..green + 4], &[0, 255, 0, 255]);
    assert!(!fs::read_to_string(stage.join("atlas.json"))
        .unwrap()
        .contains(&work.to_string_lossy().into_owned()));
    let stage = work.join("icons");
    fs::create_dir(&stage).unwrap();
    execute(
        &engines,
        &items[..1],
        &stage,
        &Options::Icons {
            sizes: vec![16, 32, 256, 512],
            fit: Fit::Contain,
            background: Background::Solid {
                color: "#0000ff".into(),
            },
        },
        cancel.clone(),
        1,
        |_| {},
    )
    .await
    .unwrap();
    let icon = decode(
        &engines,
        &stage.join("icon-32.png"),
        32,
        32,
        None,
        &stage,
        "verify",
        cancel.clone(),
        0,
    )
    .await
    .unwrap();
    assert_eq!(&icon.bytes[..4], &[0, 0, 255, 255]);
    let center = ((16 * 32 + 16) * 4) as usize;
    assert!(icon.bytes[center].abs_diff(128) <= 1);
    assert_eq!(icon.bytes[center + 1], 0);
    assert!(icon.bytes[center + 2].abs_diff(127) <= 1);
    assert_eq!(icon.bytes[center + 3], 255);
    let ico = fs::read(stage.join("favicon.ico")).unwrap();
    assert_eq!(&ico[..6], &[0, 0, 1, 0, 3, 0]);
    for index in 0..3 {
        let entry = 6 + index * 16;
        let length = u32::from_le_bytes(ico[entry + 8..entry + 12].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(ico[entry + 12..entry + 16].try_into().unwrap()) as usize;
        assert!(ico[offset..offset + length].starts_with(b"\x89PNG\r\n\x1a\n"));
    }
    let stage = work.join("transparent-icons");
    fs::create_dir(&stage).unwrap();
    let preview = preview(
        &engines,
        &items[..1],
        &stage,
        &Options::Icons {
            sizes: vec![32],
            fit: Fit::Contain,
            background: Background::Transparent,
        },
        cancel.clone(),
        2,
    )
    .await
    .unwrap();
    assert!(preview.png.starts_with(b"\x89PNG"));
    let icon = decode(
        &engines,
        &stage.join("icon-32.png"),
        32,
        32,
        None,
        &stage,
        "verify",
        cancel.clone(),
        0,
    )
    .await
    .unwrap();
    assert_eq!(icon.bytes[3], 0);
    assert!(icon.bytes[center + 3].abs_diff(128) <= 1);
    let mut grid = Rgba::blank(10, 10).unwrap();
    for (index, c) in [
        [255, 0, 0, 128],
        [0, 255, 0, 255],
        [0, 0, 255, 64],
        [255, 255, 0, 255],
    ]
    .into_iter()
    .enumerate()
    {
        grid.paste(
            &solid(4, 4, c),
            (index % 2) as u32 * 6,
            (index / 2) as u32 * 6,
            0,
        )
        .unwrap();
    }
    let input = write_png(&engines, &grid, &work, "grid.png", cancel.clone(), 0)
        .await
        .unwrap();
    let stage = work.join("split");
    fs::create_dir(&stage).unwrap();
    execute(
        &engines,
        &[fixture_item(&input)],
        &stage,
        &Options::Split {
            columns: 2,
            rows: 2,
            padding: 2,
            order: Order::ColumnMajor,
        },
        cancel.clone(),
        3,
        |_| {},
    )
    .await
    .unwrap();
    let first = decode(
        &engines,
        &stage.join("frame-0000.png"),
        4,
        4,
        None,
        &stage,
        "verify-first",
        cancel.clone(),
        0,
    )
    .await
    .unwrap();
    assert_eq!(&first.bytes[..4], &[255, 0, 0, 128]);
    let second = decode(
        &engines,
        &stage.join("frame-0001.png"),
        4,
        4,
        None,
        &stage,
        "verify-second",
        cancel.clone(),
        0,
    )
    .await
    .unwrap();
    assert_eq!(&second.bytes[..4], &[0, 0, 255, 64]);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(stage.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(
        manifest["frames"][1]["source"],
        json!({"x":0,"y":6,"width":4,"height":4})
    );
    let first = row_pattern(0, 80);
    let second = row_pattern(60, 80);
    let p = write_png(&engines, &first, &work, "screen-a.png", cancel.clone(), 0)
        .await
        .unwrap();
    let q = write_png(&engines, &second, &work, "screen-b.png", cancel.clone(), 0)
        .await
        .unwrap();
    let inputs = [fixture_item(&p), fixture_item(&q)];
    let stage = work.join("inspection");
    fs::create_dir(&stage).unwrap();
    let inspected = inspect(&engines, &inputs, &stage, true, cancel.clone(), 4)
        .await
        .unwrap();
    assert_eq!(inspected.suggestions[0].cut, 20);
    assert!(!inspected.suggestions[0].ambiguous);
    let stage = work.join("stitch");
    fs::create_dir(&stage).unwrap();
    let joined = execute(
        &engines,
        &inputs,
        &stage,
        &Options::Stitch { cuts: vec![20] },
        cancel.clone(),
        4,
        |_| {},
    )
    .await
    .unwrap();
    let result = decode(
        &engines,
        &joined,
        64,
        140,
        None,
        &stage,
        "verify",
        cancel.clone(),
        0,
    )
    .await
    .unwrap();
    let mut expected = first.bytes;
    expected.extend_from_slice(&second.bytes[20 * 64 * 4..]);
    assert_eq!(result.bytes, expected);
    assert_eq!(fs::read(a).unwrap(), originals[0]);
    assert_eq!(fs::read(b).unwrap(), originals[1]);
    let stage = work.join("cancel");
    fs::create_dir(&stage).unwrap();
    let (_sender, cancelled) = watch::channel(true);
    assert_eq!(
        execute(
            &engines,
            &items,
            &stage,
            &Options::Pack {
                columns: 2,
                frame_width: 16,
                frame_height: 16,
                padding: 0,
                fit: Fit::Cover
            },
            cancelled,
            5,
            |_| {}
        )
        .await
        .unwrap_err(),
        engine::CANCELLED
    );
    assert_eq!(fs::read_dir(stage).unwrap().count(), 0);
    fs::remove_dir_all(work).unwrap();
}
