use std::sync::Mutex;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};
use crate::transport::rtc::recovery::runtime_state::unix_now_ms;
use crate::XbxEngineMediaRuntimeStats;

const WAIT_KEYFRAME_REPEAT_SUPPRESS_MS: f64 = 260.0;
const IDLE_TIMEOUT_REPEAT_SUPPRESS_MS: f64 = 360.0;

pub(crate) fn resolve_recent_repeat_suppression(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    reason: &VideoEscalationReason,
) -> Option<RecoveryAction> {
    let now_ms = unix_now_ms();
    let (escalation, has_new_transport_recovery_epoch) =
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            let escalation = stats.latest_video_escalation_observation.clone()?;
            let has_new_transport_recovery_epoch =
                stats.transport_recovery_epoch > stats.transport_recovery_epoch_at_last_escalation;
            Some((escalation, has_new_transport_recovery_epoch))
        })
        .flatten()?;
    let elapsed_ms = now_ms - escalation.observed_at_ms;
    if elapsed_ms < 0.0 {
        return None;
    }

    match reason {
        VideoEscalationReason::WaitKeyframe => {
            let same_wait_keyframe_chain = matches!(
                escalation.reason.as_str(),
                "waitKeyframe"
                    | "ingressWaitKeyframe"
                    | "ingressFrameAbandoned"
                    | "transportAwaitRecoveryKeyframe"
            );
            let active_recovery_action = matches!(
                escalation.action.as_str(),
                "requestKeyframe"
                    | "requestDecoderReset"
                    | "requestKeyframe+decoderReset"
                    | "requestKeyframe+decoderReset(startupLowQualityRetry)"
            );
            if same_wait_keyframe_chain
                && active_recovery_action
                && !has_new_transport_recovery_epoch
                && elapsed_ms <= WAIT_KEYFRAME_REPEAT_SUPPRESS_MS
            {
                return Some(RecoveryAction::CooldownSuppressed);
            }
        }
        VideoEscalationReason::DisplaySupplyCritical => {
            let same_local_display_chain = escalation.reason == "displaySupplyCritical";
            let decoder_reset_inflight = matches!(
                escalation.action.as_str(),
                "requestDecoderReset"
                    | "requestKeyframe+decoderReset"
                    | "requestKeyframe+decoderReset(startupLowQualityRetry)"
            );
            if same_local_display_chain
                && decoder_reset_inflight
                && !has_new_transport_recovery_epoch
                && elapsed_ms <= IDLE_TIMEOUT_REPEAT_SUPPRESS_MS
            {
                return Some(RecoveryAction::CooldownSuppressed);
            }
        }
        VideoEscalationReason::AdapterIdleTimeout => {
            let same_idle_chain = escalation.reason == "adapterIdleTimeout";
            let decoder_reset_inflight = matches!(
                escalation.action.as_str(),
                "requestDecoderReset"
                    | "requestKeyframe+decoderReset"
                    | "requestKeyframe+decoderReset(startupLowQualityRetry)"
            );
            if same_idle_chain
                && decoder_reset_inflight
                && !has_new_transport_recovery_epoch
                && elapsed_ms <= IDLE_TIMEOUT_REPEAT_SUPPRESS_MS
            {
                return Some(RecoveryAction::CooldownSuppressed);
            }
        }
        _ => {}
    }

    None
}
