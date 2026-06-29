use super::{
    compare_latest_only_frame_meta, presentation_value_role_from_label,
    PlaceholderXbxEngineMediaBackend, XbxEngineHostVideoPresentMetrics,
    XbxEngineLatestOnlyFrameMeta, XbxEngineMediaBackend, XbxEnginePresentationValueRole,
};

fn meta(
    role: XbxEnginePresentationValueRole,
    rtp: u32,
    epoch: Option<u64>,
) -> XbxEngineLatestOnlyFrameMeta {
    XbxEngineLatestOnlyFrameMeta {
        presentation_value_role: role,
        rtp_timestamp: Some(rtp),
        recovery_epoch_tag: epoch,
        recovery_owner_rtp_timestamp: None,
        frame_seq: None,
        rendered_at_ms: 0.0,
        owner_preference_active: false,
        value_rank: 0,
    }
}

#[test]
fn compare_latest_only_prefers_higher_presentation_role() {
    let existing = meta(XbxEnginePresentationValueRole::Disposable, 100, None);
    let incoming = meta(XbxEnginePresentationValueRole::FreshAnchor, 90, None);
    assert_eq!(compare_latest_only_frame_meta(&existing, &incoming), -1);
}

#[test]
fn compare_latest_only_uses_wrap_aware_rtp_order() {
    let existing = meta(
        XbxEnginePresentationValueRole::SteadyContinuation,
        u32::MAX - 90,
        None,
    );
    let incoming = meta(
        XbxEnginePresentationValueRole::SteadyContinuation,
        120,
        None,
    );
    assert_eq!(compare_latest_only_frame_meta(&existing, &incoming), -1);
    assert_eq!(compare_latest_only_frame_meta(&incoming, &existing), 1);
}

#[test]
fn compare_latest_only_uses_wrap_aware_owner_rtp_order() {
    let mut existing = meta(
        XbxEnginePresentationValueRole::RecoveryContinuation,
        u32::MAX - 90,
        Some(7),
    );
    existing.recovery_owner_rtp_timestamp = Some(u32::MAX - 120);
    existing.owner_preference_active = true;

    let mut incoming = meta(
        XbxEnginePresentationValueRole::RecoveryContinuation,
        120,
        Some(7),
    );
    incoming.recovery_owner_rtp_timestamp = Some(90);
    incoming.owner_preference_active = true;

    assert_eq!(compare_latest_only_frame_meta(&existing, &incoming), -1);
    assert_eq!(compare_latest_only_frame_meta(&incoming, &existing), 1);
}

#[test]
fn presentation_value_role_from_label_maps_known_roles() {
    assert_eq!(
        presentation_value_role_from_label("fresh_anchor"),
        XbxEnginePresentationValueRole::FreshAnchor
    );
    assert_eq!(
        presentation_value_role_from_label("unknown-label"),
        XbxEnginePresentationValueRole::Disposable
    );
}

#[test]
fn placeholder_backend_copies_complete_host_present_metrics() {
    let mut backend = PlaceholderXbxEngineMediaBackend::default();

    backend
        .update_host_video_present_metrics(XbxEngineHostVideoPresentMetrics {
            latest_host_submit_time_ms: Some(1_000.0),
            latest_host_submit_rtp_timestamp: Some(90_001),
            latest_host_present_time_ms: Some(1_030.0),
            host_view_generation: 7,
            latest_host_view_created_at_ms: Some(900.0),
            host_mailbox_submit_epoch: 12,
            host_display_tick_epoch: 34,
            host_frame_present_epoch: 11,
            host_mailbox_enqueue_count_total: 12,
            host_mailbox_drop_count_total: 2,
            host_mailbox_overwrite_count_total: 3,
            no_pending_take_count_total: 8,
            no_pending_streak: 61,
            no_pending_max_streak: 70,
            last_displayed_frame_seq: Some(77),
            last_displayed_frame_rtp_timestamp: Some(88_888),
            last_displayed_at_ms: Some(1_030.0),
            present_fps: 58.0,
            cadence_phase: Some("starved".to_string()),
            descriptor_upload_mode: Some("d3d11-native".to_string()),
            descriptor_metal_import_count_total: 4,
            descriptor_cpu_upload_count_total: 5,
        })
        .expect("metrics update");

    let stats = backend.snapshot_runtime_stats().expect("stats");
    assert_eq!(stats.latest_host_mailbox_submit_time_ms, Some(1_000.0));
    assert_eq!(stats.latest_video_host_submit_rtp_timestamp, Some(90_001));
    assert_eq!(stats.latest_video_host_present_time_ms, Some(1_030.0));
    assert_eq!(stats.host_view_generation, 7);
    assert_eq!(stats.latest_host_view_created_at_ms, Some(900.0));
    assert_eq!(stats.host_mailbox_submit_epoch, 12);
    assert_eq!(stats.host_display_tick_epoch, 34);
    assert_eq!(stats.host_frame_present_epoch, 11);
    assert_eq!(stats.host_mailbox_enqueue_count_total, 12);
    assert_eq!(stats.host_mailbox_drop_count_total, 2);
    assert_eq!(stats.host_mailbox_overwrite_count_total, 3);
    assert_eq!(stats.host_no_pending_take_count_total, 8);
    assert_eq!(stats.host_no_pending_streak, 61);
    assert_eq!(stats.host_no_pending_max_streak, 70);
    assert_eq!(
        stats.host_no_pending_pressure_level.as_deref(),
        Some("high")
    );
    assert_eq!(stats.last_displayed_frame_seq, Some(77));
    assert_eq!(stats.last_displayed_frame_rtp_timestamp, Some(88_888));
    assert_eq!(stats.last_displayed_at_ms, Some(1_030.0));
    assert_eq!(stats.video_present_fps, 58.0);
    assert_eq!(stats.host_cadence_phase.as_deref(), Some("starved"));
    assert_eq!(
        stats.video_present_descriptor_upload_mode.as_deref(),
        Some("d3d11-native")
    );
    assert_eq!(stats.video_present_descriptor_metal_import_count_total, 4);
    assert_eq!(stats.video_present_descriptor_cpu_upload_count_total, 5);
}
