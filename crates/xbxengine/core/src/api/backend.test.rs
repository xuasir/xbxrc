use super::{
    compare_latest_only_frame_meta, presentation_value_role_from_label,
    XbxEngineLatestOnlyFrameMeta, XbxEnginePresentationValueRole,
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
