//! Keyframe 请求 episode 生命周期：与 `status` / `response_verdict` 并行，供压制与诊断统一读取。

use crate::XbxEngineKeyframeRequestEpisodeObservation;

/// 与产品语义对齐的生命周期阶段（字符串落盘在 `lifecycle_phase`）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum KeyframeRequestLifecyclePhase {
    Requesting,
    Sent,
    PacketSeen,
    Decoded,
    Success,
    Failure,
}

impl KeyframeRequestLifecyclePhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Requesting => "requesting",
            Self::Sent => "sent",
            Self::PacketSeen => "packetSeen",
            Self::Decoded => "decoded",
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }
}

pub(crate) fn derive_keyframe_lifecycle_phase(
    transport_recovery_epoch: u64,
    video_anchor_clean_epoch: Option<u64>,
    video_anchor_clean_observed_at_ms: Option<f64>,
    episode: &XbxEngineKeyframeRequestEpisodeObservation,
) -> KeyframeRequestLifecyclePhase {
    let status = episode.status.as_str();
    let verdict = episode.response_verdict.as_deref();

    if status == "succeeded" || verdict == Some("cleanAnchorCommitted") {
        return KeyframeRequestLifecyclePhase::Success;
    }

    // `decoded` 必须早于对 `verdict == missed` 的失败判定，否则 timeout 与 decode 竞态会永久判 Failure。
    if status == "decoded" {
        let anchor_ok = video_anchor_clean_epoch == Some(transport_recovery_epoch)
            && video_anchor_clean_observed_at_ms.is_some();
        if anchor_ok {
            return KeyframeRequestLifecyclePhase::Success;
        }
        return KeyframeRequestLifecyclePhase::Decoded;
    }

    if matches!(status, "deferred" | "failed" | "missed" | "expired-unsent")
        || matches!(
            verdict,
            Some("transportDeferred" | "transportFailed" | "missed" | "unsentExpired")
        )
    {
        return KeyframeRequestLifecyclePhase::Failure;
    }

    if matches!(status, "packet-seen" | "response-observed") {
        return KeyframeRequestLifecyclePhase::PacketSeen;
    }

    if status == "sent" {
        return KeyframeRequestLifecyclePhase::Sent;
    }

    if status == "requested" {
        return KeyframeRequestLifecyclePhase::Requesting;
    }

    if episode.sent_at_ms.is_some() {
        KeyframeRequestLifecyclePhase::Sent
    } else {
        KeyframeRequestLifecyclePhase::Requesting
    }
}

pub(crate) fn apply_keyframe_episode_lifecycle_field(
    transport_recovery_epoch: u64,
    video_anchor_clean_epoch: Option<u64>,
    video_anchor_clean_observed_at_ms: Option<f64>,
    episode: &mut XbxEngineKeyframeRequestEpisodeObservation,
) {
    let phase = derive_keyframe_lifecycle_phase(
        transport_recovery_epoch,
        video_anchor_clean_epoch,
        video_anchor_clean_observed_at_ms,
        episode,
    );
    episode.lifecycle_phase = Some(phase.as_str().to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::XbxEngineKeyframeRequestEpisodeObservation;

    #[test]
    fn derive_prefers_decoded_over_missed_verdict_when_anchor_clean() {
        let episode = XbxEngineKeyframeRequestEpisodeObservation {
            status: "decoded".to_string(),
            response_verdict: Some("missed".to_string()),
            ..Default::default()
        };
        let phase = derive_keyframe_lifecycle_phase(7, Some(7), Some(50.0), &episode);
        assert_eq!(phase, KeyframeRequestLifecyclePhase::Success);
    }

    #[test]
    fn derive_decoded_without_anchor_match_stays_decoded_despite_missed_verdict() {
        let episode = XbxEngineKeyframeRequestEpisodeObservation {
            status: "decoded".to_string(),
            response_verdict: Some("missed".to_string()),
            ..Default::default()
        };
        let phase = derive_keyframe_lifecycle_phase(7, None, None, &episode);
        assert_eq!(phase, KeyframeRequestLifecyclePhase::Decoded);
    }
}
