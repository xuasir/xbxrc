use std::sync::{Arc, Mutex};

use crate::media::video::decode::actor::DecodeActorHandle;
use crate::transport::webrtc::data_channel::{
    recovery_requests_ready, request_decoder_reset_from_state,
    request_decoder_reset_on_control_channel, request_video_keyframe_from_state,
    request_video_keyframe_on_control_channel, XbxDataChannelState,
};
use crate::transport::webrtc::escalation::{RecoveryAction, VideoEscalationDecision};
use crate::{
    XbxEngineMediaRuntimeStats, XbxEnginePendingRuntimeRecoveryAction,
    XbxEngineVideoEscalationObservation,
};
use webrtc::data_channel::{data_channel_state::RTCDataChannelState, RTCDataChannel};

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
    execute_recovery_action(
        decision.observation_id,
        decision.action,
        reason_label,
        runtime_stats,
        pending_runtime_action,
        data_channel_state,
        decode_handle,
    )
    .await;
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.latest_video_escalation_observation = Some(XbxEngineVideoEscalationObservation {
            observation_id: decision.observation_id,
            reason: reason_label.to_string(),
            action: decision.action.label().to_string(),
            observed_at_ms,
        });
        bump_recovery_action_counter(&mut stats, decision.action);
    }
}

async fn execute_recovery_action(
    observation_id: u64,
    action: RecoveryAction,
    reason_label: &str,
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    pending_runtime_action: &Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
    data_channel_state: &Arc<Mutex<XbxDataChannelState>>,
    decode_handle: Option<&Arc<DecodeActorHandle>>,
) {
    match action {
        RecoveryAction::RequestKeyframe => {
            dispatch_keyframe_recovery(runtime_stats, data_channel_state).await;
        }
        RecoveryAction::RequestDecoderReset => {
            dispatch_decoder_reset_recovery(runtime_stats, data_channel_state, decode_handle).await;
        }
        RecoveryAction::RequestKeyframeAndDecoderReset | RecoveryAction::StartupLowQualityRetry => {
            dispatch_keyframe_recovery(runtime_stats, data_channel_state).await;
            dispatch_decoder_reset_recovery(runtime_stats, data_channel_state, decode_handle).await;
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
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    data_channel_state: &Arc<Mutex<XbxDataChannelState>>,
) {
    if let Some(control_channel) = actionable_control_channel(data_channel_state) {
        match request_video_keyframe_on_control_channel(data_channel_state, &control_channel).await
        {
            Ok(()) => {
                if let Ok(mut state) = data_channel_state.lock() {
                    state.pending_keyframe_request = false;
                }
            }
            Err(_) => {
                if let Ok(mut state) = data_channel_state.lock() {
                    state.pending_keyframe_request = true;
                }
            }
        }
        return;
    }
    if already_pending_keyframe(data_channel_state) {
        return;
    }
    let _ = request_video_keyframe_from_state(data_channel_state, runtime_stats).await;
}

async fn dispatch_decoder_reset_recovery(
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    data_channel_state: &Arc<Mutex<XbxDataChannelState>>,
    decode_handle: Option<&Arc<DecodeActorHandle>>,
) {
    // decoder reset 语义必须同时覆盖本地硬解会话；否则远端即便补关键帧，
    // 本地坏掉的 VideoToolbox session 仍会继续卡住。
    if let Some(decode_handle) = decode_handle {
        decode_handle.flush();
    }
    if let Some(control_channel) = actionable_control_channel(data_channel_state) {
        match request_decoder_reset_on_control_channel(data_channel_state, &control_channel).await {
            Ok(()) => {
                if let Ok(mut state) = data_channel_state.lock() {
                    state.pending_decoder_reset = false;
                    state.pending_keyframe_request = false;
                }
            }
            Err(_) => {
                if let Ok(mut state) = data_channel_state.lock() {
                    state.pending_decoder_reset = true;
                }
            }
        }
        return;
    }
    if already_pending_decoder_reset(data_channel_state) {
        return;
    }
    let _ = request_decoder_reset_from_state(data_channel_state, runtime_stats).await;
}

fn actionable_control_channel(
    data_channel_state: &Arc<Mutex<XbxDataChannelState>>,
) -> Option<Arc<RTCDataChannel>> {
    let Ok(state) = data_channel_state.lock() else {
        return None;
    };
    if recovery_requests_ready(&state) || startup_fallback_ready(&state) {
        return state.control_channel.clone();
    }
    None
}

fn startup_fallback_ready(state: &XbxDataChannelState) -> bool {
    // 启动兜底：message handshake 未完成时，允许在 control open 后先执行恢复动作，
    // 以恢复 3f41d93 之前的启动可用性，避免持续卡在 pending。
    !state.message_handshake_acked
        && state.control_started
        && state
            .control_channel
            .as_ref()
            .is_some_and(|channel| channel.ready_state() == RTCDataChannelState::Open)
}

fn already_pending_keyframe(data_channel_state: &Arc<Mutex<XbxDataChannelState>>) -> bool {
    data_channel_state
        .lock()
        .ok()
        .is_some_and(|state| state.pending_keyframe_request)
}

fn already_pending_decoder_reset(data_channel_state: &Arc<Mutex<XbxDataChannelState>>) -> bool {
    data_channel_state
        .lock()
        .ok()
        .is_some_and(|state| state.pending_decoder_reset)
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
