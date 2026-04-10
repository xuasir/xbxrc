use crate::XbxEngineVideoTimelineObservation;

const TRANSPORT_AWAIT_UNRESOLVED_REASONS: [&str; 3] = [
    "awaitingRecoveryKeyframe",
    "awaitRecoveryKeyframe",
    "referenceChainUnrecoverable",
];

pub(crate) fn is_transport_await_unresolved_reason(reason: &str) -> bool {
    TRANSPORT_AWAIT_UNRESOLVED_REASONS.contains(&reason)
}

pub(crate) fn is_transport_await_probe_source_event(source_event: Option<&str>) -> bool {
    matches!(
        source_event,
        Some(
            "frame-await-recovery-keyframe"
                | "frame-inspection-rejected-await-keyframe"
                | "frame-inspection-rejected-trigger-recovery-keyframe"
        )
    )
}

fn is_recovery_sustaining_observation(
    chain_state: Option<&str>,
    chain_reason: Option<&str>,
) -> bool {
    matches!(chain_state, Some("sustaining-recovery"))
        || matches!(chain_reason, Some("recoverySustaining"))
}

pub(crate) fn is_ingress_waiting_keyframe(
    chain_state: Option<&str>,
    chain_reason: Option<&str>,
    source_event: Option<&str>,
) -> bool {
    if is_recovery_sustaining_observation(chain_state, chain_reason) {
        return false;
    }
    let probe_event_waiting = is_transport_await_probe_source_event(source_event)
        && !matches!(chain_state, Some("healthy"));
    matches!(chain_state, Some("broken" | "recovering"))
        || chain_reason.is_some_and(is_transport_await_unresolved_reason)
        || probe_event_waiting
}

pub(crate) fn has_unresolved_transport_await_issue_from_observation(
    timeline: &XbxEngineVideoTimelineObservation,
) -> bool {
    if is_recovery_sustaining_observation(
        Some(timeline.chain.state.as_str()),
        timeline.chain.reason.as_deref(),
    ) {
        return false;
    }
    if timeline
        .chain
        .reason
        .as_deref()
        .is_some_and(is_transport_await_unresolved_reason)
    {
        return true;
    }
    if timeline
        .frame
        .as_ref()
        .and_then(|frame| frame.close_reason.as_deref())
        .is_some_and(is_transport_await_unresolved_reason)
    {
        return true;
    }
    timeline.gap.as_ref().is_some_and(|gap| {
        !matches!(gap.state.as_str(), "resolved" | "expired")
            && timeline
                .chain
                .reason
                .as_deref()
                .is_some_and(is_transport_await_unresolved_reason)
    })
}

pub(crate) fn is_media_healthy_baseline(
    connected: bool,
    chain_healthy: bool,
    track_state: Option<&str>,
    track_video_bytes_total: Option<u64>,
    decode_age_ms: Option<f64>,
    present_age_ms: Option<f64>,
    decode_fresh_limit_ms: f64,
    present_fresh_limit_ms: f64,
    decoder_stalled: bool,
    renderer_stalled: bool,
) -> bool {
    if !connected || !chain_healthy || decoder_stalled || renderer_stalled {
        return false;
    }
    let track_attached = matches!(track_state, Some("remoteTrackAttached"));
    let has_video_bytes = track_video_bytes_total.is_some_and(|bytes| bytes > 0);
    let decode_fresh = decode_age_ms.is_some_and(|age| age <= decode_fresh_limit_ms);
    let present_fresh = present_age_ms.is_some_and(|age| age <= present_fresh_limit_ms);
    track_attached && has_video_bytes && decode_fresh && present_fresh
}
