use std::sync::{Arc, Mutex};

use crate::media::video::decode::actor::DecodeActorHandle;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::protocol::data_channel_state::{
    request_decoder_reset_on_control_channel, request_video_keyframe_on_control_channel,
    RecoveryActionDispatcher, XbxDataChannelState,
};
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationDecision};
use crate::{
    XbxEngineMediaRuntimeStats, XbxEnginePendingRuntimeRecoveryAction,
    XbxEngineVideoEscalationObservation,
};

/**
 * recovery 动作真正落地的统一出口：
 * - 执行 data channel/control 动作
 * - 写 runtime stats escalation observation
 * - 更新恢复动作计数
 */
pub(crate) async fn apply_recovery_decision(
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    pending_runtime_action: &Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
    data_channel_state: &Arc<Mutex<XbxDataChannelState>>,
    decode_handle: Option<&Arc<DecodeActorHandle>>,
    decision: VideoEscalationDecision,
    reason_label: &str,
    observed_at_ms: f64,
) {
    let dispatcher =
        RecoveryActionDispatcher::new(data_channel_state.clone(), runtime_stats.clone());
    execute_recovery_action(
        decision.observation_id,
        decision.action,
        reason_label,
        &dispatcher,
        pending_runtime_action,
        decode_handle,
    )
    .await;
    RuntimeStatsSink::update_shared(runtime_stats.as_ref(), |stats| {
        stats.transport_recovery_epoch_at_last_escalation = stats.transport_recovery_epoch;
        stats.latest_video_escalation_observation = Some(XbxEngineVideoEscalationObservation {
            observation_id: decision.observation_id,
            reason: reason_label.to_string(),
            action: decision.action.label().to_string(),
            observed_at_ms,
        });
        bump_recovery_action_counter(stats, decision.action);
    });
}

async fn execute_recovery_action(
    observation_id: u64,
    action: RecoveryAction,
    reason_label: &str,
    dispatcher: &RecoveryActionDispatcher,
    pending_runtime_action: &Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
    decode_handle: Option<&Arc<DecodeActorHandle>>,
) {
    log_recovery_action_start(observation_id, action, reason_label, dispatcher);
    match action {
        RecoveryAction::RequestKeyframe => {
            dispatch_keyframe_recovery(observation_id, reason_label, dispatcher).await;
        }
        RecoveryAction::RequestDecoderReset => {
            dispatch_decoder_reset_recovery(
                observation_id,
                reason_label,
                dispatcher,
                decode_handle,
            )
            .await;
        }
        RecoveryAction::RequestKeyframeAndDecoderReset | RecoveryAction::StartupLowQualityRetry => {
            dispatch_keyframe_recovery(observation_id, reason_label, dispatcher).await;
            dispatch_decoder_reset_recovery(
                observation_id,
                reason_label,
                dispatcher,
                decode_handle,
            )
            .await;
        }
        RecoveryAction::WaitForBurst
        | RecoveryAction::WaitForDecoderResetBurst
        | RecoveryAction::CooldownSuppressed
        | RecoveryAction::StartupGraceSuppressed => {}
        RecoveryAction::RequestReconnectCandidate => {
            if let Ok(mut pending_action) = pending_runtime_action.lock() {
                // reconnect 候选动作单独存放在控制面，不再借 runtime stats 传命令。
                *pending_action = Some(
                    XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
                        observation_id,
                        reason: reason_label.to_string(),
                    },
                );
            }
        }
    }
}

async fn dispatch_keyframe_recovery(
    observation_id: u64,
    reason_label: &str,
    dispatcher: &RecoveryActionDispatcher,
) {
    if let Some(control_channel) = dispatcher.actionable_control_channel() {
        crate::xbx_log_info!(
            "[xbxengine][recovery] observation_id={observation_id} reason={reason_label} dispatch keyframe via actionable control channel ({})",
            dispatcher.describe_state()
        );
        match request_video_keyframe_on_control_channel(&dispatcher.runtime_state, &control_channel)
            .await
        {
            Ok(()) => {
                dispatcher.mark_keyframe_pending(false);
                crate::xbx_log_info!(
                    "[xbxengine][recovery] observation_id={observation_id} reason={reason_label} keyframe dispatch completed"
                );
            }
            Err(error) => {
                dispatcher.mark_keyframe_pending(true);
                crate::xbx_log_warn!(
                    "[xbxengine][recovery] observation_id={observation_id} reason={reason_label} keyframe dispatch failed: {error}"
                );
            }
        }
        return;
    }
    if dispatcher.already_pending_keyframe() {
        crate::xbx_log_info!(
            "[xbxengine][recovery] observation_id={observation_id} reason={reason_label} keyframe dispatch skipped because request is already pending ({})",
            dispatcher.describe_state()
        );
        return;
    }
    crate::xbx_log_info!(
        "[xbxengine][recovery] observation_id={observation_id} reason={reason_label} keyframe dispatch falling back to state-gated path ({})",
        dispatcher.describe_state()
    );
    let _ = dispatcher.request_keyframe().await;
}

async fn dispatch_decoder_reset_recovery(
    observation_id: u64,
    reason_label: &str,
    dispatcher: &RecoveryActionDispatcher,
    decode_handle: Option<&Arc<DecodeActorHandle>>,
) {
    // decoder reset 语义必须同时覆盖本地硬解会话；否则远端即便补关键帧，
    // 本地坏掉的 VideoToolbox session 仍会继续卡住。
    if let Some(decode_handle) = decode_handle {
        crate::xbx_log_info!(
            "[xbxengine][recovery] observation_id={observation_id} reason={reason_label} flushing local decode actor before decoder reset"
        );
        decode_handle.flush();
    }
    if let Some(control_channel) = dispatcher.actionable_control_channel() {
        crate::xbx_log_info!(
            "[xbxengine][recovery] observation_id={observation_id} reason={reason_label} dispatch decoder reset via actionable control channel ({})",
            dispatcher.describe_state()
        );
        match request_decoder_reset_on_control_channel(&dispatcher.runtime_state, &control_channel)
            .await
        {
            Ok(()) => {
                dispatcher.clear_recovery_pending();
                crate::xbx_log_info!(
                    "[xbxengine][recovery] observation_id={observation_id} reason={reason_label} decoder reset dispatch completed"
                );
            }
            Err(error) => {
                dispatcher.mark_decoder_reset_pending(true);
                crate::xbx_log_warn!(
                    "[xbxengine][recovery] observation_id={observation_id} reason={reason_label} decoder reset dispatch failed: {error}"
                );
            }
        }
        return;
    }
    if dispatcher.already_pending_decoder_reset() {
        crate::xbx_log_info!(
            "[xbxengine][recovery] observation_id={observation_id} reason={reason_label} decoder reset dispatch skipped because request is already pending ({})",
            dispatcher.describe_state()
        );
        return;
    }
    crate::xbx_log_info!(
        "[xbxengine][recovery] observation_id={observation_id} reason={reason_label} decoder reset dispatch falling back to state-gated path ({})",
        dispatcher.describe_state()
    );
    let _ = dispatcher.request_decoder_reset().await;
}

fn bump_recovery_action_counter(stats: &mut XbxEngineMediaRuntimeStats, action: RecoveryAction) {
    match action {
        RecoveryAction::RequestKeyframe | RecoveryAction::RequestDecoderReset => {
            stats.video_pli_request_count_total =
                stats.video_pli_request_count_total.saturating_add(1);
        }
        RecoveryAction::RequestKeyframeAndDecoderReset | RecoveryAction::StartupLowQualityRetry => {
            stats.video_pli_request_count_total =
                stats.video_pli_request_count_total.saturating_add(2);
        }
        RecoveryAction::WaitForBurst
        | RecoveryAction::WaitForDecoderResetBurst
        | RecoveryAction::CooldownSuppressed
        | RecoveryAction::StartupGraceSuppressed
        | RecoveryAction::RequestReconnectCandidate => {}
    }
}

fn log_recovery_action_start(
    observation_id: u64,
    action: RecoveryAction,
    reason_label: &str,
    dispatcher: &RecoveryActionDispatcher,
) {
    crate::xbx_log_info!(
        "[xbxengine][recovery] observation_id={observation_id} action={} reason={reason_label} ({})",
        action.label(),
        dispatcher.describe_state()
    );
}
