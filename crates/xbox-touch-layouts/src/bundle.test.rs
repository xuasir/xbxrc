use super::*;
use serde_json::{json, Value};
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;

fn archive(files: Vec<(&str, Vec<u8>)>) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in files {
        writer
            .start_file(
                name,
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(&data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn manifest() -> Vec<u8> {
    include_bytes!(
        "../../../dev-docs/validation/2026-11-12-tak-layout-and-engine/examples/manifest.json"
    )
    .to_vec()
}

fn layout(action: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "$schema": "https://raw.githubusercontent.com/microsoft/xbox-game-streaming-tools/main/touch-adaptation-kit/schemas/layout/latest/layout.json",
        "displayName": "Gameplay", "content": {"right":{"inner":[{"type":"button","action":action}]}}
    })).unwrap()
}

fn base_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("takxmanifest.json", manifest()),
        (
            "context.json",
            br#"{"state":{"enabled":false},"definitions":{"action":"gamepadA"}}"#.to_vec(),
        ),
        ("layouts/neutral/gameplay.json", layout("gamepadA")),
    ]
}

#[test]
fn decodes_official_manifest_default_and_multiple_layouts() {
    let mut files = base_files();
    files.push(("layouts/neutral/driving.json", layout("gamepadB")));
    let bundle = decode_bundle(&archive(files), "zh-Hans", 3.0).unwrap();
    assert_eq!(bundle.version, "1.0.0.0");
    assert_eq!(bundle.layouts.len(), 2);
    assert!(
        bundle
            .layouts
            .iter()
            .find(|layout| layout.id == "gameplay")
            .unwrap()
            .is_default
    );
    assert!(bundle.layouts.iter().all(|layout| layout.issues.is_empty()));
}

#[test]
fn decodes_manifest_with_short_service_version() {
    let mut files = base_files();
    let mut manifest: Value = serde_json::from_slice(&files[0].1).unwrap();
    manifest["version"] = json!("1.0");
    files[0].1 = serde_json::to_vec(&manifest).unwrap();
    assert_eq!(
        decode_bundle(&archive(files), "en-US", 2.0)
            .unwrap()
            .version,
        "1.0"
    );
}

#[test]
fn decodes_microsoft_sample_packages_with_bom_layouts_and_original_assets() {
    let samples: [(&[u8], usize, &str, usize); 2] = [
        (
            include_bytes!("../../../dev-docs/validation/2026-11-25-ios-touch-layout-download-followup/fixtures/official-sample-layouts.takx"),
            6,
            "standard-controller",
            0,
        ),
        (
            include_bytes!("../../../dev-docs/validation/2026-11-25-ios-touch-layout-download-followup/fixtures/official-custom-assets.takx"),
            1,
            "custom-assets",
            9,
        ),
    ];
    for (bytes, layout_count, default_id, asset_count) in samples {
        let bundle = decode_bundle(bytes, "zh-CN", 2.0).unwrap();
        assert_eq!(bundle.layouts.len(), layout_count);
        assert!(bundle.layouts.iter().all(|layout| layout.issues.is_empty()));
        let default = bundle
            .layouts
            .iter()
            .find(|layout| layout.is_default)
            .unwrap();
        assert_eq!(default.id, default_id);
        assert_eq!(default.assets.len(), asset_count);
        for asset in &default.assets {
            assert!(asset.path.starts_with("assets/neutral/@2.0x/"));
            let original = &bundle.files[&asset.path];
            assert!(original.starts_with(b"\x89PNG\r\n\x1a\n"));
            assert_eq!(asset.content_hash, crate::content_hash(original));
        }
    }
}

#[test]
fn decodes_utf8_bom_and_unicode_edges_in_manifest_context_and_layout() {
    let mut files = base_files();
    files[2].1 = serde_json::to_vec(&json!({
        "displayName":"游戏布局",
        "content":{"right":{"inner":[{"type":"button","action":{
            "$ref":"../../context.json#/definitions/action"
        }}]}}
    }))
    .unwrap();
    for (_, bytes) in &mut files {
        let mut decorated = "\u{FEFF}\u{00A0}\r\n".as_bytes().to_vec();
        decorated.extend_from_slice(bytes);
        decorated.extend_from_slice("\n\u{3000}\u{FEFF}".as_bytes());
        *bytes = decorated;
    }
    let bundle = decode_bundle(&archive(files), "zh-CN", 2.0).unwrap();
    assert_eq!(bundle.layouts[0].name, "游戏布局");
    assert_eq!(
        bundle.layouts[0].document["content"]["right"]["inner"][0]["action"],
        "gamepadA"
    );
    assert!(bundle.layouts[0].issues.is_empty());
}

#[test]
fn malformed_layout_keeps_valid_layouts_available_with_a_local_diagnostic() {
    let mut files = base_files();
    files.push(("layouts/neutral/broken.json", b"{broken".to_vec()));
    let bundle = decode_bundle(&archive(files), "en-US", 2.0).unwrap();
    let broken = bundle
        .layouts
        .iter()
        .find(|layout| layout.id == "broken")
        .unwrap();
    assert!(broken.document.is_null());
    assert!(broken.issues[0].contains("行 1"));
    assert!(bundle
        .layouts
        .iter()
        .find(|layout| layout.id == "gameplay")
        .unwrap()
        .issues
        .is_empty());
}

#[test]
fn locale_overrides_neutral_and_falls_back_between_regions() {
    let mut files = base_files();
    files.push(("layouts/en/gameplay.json", layout("gamepadB")));
    files.push(("layouts/en/en-us/gameplay.json", layout("gamepadX")));
    let bytes = archive(files);
    let action = |locale: &str| {
        decode_bundle(&bytes, locale, 2.0).unwrap().layouts[0].document["content"]["right"]["inner"]
            [0]["action"]
            .clone()
    };
    assert_eq!(action("en-US"), json!("gamepadX"));
    assert_eq!(action("en-GB"), json!("gamepadB"));
    assert_eq!(action("ja-JP"), json!("gamepadA"));
}

#[test]
fn expands_context_default_and_internal_definitions() {
    let mut files = base_files();
    files[2].1 = serde_json::to_vec(&json!({
        "definitions":{"button":{"type":"button","action":{"$ref":"../../context.json#/definitions/action"},"enabled":{"$ref":"../../context.json#/state/enabled"}}},
        "content":{"right":{"inner":[{"$ref":"#/definitions/button"}]}}
    })).unwrap();
    let bundle = decode_bundle(&archive(files), "en-US", 2.0).unwrap();
    let button = &bundle.layouts[0].document["content"]["right"]["inner"][0];
    assert_eq!(button["action"], "gamepadA");
    assert_eq!(button["enabled"], false);
    assert!(bundle.layouts[0].issues.is_empty());
}

#[test]
fn selects_localized_assets_at_nearest_display_density() {
    let mut files = base_files();
    let mut document: Value = serde_json::from_slice(&files[2].1).unwrap();
    document["content"]["right"]["inner"][0]["styles"] =
        json!({"default":{"faceImage":{"type":"asset","value":"fire"}}});
    files[2].1 = serde_json::to_vec(&document).unwrap();
    files.extend([
        ("assets/neutral/@1.0x/fire.png", vec![1]),
        ("assets/neutral/@3.0x/fire.png", vec![3]),
        ("assets/en/@2.0x/fire.png", vec![2]),
    ]);
    let bytes = archive(files);
    let localized = decode_bundle(&bytes, "en-US", 3.0).unwrap();
    assert_eq!(
        localized.layouts[0].assets[0].path,
        "assets/en/@2.0x/fire.png"
    );
    let neutral = decode_bundle(&bytes, "ja-JP", 3.0).unwrap();
    assert_eq!(
        neutral.layouts[0].assets[0].path,
        "assets/neutral/@3.0x/fire.png"
    );
    assert_eq!(
        neutral.layouts[0].document["content"]["right"]["inner"][0]["styles"]["default"]
            ["faceImage"]["value"],
        "assets/neutral/@3.0x/fire.png"
    );
}

#[test]
fn bad_reference_keeps_other_layouts_available() {
    let mut files = base_files();
    files.push((
        "layouts/neutral/broken.json",
        br##"{"content":{"right":{"inner":[{"$ref":"#/missing"}]}}}"##.to_vec(),
    ));
    let bundle = decode_bundle(&archive(files), "en", 1.0).unwrap();
    assert!(
        bundle
            .layouts
            .iter()
            .find(|value| value.id == "broken")
            .unwrap()
            .issues
            .len()
            > 0
    );
    assert!(bundle
        .layouts
        .iter()
        .find(|value| value.id == "gameplay")
        .unwrap()
        .issues
        .is_empty());
}

#[test]
fn circular_and_external_references_are_diagnosed() {
    for reference in [
        "#/content",
        "../../../outside.json#/x",
        "https://example.com/layout.json#/x",
    ] {
        let mut files = base_files();
        files[2].1 = serde_json::to_vec(&json!({"content":{"value":{"$ref":reference}}})).unwrap();
        let bundle = decode_bundle(&archive(files), "en-US", 2.0).unwrap();
        assert!(!bundle.layouts[0].issues.is_empty(), "{reference}");
    }
}

#[test]
fn archive_rejects_traversal_absolute_and_conflicting_paths() {
    for path in [
        "../escape.json",
        "/escape.json",
        "C:/escape.json",
        "a\\escape.json",
    ] {
        let mut files = base_files();
        files.push((path, vec![1]));
        assert!(decode_bundle(&archive(files), "en", 1.0).is_err(), "{path}");
    }
    let mut files = base_files();
    files.push(("LAYOUTS/neutral/GAMEPLAY.json", layout("gamepadB")));
    assert!(decode_bundle(&archive(files), "en", 1.0).is_err());
}

#[test]
fn rejects_invalid_manifest_and_accepts_a_missing_optional_context() {
    let mut files = base_files();
    files[0].1 = br#"{"layouts":"layouts","version":"invalid"}"#.to_vec();
    assert!(decode_bundle(&archive(files), "en", 1.0).is_err());
    let mut files = base_files();
    files.remove(1);
    assert!(decode_bundle(&archive(files.clone()), "en", 1.0)
        .unwrap()
        .layouts[0]
        .issues
        .is_empty());
    files[1].1 = br##"{"content":{"value":{"$ref":"../../context.json#/state"}}}"##.to_vec();
    assert!(
        !decode_bundle(&archive(files), "en", 1.0).unwrap().layouts[0]
            .issues
            .is_empty()
    );
}

#[test]
fn rejects_json_and_archive_budgets() {
    assert!(decode_bundle(&vec![0; MAX_BUNDLE_BYTES + 1], "en", 1.0).is_err());
    let mut files = base_files();
    files[2].1 = vec![b' '; MAX_JSON_BYTES + 1];
    assert!(decode_bundle(&archive(files), "en", 1.0).is_err());
    assert!(decode_bundle(b"invalid zip", "en", 1.0).is_err());
    assert!(decode_bundle(&archive(base_files()), "../../en", 1.0).is_err());
}

#[test]
fn content_identity_is_stable_and_changes_with_bytes() {
    assert_eq!(content_hash(b"one"), content_hash(b"one"));
    assert_ne!(content_hash(b"one"), content_hash(b"two"));
    assert_eq!(content_hash(b"one").len(), 64);
}
