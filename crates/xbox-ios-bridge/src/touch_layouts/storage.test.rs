use super::*;
use std::{
    fs,
    io::{Cursor, Write},
};
use zip::write::SimpleFileOptions;

struct TestDirectory(PathBuf);
impl TestDirectory {
    fn new() -> Self {
        let parent = std::env::var_os("XBXRC_TOUCH_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = parent.join(format!("xbxrc-touch-library-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn library(&self) -> Arc<XboxTouchLayoutLibrary> {
        XboxTouchLayoutLibrary::new(self.0.to_string_lossy().into()).unwrap()
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn community_layout_update_reuses_identity_and_survives_restart() {
    let directory = TestDirectory::new();
    let definition = serde_json::json!({"product_id":"GAMEPRODUCT","layouts":["controls"],"default_layout":"custom"});
    let source = serde_json::json!({"schema_version":1,"layouts":{"custom":{
        "name":"Community controls","author":"redphx","content":{"right":{"inner":[{"type":"button","action":"gamepadA"}]}}
    }}});
    let mut saved_id = String::new();
    for revision in [
        "4416b1ab07588a3244a5c02ce1fa5dfa64d8df74",
        "d66bd2925ee4518a955955b0b52890a407575285",
    ] {
        let (original, decoded) = xbox_touch_layouts::community_bundle(
            &definition,
            &[source.clone()],
            revision,
            "GAMEPRODUCT",
            "en-US",
            2.0,
        )
        .unwrap();
        let package = Arc::new(XboxOfficialTouchBundle {
            identity: types::GameIdentity {
                product_id: "GAMEPRODUCT".into(),
                title_id: "10".into(),
                name: "Game".into(),
            },
            bundle_id: format!("community:better-xcloud:{revision}"),
            original,
            decoded,
        });
        let items = directory
            .library()
            .save(package, "custom".into())
            .await
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].bundle_id, "community:better-xcloud");
        assert_eq!(items[0].bundle_version, &revision[..12]);
        if saved_id.is_empty() {
            saved_id = items[0].id.clone();
        }
        assert_eq!(saved_id, items[0].id);
    }
    let reopened = directory.library();
    let snapshot = reopened.load_layout(saved_id.clone()).await.unwrap();
    let document: serde_json::Value = serde_json::from_str(&snapshot.document_json).unwrap();
    assert_eq!(document["author"], "redphx");
    assert!(reopened.delete_layout(saved_id).await.unwrap().is_empty());
    assert_eq!(
        fs::read_dir(directory.0.join("packages")).unwrap().count(),
        0
    );
}

#[tokio::test]
async fn horizon_6_community_layout_saves_and_previews_for_the_requested_product() {
    let directory = TestDirectory::new();
    let definition: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../dev-docs/validation/2026-11-25-ios-touch-layout-download-followup/fixtures/forza-horizon-6.json"
    ))
    .unwrap();
    let source: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../iosapp/XBXRC/Resources/TouchLayouts/forza-layouts.json"
    ))
    .unwrap();
    let (original, decoded) = xbox_touch_layouts::community_bundle(
        &definition,
        &[source],
        "4416b1ab07588a3244a5c02ce1fa5dfa64d8df74",
        "HORIZON6EDITION",
        "zh-CN",
        2.0,
    )
    .unwrap();
    let bundle = Arc::new(XboxOfficialTouchBundle {
        identity: types::GameIdentity {
            product_id: "HORIZON6EDITION".into(),
            title_id: "2079757188".into(),
            name: "Forza Horizon 6".into(),
        },
        bundle_id: "community:better-xcloud:4416b1ab07588a3244a5c02ce1fa5dfa64d8df74".into(),
        original,
        decoded,
    });
    let snapshot = bundle.preview("layout-1".into()).await.unwrap();
    assert!(snapshot.issues.is_empty());
    let saved = directory
        .library()
        .save(bundle, "layout-1".into())
        .await
        .unwrap();
    assert_eq!(saved[0].product_id, "HORIZON6EDITION");
    assert_eq!(saved[0].xbox_title_id, "2079757188");
    assert!(saved[0].is_default);
    let reloaded = directory
        .library()
        .load_layout(saved[0].id.clone())
        .await
        .unwrap();
    let document: serde_json::Value = serde_json::from_str(&reloaded.document_json).unwrap();
    assert_eq!(document["author"], "redphx");
}

#[tokio::test]
async fn microsoft_sample_packages_preview_save_and_reload_with_original_assets() {
    let samples: [(&[u8], &str); 2] = [
        (
            include_bytes!("../../../../dev-docs/validation/2026-11-25-ios-touch-layout-download-followup/fixtures/official-sample-layouts.takx"),
            "standard-controller",
        ),
        (
            include_bytes!("../../../../dev-docs/validation/2026-11-25-ios-touch-layout-download-followup/fixtures/official-custom-assets.takx"),
            "custom-assets",
        ),
    ];
    for (bytes, layout_id) in samples {
        let directory = TestDirectory::new();
        let package = Arc::new(XboxOfficialTouchBundle {
            identity: types::GameIdentity {
                product_id: "GAMEPRODUCT".into(),
                title_id: "10".into(),
                name: "Microsoft sample".into(),
            },
            bundle_id: layout_id.into(),
            original: bytes.to_vec(),
            decoded: decode_bundle(bytes, "zh-CN", 2.0).unwrap(),
        });
        for choice in package.choices() {
            let preview = package.preview(choice.id).await.unwrap();
            assert!(preview.issues.is_empty());
            assert!(
                serde_json::from_str::<serde_json::Value>(&preview.document_json).unwrap()
                    ["content"]
                    .is_object()
            );
        }
        let preview = package.preview(layout_id.into()).await.unwrap();
        let saved = directory
            .library()
            .save(package, layout_id.into())
            .await
            .unwrap();
        let reloaded = directory
            .library()
            .load_layout(saved[0].id.clone())
            .await
            .unwrap();
        assert!(saved[0].is_default);
        assert_eq!(preview.document_json, reloaded.document_json);
        assert_eq!(preview.assets.len(), reloaded.assets.len());
        for (before, after) in preview.assets.iter().zip(&reloaded.assets) {
            assert_eq!(before.id, after.id);
            assert_eq!(before.data, after.data);
        }
    }
}

fn bundle(action: &str) -> Arc<XboxOfficialTouchBundle> {
    bundle_with_asset(action, None)
}

fn bundle_with_asset(action: &str, asset: Option<&[u8]>) -> Arc<XboxOfficialTouchBundle> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let manifest =
        br#"{"version":"1.0.0.0","layouts":"layouts","assets":"assets","defaultLayout":"gameplay"}"#.to_vec();
    let mut document =
        serde_json::json!({"content":{"right":{"inner":[{"type":"button","action":action}]}}});
    if asset.is_some() {
        document["content"]["right"]["inner"][0]["styles"] =
            serde_json::json!({"default":{"faceImage":{"type":"asset","value":"fire"}}});
    }
    let layout = serde_json::to_vec(&document).unwrap();
    let mut files = vec![
        ("takxmanifest.json", manifest),
        ("layouts/neutral/gameplay.json", layout.clone()),
        ("layouts/neutral/driving.json", layout),
    ];
    if let Some(bytes) = asset {
        files.push(("assets/neutral/@1.0x/fire.png", bytes.to_vec()));
    }
    for (name, bytes) in files {
        zip.start_file(name, SimpleFileOptions::default()).unwrap();
        zip.write_all(&bytes).unwrap();
    }
    let original = zip.finish().unwrap().into_inner();
    let decoded = decode_bundle(&original, "en-US", 2.0).unwrap();
    Arc::new(XboxOfficialTouchBundle {
        identity: types::GameIdentity {
            product_id: "GAMEPRODUCT".into(),
            title_id: "10".into(),
            name: "Game".into(),
        },
        bundle_id: "official-package".into(),
        original,
        decoded,
    })
}

#[tokio::test]
async fn saves_multiple_layouts_and_reloads_shared_bundle_after_restart() {
    let directory = TestDirectory::new();
    let library = directory.library();
    let draft = bundle("gamepadA");
    library
        .save(draft.clone(), "gameplay".into())
        .await
        .unwrap();
    let records = library.save(draft, "driving".into()).await.unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].content_hash, records[1].content_hash);
    let restarted = directory.library();
    assert_eq!(restarted.saved_layouts().await.unwrap().len(), 2);
    assert_eq!(
        fs::read_dir(directory.0.join("packages")).unwrap().count(),
        1
    );
    let snapshot = restarted.load_layout(records[0].id.clone()).await.unwrap();
    assert!(snapshot.document_json.contains("gamepadA"));
    assert_eq!(snapshot.id, records[0].id);
}

#[tokio::test]
async fn duplicate_save_is_idempotent_and_update_keeps_stable_selection_identity() {
    let directory = TestDirectory::new();
    let library = directory.library();
    let draft = bundle("gamepadA");
    let first = library
        .save(draft.clone(), "gameplay".into())
        .await
        .unwrap()
        .remove(0);
    let duplicate = library.save(draft, "gameplay".into()).await.unwrap();
    assert_eq!(duplicate.len(), 1);
    assert_eq!(first.id, duplicate[0].id);
    assert_eq!(first.saved_at_ms, duplicate[0].saved_at_ms);
    let updated = library
        .save(bundle("gamepadB"), "gameplay".into())
        .await
        .unwrap()
        .remove(0);
    assert_eq!(first.id, updated.id);
    assert_ne!(first.revision, updated.revision);
    assert!(library
        .load_layout(updated.id)
        .await
        .unwrap()
        .document_json
        .contains("gamepadB"));
    assert_eq!(
        fs::read_dir(directory.0.join("packages")).unwrap().count(),
        1
    );
}

#[tokio::test]
async fn deleting_one_layout_preserves_shared_resources_and_last_delete_reclaims_them() {
    let directory = TestDirectory::new();
    let library = directory.library();
    let draft = bundle("gamepadA");
    library
        .save(draft.clone(), "gameplay".into())
        .await
        .unwrap();
    let records = library.save(draft, "driving".into()).await.unwrap();
    let package = directory.0.join("packages").join(&records[0].content_hash);
    let deleted_compiled = package
        .join("compiled")
        .join(format!("{}.json", records[0].revision));
    let remaining = library.delete_layout(records[0].id.clone()).await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert!(!deleted_compiled.exists());
    assert!(package.join("original.takx").exists());
    assert!(library.load_layout(remaining[0].id.clone()).await.is_ok());
    library
        .delete_layout(remaining[0].id.clone())
        .await
        .unwrap();
    assert!(library.saved_layouts().await.unwrap().is_empty());
    assert_eq!(
        fs::read_dir(directory.0.join("packages")).unwrap().count(),
        0
    );
}

#[tokio::test]
async fn reopening_library_reclaims_orphaned_packages_snapshots_and_pending_writes() {
    let directory = TestDirectory::new();
    let library = directory.library();
    let saved = library
        .save(bundle("gamepadA"), "gameplay".into())
        .await
        .unwrap()
        .remove(0);
    let orphan = directory.0.join("packages").join("a".repeat(64));
    fs::create_dir_all(orphan.join("expanded")).unwrap();
    fs::write(orphan.join("original.takx"), "interrupted-save").unwrap();
    let compiled = directory
        .0
        .join("packages")
        .join(&saved.content_hash)
        .join("compiled")
        .join(format!("{}.json", "b".repeat(64)));
    fs::write(&compiled, "old-revision").unwrap();
    let pending = directory
        .0
        .join(format!(".pending-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&pending).unwrap();
    fs::write(pending.join("original.takx"), "interrupted-write").unwrap();
    let pending_index = directory
        .0
        .join(format!("index.pending-{}", uuid::Uuid::new_v4()));
    fs::write(&pending_index, "interrupted-index").unwrap();
    assert_eq!(directory.library().saved_layouts().await.unwrap().len(), 1);
    for path in [orphan, compiled, pending, pending_index] {
        assert!(!path.exists(), "residual resource: {}", path.display());
    }
    assert!(library.load_layout(saved.id).await.is_ok());
}

#[cfg(unix)]
#[tokio::test]
async fn failed_delete_cleanup_is_reported_and_retried_when_listing_layouts() {
    use std::os::unix::fs::PermissionsExt;
    let directory = TestDirectory::new();
    let library = directory.library();
    let saved = library
        .save(bundle("gamepadA"), "gameplay".into())
        .await
        .unwrap()
        .remove(0);
    let packages = directory.0.join("packages");
    let permissions = fs::metadata(&packages).unwrap().permissions();
    fs::set_permissions(&packages, fs::Permissions::from_mode(0o500)).unwrap();
    let result = library.delete_layout(saved.id).await;
    fs::set_permissions(&packages, permissions).unwrap();
    assert!(result.unwrap_err().to_string().contains("清理失败"));
    assert!(library.saved_layouts().await.unwrap().is_empty());
    assert_eq!(fs::read_dir(packages).unwrap().count(), 0);
}

#[tokio::test]
async fn corrupt_compiled_layout_fails_without_affecting_other_layouts() {
    let directory = TestDirectory::new();
    let library = directory.library();
    let draft = bundle("gamepadA");
    library
        .save(draft.clone(), "gameplay".into())
        .await
        .unwrap();
    let records = library.save(draft, "driving".into()).await.unwrap();
    fs::write(
        directory
            .0
            .join("packages")
            .join(&records[0].content_hash)
            .join("compiled")
            .join(format!("{}.json", records[0].revision)),
        b"corrupt",
    )
    .unwrap();
    assert!(library.load_layout(records[0].id.clone()).await.is_err());
    assert!(library.load_layout(records[1].id.clone()).await.is_ok());
}

#[tokio::test]
async fn invalid_index_entry_is_ignored_and_valid_entries_remain() {
    let directory = TestDirectory::new();
    let library = directory.library();
    let record = library
        .save(bundle("gamepadA"), "gameplay".into())
        .await
        .unwrap()
        .remove(0);
    let path = directory.0.join("index.json");
    let mut index: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut bad = serde_json::to_value(&record).unwrap();
    bad["content_hash"] = serde_json::json!("../escape");
    index["layouts"].as_array_mut().unwrap().push(bad);
    fs::write(path, serde_json::to_vec(&index).unwrap()).unwrap();
    let entries = directory.library().saved_layouts().await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, record.id);
}

#[tokio::test]
async fn failed_save_keeps_previous_layout_and_pending_directories_are_cleaned() {
    let directory = TestDirectory::new();
    let library = directory.library();
    let records = library
        .save(bundle("gamepadA"), "gameplay".into())
        .await
        .unwrap();
    assert!(library
        .save(bundle("gamepadB"), "missing".into())
        .await
        .is_err());
    assert!(library
        .load_layout(records[0].id.clone())
        .await
        .unwrap()
        .document_json
        .contains("gamepadA"));
    assert!(fs::read_dir(&directory.0)
        .unwrap()
        .flatten()
        .all(|entry| !entry.file_name().to_string_lossy().starts_with(".pending")));
}

#[tokio::test]
async fn applied_layout_recency_is_persisted() {
    let directory = TestDirectory::new();
    let library = directory.library();
    let record = library
        .save(bundle("gamepadA"), "gameplay".into())
        .await
        .unwrap()
        .remove(0);
    assert_eq!(record.last_used_at_ms, 0);
    library.mark_used(record.id.clone()).await.unwrap();
    assert!(directory.library().saved_layouts().await.unwrap()[0].last_used_at_ms > 0);
}

#[test]
fn library_requires_absolute_root() {
    assert!(XboxTouchLayoutLibrary::new("relative/path".into()).is_err());
}

#[tokio::test]
async fn corrupted_asset_uses_fallback_and_same_version_save_repairs_shared_resources() {
    let directory = TestDirectory::new();
    let library = directory.library();
    let draft = bundle_with_asset("gamepadA", Some(&[1, 2, 3]));
    library
        .save(draft.clone(), "gameplay".into())
        .await
        .unwrap();
    let records = library.save(draft.clone(), "driving".into()).await.unwrap();
    let asset = directory
        .0
        .join("packages")
        .join(&records[0].content_hash)
        .join("expanded/assets/neutral/@1.0x/fire.png");
    fs::write(asset, [3, 2, 1]).unwrap();
    let degraded = library.load_layout(records[0].id.clone()).await.unwrap();
    assert!(degraded.assets.is_empty());
    assert!(!degraded.issues.is_empty());
    library.save(draft, "gameplay".into()).await.unwrap();
    for record in records {
        assert_eq!(
            library.load_layout(record.id).await.unwrap().assets[0].data,
            [1, 2, 3]
        );
    }
}

#[tokio::test]
async fn failed_compiled_write_reclaims_new_package_and_preserves_previous_index() {
    let directory = TestDirectory::new();
    let library = directory.library();
    let previous = library
        .save(bundle("gamepadA"), "gameplay".into())
        .await
        .unwrap()
        .remove(0);
    let draft = bundle("gamepadB");
    let new_package = directory
        .0
        .join("packages")
        .join(xbox_touch_layouts::content_hash(&draft.original));
    fs::create_dir_all(&new_package).unwrap();
    fs::write(new_package.join("compiled"), "write-blocker").unwrap();
    assert!(library.save(draft, "gameplay".into()).await.is_err());
    assert!(!new_package.exists());
    assert_eq!(library.saved_layouts().await.unwrap()[0].id, previous.id);
    assert!(library
        .load_layout(previous.id)
        .await
        .unwrap()
        .document_json
        .contains("gamepadA"));
}

#[tokio::test]
async fn protocol_cache_regenerates_without_changing_saved_identity_or_revision() {
    let directory = TestDirectory::new();
    let library = directory.library();
    let record = library
        .save(bundle("gamepadA"), "gameplay".into())
        .await
        .unwrap()
        .remove(0);
    let file = directory
        .0
        .join("packages")
        .join(&record.content_hash)
        .join("compiled")
        .join(format!(
            "{}.{}.ir.json",
            record.revision,
            xbox_touch_layouts::protocol_revision()
        ));
    assert!(file.exists());
    fs::write(&file, "old-or-damaged-protocol-cache").unwrap();
    let loaded = library.load_layout(record.id.clone()).await.unwrap();
    let ir: serde_json::Value = serde_json::from_str(&loaded.document_json).unwrap();
    assert_eq!(
        ir["protocolRevision"],
        xbox_touch_layouts::protocol_revision()
    );
    assert_eq!(ir["isRunnable"], true);
    let after = library.saved_layouts().await.unwrap().remove(0);
    assert_eq!(after.id, record.id);
    assert_eq!(after.revision, record.revision);
    assert!(serde_json::from_slice::<serde_json::Value>(&fs::read(file).unwrap()).is_ok());
}
