use super::index::{delete_index, get_index_status, rebuild_index, update_index};
use super::model::SscdModel;
use super::search::search_index;
use super::thumbnail::load_result_thumbnail;
use super::types::{DedupIndexConfig, DedupSearchRequest};
use crate::ims_pack::pack_ims;
use base64::Engine;
use image::{DynamicImage, Rgb, RgbImage};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn patterned_image(width: u32, height: u32) -> DynamicImage {
    let mut image = RgbImage::new(width, height);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let block = ((x / 24) + (y / 24)) % 2;
        let accent = ((x * 5 + y * 3) % 255) as u8;
        *pixel = if block == 0 {
            Rgb([220, accent, 40])
        } else {
            Rgb([30, 80, 200])
        };
    }
    DynamicImage::ImageRgb8(image)
}

#[test]
fn index_update_search_and_cleanup_round_trip() {
    let workspace = tempfile::tempdir().unwrap();
    let gallery = workspace.path().join("gallery");
    let app_data = workspace.path().join("app-data");
    std::fs::create_dir_all(&gallery).unwrap();

    let source_path = gallery.join("source.png");
    let resized_path = gallery.join("resized.jpg");
    let cropped_path = gallery.join("cropped.png");
    let unrelated_path = gallery.join("unrelated.png");
    let ims_path = gallery.join("container.ims.png");
    let source = patterned_image(320, 240);
    source.save(&source_path).unwrap();
    source
        .resize_exact(960, 720, image::imageops::FilterType::Lanczos3)
        .save(&resized_path)
        .unwrap();
    source
        .crop_imm(32, 24, 256, 192)
        .resize_exact(320, 240, image::imageops::FilterType::Lanczos3)
        .save(&cropped_path)
        .unwrap();
    DynamicImage::ImageRgb8(RgbImage::from_pixel(320, 240, Rgb([4, 240, 40])))
        .save(&unrelated_path)
        .unwrap();
    pack_ims(
        &[
            source_path.to_string_lossy().to_string(),
            unrelated_path.to_string_lossy().to_string(),
        ],
        0,
        &ims_path.to_string_lossy(),
        false,
    )
    .unwrap();

    let model_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("models")
        .join("sscd_disc_mixup.onnx");
    let model = Arc::new(SscdModel::load(&model_path, false, 2).unwrap());
    let config = DedupIndexConfig {
        root_path: gallery.to_string_lossy().to_string(),
        recursive: true,
        max_depth: 2,
        multi_threaded: true,
        worker_count: Some(2),
    };
    let cancel = AtomicBool::new(false);
    let first = update_index(&app_data, &config, model.clone(), &cancel, |_, _, _, _| {}).unwrap();
    assert_eq!(first.final_count, 5);
    assert_eq!(first.added, 5);
    let first_revision = get_index_status(&app_data, &gallery)
        .unwrap()
        .revision
        .expect("index revision");

    let response = search_index(
        &app_data,
        &DedupSearchRequest {
            source_path: source_path.to_string_lossy().to_string(),
            root_path: gallery.to_string_lossy().to_string(),
            threshold: 0.65,
            exclude_source: false,
        },
        model.clone(),
        &cancel,
    )
    .unwrap();
    assert!(response
        .results
        .iter()
        .any(|result| result.exact && result.filename == "source.png"));
    assert!(response
        .results
        .iter()
        .any(|result| result.filename == "resized.jpg"));
    assert!(response
        .results
        .iter()
        .any(|result| result.filename == "cropped.png"));
    let ims_result = response
        .results
        .iter()
        .find(|result| result.filename == "container.ims.png")
        .unwrap();
    assert!(ims_result.exact);
    assert_eq!(ims_result.kind, "ims_cover");
    assert!(ims_result.modified_ms > 0);
    let thumbnail = load_result_thumbnail(&app_data, &gallery, ims_result.item_id).unwrap();
    assert_eq!(thumbnail.mime, "image/jpeg");
    let thumbnail_bytes = base64::engine::general_purpose::STANDARD
        .decode(thumbnail.bytes_base64)
        .unwrap();
    assert!(!thumbnail_bytes.is_empty());
    image::load_from_memory(&thumbnail_bytes).unwrap();

    std::fs::remove_file(&unrelated_path).unwrap();
    cancel.store(true, Ordering::Relaxed);
    assert!(update_index(&app_data, &config, model.clone(), &cancel, |_, _, _, _| {}).is_err());
    let cancelled_status = get_index_status(&app_data, &gallery).unwrap();
    assert_eq!(cancelled_status.item_count, 5);
    assert_eq!(cancelled_status.revision, Some(first_revision.clone()));
    cancel.store(false, Ordering::Relaxed);
    let second = update_index(&app_data, &config, model.clone(), &cancel, |_, _, _, _| {}).unwrap();
    assert_eq!(second.removed, 1);
    assert_eq!(second.unchanged, 4);
    assert_eq!(second.final_count, 4);
    let second_revision = get_index_status(&app_data, &gallery)
        .unwrap()
        .revision
        .expect("updated revision");
    assert_ne!(second_revision, first_revision);
    let unchanged = update_index(&app_data, &config, model, &cancel, |_, _, _, _| {}).unwrap();
    assert_eq!(
        unchanged.added + unchanged.updated + unchanged.removed + unchanged.failed,
        0
    );
    assert_eq!(
        get_index_status(&app_data, &gallery).unwrap().revision,
        Some(second_revision)
    );

    let single_threaded = DedupIndexConfig {
        multi_threaded: false,
        worker_count: Some(1),
        ..config.clone()
    };
    let rebuilt = rebuild_index(
        &app_data,
        &single_threaded,
        Arc::new(SscdModel::load(&model_path, false, 1).unwrap()),
        &cancel,
        |_, _, _, _| {},
    )
    .unwrap();
    assert_eq!(rebuilt.added, 4);
    assert_eq!(rebuilt.unchanged, 0);
    assert_eq!(rebuilt.final_count, 4);

    let status = get_index_status(&app_data, &gallery).unwrap();
    assert!(status.exists);
    assert_eq!(status.item_count, 4);
    assert_eq!(status.max_depth, 2);
    assert!(delete_index(&app_data, &status.index_id).unwrap() > 0);
    assert!(!get_index_status(&app_data, &gallery).unwrap().exists);
}
