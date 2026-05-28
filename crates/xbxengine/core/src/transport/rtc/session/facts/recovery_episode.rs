#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryEpisodeStage {
    Requested,
    Sent,
    ResponseObserved,
    Decoded,
    Deferred,
    Expired,
}

/// RFC: 恢复进度统一七级语义，作为跨层事实口径。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryProgressLevel {
    WaitingResponse,
    ContinuationSeen,
    AnchorSeen,
    Decoded,
    PlaybackRecovered,
    CleanAnchorCommitted,
    DisplayStable,
}

impl RecoveryProgressLevel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::WaitingResponse => "WaitingResponse",
            Self::ContinuationSeen => "ContinuationSeen",
            Self::AnchorSeen => "AnchorSeen",
            Self::Decoded => "Decoded",
            Self::PlaybackRecovered => "PlaybackRecovered",
            Self::CleanAnchorCommitted => "CleanAnchorCommitted",
            Self::DisplayStable => "DisplayStable",
        }
    }
}

pub(crate) fn recovery_progress_level_from_str(value: &str) -> Option<RecoveryProgressLevel> {
    match value {
        "WaitingResponse" => Some(RecoveryProgressLevel::WaitingResponse),
        "ContinuationSeen" => Some(RecoveryProgressLevel::ContinuationSeen),
        "AnchorSeen" => Some(RecoveryProgressLevel::AnchorSeen),
        "Decoded" => Some(RecoveryProgressLevel::Decoded),
        "PlaybackRecovered" => Some(RecoveryProgressLevel::PlaybackRecovered),
        "CleanAnchorCommitted" => Some(RecoveryProgressLevel::CleanAnchorCommitted),
        "DisplayStable" => Some(RecoveryProgressLevel::DisplayStable),
        _ => None,
    }
}

impl RecoveryEpisodeStage {
    #[allow(dead_code)]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "Requested",
            Self::Sent => "Sent",
            Self::ResponseObserved => "ResponseObserved",
            Self::Decoded => "Decoded",
            Self::Deferred => "Deferred",
            Self::Expired => "Expired",
        }
    }
}

pub(crate) fn recovery_episode_stage_from_status(status: &str) -> Option<RecoveryEpisodeStage> {
    match status {
        "requested" => Some(RecoveryEpisodeStage::Requested),
        "sent" => Some(RecoveryEpisodeStage::Sent),
        "response-observed" | "packet-seen" => Some(RecoveryEpisodeStage::ResponseObserved),
        "decoded" => Some(RecoveryEpisodeStage::Decoded),
        "deferred" => Some(RecoveryEpisodeStage::Deferred),
        "expired-unsent" | "missed" => Some(RecoveryEpisodeStage::Expired),
        _ => None,
    }
}

pub(crate) fn recovery_progress_level_from_episode(
    status: &str,
    response_verdict: Option<&str>,
    first_video_packet_is_keyframe: Option<bool>,
    first_keyframe_packet_at_ms: Option<f64>,
    first_keyframe_decoded_at_ms: Option<f64>,
    has_current_clean_anchor: bool,
    has_display_stable: bool,
) -> Option<RecoveryProgressLevel> {
    if has_display_stable {
        return Some(RecoveryProgressLevel::DisplayStable);
    }
    if has_current_clean_anchor || response_verdict == Some("cleanAnchorCommitted") {
        return Some(RecoveryProgressLevel::CleanAnchorCommitted);
    }
    if first_keyframe_decoded_at_ms.is_some() || status == "decoded" {
        return Some(RecoveryProgressLevel::Decoded);
    }
    if first_keyframe_packet_at_ms.is_some()
        || first_video_packet_is_keyframe == Some(true)
        || matches!(status, "packet-seen")
    {
        return Some(RecoveryProgressLevel::AnchorSeen);
    }
    if matches!(status, "response-observed")
        || (first_video_packet_is_keyframe == Some(false) && response_verdict != Some("pending"))
    {
        return Some(RecoveryProgressLevel::ContinuationSeen);
    }
    if matches!(
        status,
        "requested" | "sent" | "deferred" | "failed" | "expired-unsent" | "missed"
    ) {
        return Some(RecoveryProgressLevel::WaitingResponse);
    }
    None
}

pub(crate) fn recovery_progress_missing_anchor(progress: Option<RecoveryProgressLevel>) -> bool {
    matches!(
        progress,
        Some(RecoveryProgressLevel::WaitingResponse | RecoveryProgressLevel::ContinuationSeen)
            | None
    )
}

pub(crate) fn recovery_progress_allows_decoder_reset(
    progress: Option<RecoveryProgressLevel>,
) -> bool {
    matches!(
        progress,
        Some(
            RecoveryProgressLevel::AnchorSeen
                | RecoveryProgressLevel::Decoded
                | RecoveryProgressLevel::PlaybackRecovered
                | RecoveryProgressLevel::CleanAnchorCommitted
                | RecoveryProgressLevel::DisplayStable
        )
    )
}
