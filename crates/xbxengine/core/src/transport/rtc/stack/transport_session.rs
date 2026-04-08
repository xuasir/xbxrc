use std::sync::{Arc, Mutex};

use crate::api::backend::{
    XbxEngineMediaRuntimeStats, XbxEnginePendingRuntimeRecoveryAction, XbxEngineVideoBweObservation,
};
use crate::api::runtime::XbxEngineRuntimeConfig;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::connection::RtcConnectionService;
use crate::transport::rtc::executor::peer::{
    stage_reconnect_candidate, StageReconnectCandidateOutcome,
};
use crate::transport::rtc::facts::{
    CommandResultFact, CommandResultStatus, TimerFact, TransportCommand, TransportFact,
};
use crate::transport::rtc::recovery::escalation::{
    RecoveryAction, VideoEscalationController, VideoEscalationReason,
};
use crate::transport::rtc::session::actor::SessionActor;
use crate::transport::rtc::session::clock::SystemSessionClock;
use crate::transport::rtc::session::policy::RtcSessionPolicy;
use crate::transport::rtc::stream::RtcMediaService;
use crate::XbxEngineRuntimeError;

/// 控制面 decoder reset 最短间隔：短窗内多条决策会共用一次 RTC 请求，避免反复 advance recovery epoch。
const DECODER_RESET_CONTROL_MIN_SPACING_MS: f64 = 600.0;
/// 恢复命令族的 in-flight 观察窗口：用于识别同族合并与升级，而不是回落到泛化 cooldown。
const RECOVERY_COMMAND_FAMILY_IN_FLIGHT_WINDOW_MS: f64 = 960.0;
const RECOVERY_COMMAND_REASON_FAMILY_IN_FLIGHT_DECODER_RESET: &str =
    "familyInFlight:decoderResetInFlight";
const RECOVERY_COMMAND_REASON_FAMILY_IN_FLIGHT_CONTROL_PENDING: &str =
    "familyInFlight:controlChannelPending";
const RECOVERY_COMMAND_REASON_SAME_FAMILY_KEYFRAME_COALESCED: &str =
    "sameFamilyCoalesced:keyframeInFlight";
const RECOVERY_COMMAND_REASON_SAME_FAMILY_DECODER_RESET_COALESCED: &str =
    "sameFamilyCoalesced:decoderResetInFlight";
const RECOVERY_COMMAND_REASON_SAME_FAMILY_TRANSPORT_STAGE_COALESCED: &str =
    "sameFamilyCoalesced:transportStageSuppressed";
const RECOVERY_COMMAND_SEMANTIC_FAMILY_UPGRADE_KEYFRAME_TO_DECODER_RESET: &str =
    "familyUpgrade:keyframeInFlight->decoderReset";
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecoveryCommandKind {
    RequestKeyframe,
    RequestDecoderReset,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RecoveryCommandFamilyDecision {
    Proceed,
    Defer { reason: String },
    Upgrade { detail: String },
}

// 负责把 transport fact/command 和 connection/media 副作用桥接起来，
// 让 stack.rs 只保留编排入口。
pub(crate) struct RtcTransportSessionBridge<'a> {
    runtime_stats: &'a Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    runtime_config: &'a Arc<Mutex<XbxEngineRuntimeConfig>>,
    pending_runtime_recovery_action: &'a Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
    connection: &'a Arc<Mutex<RtcConnectionService>>,
    media: &'a Arc<Mutex<RtcMediaService>>,
    local_decoder_reset_handle:
        &'a Arc<Mutex<Option<Arc<crate::media::video::decode::actor::DecodeActorHandle>>>>,
    transport_session: &'a Arc<Mutex<SessionActor<SystemSessionClock, RtcSessionPolicy>>>,
    transport_fact_sink: &'a Arc<Mutex<Vec<TransportFact>>>,
}

impl<'a> RtcTransportSessionBridge<'a> {
    pub(crate) fn new(
        runtime_stats: &'a Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        runtime_config: &'a Arc<Mutex<XbxEngineRuntimeConfig>>,
        pending_runtime_recovery_action: &'a Arc<
            Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>,
        >,
        connection: &'a Arc<Mutex<RtcConnectionService>>,
        media: &'a Arc<Mutex<RtcMediaService>>,
        local_decoder_reset_handle: &'a Arc<
            Mutex<Option<Arc<crate::media::video::decode::actor::DecodeActorHandle>>>,
        >,
        transport_session: &'a Arc<Mutex<SessionActor<SystemSessionClock, RtcSessionPolicy>>>,
        transport_fact_sink: &'a Arc<Mutex<Vec<TransportFact>>>,
    ) -> Self {
        Self {
            runtime_stats,
            runtime_config,
            pending_runtime_recovery_action,
            connection,
            media,
            local_decoder_reset_handle,
            transport_session,
            transport_fact_sink,
        }
    }

    pub(crate) fn pump_connection_and_media_ingress(&self) {
        crate::xbx_log_debug!("[xbxengine][rtc-stack] pump_connection_and_media_ingress enter");
        self.record_transport_fact(TransportFact::Timer(TimerFact::MetricsSampleTick {
            observed_at_ms: crate::transport::rtc::stats::now_ms_f64(),
        }));
        let (ingress_packets, connection_facts) = self
            .connection
            .lock()
            .ok()
            .map(|mut connection| {
                crate::xbx_log_debug!("[xbxengine][rtc-stack] pump_connection lock acquired");
                let _ = connection.pump(self.runtime_stats);
                (
                    connection.take_media_ingress_packets(),
                    connection.take_transport_facts(),
                )
            })
            .unwrap_or_default();
        for fact in connection_facts {
            self.record_transport_fact(fact);
        }
        crate::xbx_log_debug!(
            "[xbxengine][rtc-stack] pump_connection_and_media_ingress ingress_packets={}",
            ingress_packets.len()
        );
        if !ingress_packets.is_empty() {
            if let Ok(mut media) = self.media.lock() {
                for (packet, rtp_meta) in ingress_packets {
                    // 第一阶段只把连接层原始包送进媒体入口，不进入组帧/送解链。
                    media.observe_ingress_packet(packet, rtp_meta, self.runtime_stats);
                }
            }
        }
        self.drain_transport_fact_sink();
        crate::xbx_log_debug!("[xbxengine][rtc-stack] pump_connection_and_media_ingress exit");
    }

    pub(crate) fn record_transport_fact(&self, fact: TransportFact) {
        let mut pending_commands = Vec::new();
        if let Ok(mut session) = self.transport_session.lock() {
            session.enqueue_fact(fact);
            let _ = session.drain_once(64);
            while let Some(command) = session.pop_next_command() {
                pending_commands.push(command);
            }
        }
        for command in pending_commands {
            self.apply_transport_session_command(command);
        }
    }

    pub(crate) fn reset_transport_session(&self) {
        if let Ok(mut session) = self.transport_session.lock() {
            *session = SessionActor::new(
                SystemSessionClock,
                RtcSessionPolicy::new(self.runtime_config.clone(), self.runtime_stats.clone()),
            );
        }
    }

    pub(crate) fn drain_transport_fact_sink(&self) {
        let facts = self
            .transport_fact_sink
            .lock()
            .ok()
            .map(|mut pending| std::mem::take(&mut *pending))
            .unwrap_or_default();
        for fact in facts {
            self.record_transport_fact(fact);
        }
    }

    pub(crate) fn record_transport_command_result(
        &self,
        command: TransportCommand,
        result: &Result<(), XbxEngineRuntimeError>,
    ) {
        let status = match result {
            Ok(()) => CommandResultStatus::Succeeded,
            Err(error) => CommandResultStatus::Failed {
                error: error.to_string(),
            },
        };
        self.record_transport_command_status(command, status);
    }

    pub(crate) fn record_transport_command_status(
        &self,
        command: TransportCommand,
        status: CommandResultStatus,
    ) {
        self.record_transport_command_status_with_semantic(command, status, None);
    }

    fn record_transport_command_status_with_semantic(
        &self,
        command: TransportCommand,
        status: CommandResultStatus,
        semantic_detail: Option<String>,
    ) {
        let observed_at_ms = crate::transport::rtc::stats::now_ms_f64();
        // CommandResult 若同步回灌到 session，会在同一调用栈里触发
        // apply_command -> command_result -> apply_command 的递归环。
        // 这里统一改为入队，交给下一轮 drain 处理，避免 tokio worker 栈溢出。
        if let Ok(mut pending) = self.transport_fact_sink.lock() {
            pending.push(TransportFact::CommandResult(CommandResultFact {
                command: command.clone(),
                status: status.clone(),
                observed_at_ms,
            }));
        }
        self.update_recovery_decision_command_result(
            &command,
            &status,
            observed_at_ms,
            semantic_detail.as_deref(),
        );
        self.record_transport_command_semantic_observation(
            &command,
            &status,
            semantic_detail.as_deref(),
            observed_at_ms,
        );
    }

    pub(crate) fn apply_transport_session_command(&self, command: TransportCommand) {
        match &command {
            TransportCommand::RequestReconnectCandidate {
                observation_id,
                reason,
                reason_domain,
            } => {
                let result = self
                    .pending_runtime_recovery_action
                    .lock()
                    .map_err(|_| {
                        XbxEngineRuntimeError::new("xbxEngineRtcPendingRecoveryActionLockFailed")
                    })
                    .map(|mut pending| {
                        let stage_outcome = stage_reconnect_candidate(
                            &mut pending,
                            *observation_id,
                            reason.clone(),
                            *reason_domain,
                        );
                        let pending_reason = pending.as_ref().map(|action| match action {
                            XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
                                reason,
                                ..
                            } => reason.clone(),
                        });
                        (stage_outcome, pending_reason)
                    });
                if result.as_ref().is_ok_and(|(stage_outcome, _)| {
                    !matches!(stage_outcome, StageReconnectCandidateOutcome::Unchanged)
                }) {
                    self.record_recovery_escalation_observation(
                        *observation_id,
                        reason.clone(),
                        "requestReconnectCandidate".to_string(),
                        RecoveryAction::RequestReconnectCandidate,
                    );
                }
                RuntimeStatsSink::update_shared(self.runtime_stats, |stats| {
                    match result.as_ref() {
                        Ok((
                            StageReconnectCandidateOutcome::StagedNew
                            | StageReconnectCandidateOutcome::StagedUpdated,
                            pending_reason,
                        )) => {
                            stats.latest_observation_label =
                                Some("rtcReconnectCandidateStaged".to_string());
                            stats.latest_observation_summary = Some(format!(
                                "observationId={} reason={} pendingReason={}",
                                observation_id,
                                reason,
                                pending_reason.as_deref().unwrap_or("none")
                            ));
                        }
                        Ok((StageReconnectCandidateOutcome::Unchanged, pending_reason)) => {
                            stats.latest_observation_label =
                                Some("rtcReconnectCandidateRejected".to_string());
                            stats.latest_observation_summary = Some(format!(
                                "observationId={} reason={} pendingReason={} staged=false",
                                observation_id,
                                reason,
                                pending_reason.as_deref().unwrap_or("none")
                            ));
                        }
                        Err(error) => {
                            stats.latest_observation_label =
                                Some("rtcReconnectCandidateStageFailed".to_string());
                            stats.latest_observation_summary =
                                Some(format!("reason={} error={error}", reason));
                        }
                    }
                });
                let command_result = match result.as_ref() {
                    Ok((
                        StageReconnectCandidateOutcome::StagedNew
                        | StageReconnectCandidateOutcome::StagedUpdated,
                        _,
                    )) => CommandResultStatus::Succeeded,
                    Ok((StageReconnectCandidateOutcome::Unchanged, pending_reason)) => {
                        CommandResultStatus::Deferred {
                            reason: format!(
                                "pendingReason={}",
                                pending_reason.as_deref().unwrap_or("none")
                            ),
                        }
                    }
                    Err(error) => CommandResultStatus::Failed {
                        error: error.to_string(),
                    },
                };
                self.record_transport_command_status(command, command_result);
            }
            TransportCommand::RequestKeyframe {
                observation_id,
                reason,
            } => {
                let requested_at_ms = crate::transport::rtc::stats::now_ms_f64();
                match self.resolve_recovery_command_family_decision(
                    RecoveryCommandKind::RequestKeyframe,
                    requested_at_ms,
                ) {
                    RecoveryCommandFamilyDecision::Defer { reason } => {
                        self.record_transport_command_status(
                            command,
                            CommandResultStatus::Deferred { reason },
                        );
                        return;
                    }
                    RecoveryCommandFamilyDecision::Upgrade { .. }
                    | RecoveryCommandFamilyDecision::Proceed => {}
                }
                RuntimeStatsSink::new(self.runtime_stats.clone())
                    .record_keyframe_request_episode_requested(
                        *observation_id,
                        Some(reason.clone()),
                        requested_at_ms,
                        None,
                    );
                let result = self
                    .connection
                    .lock()
                    .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))
                    .and_then(|mut connection| {
                        connection.request_video_keyframe(self.runtime_stats)
                    });
                let latest_observation_label =
                    RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
                        stats.latest_observation_label.clone()
                    })
                    .flatten();
                let (command_status, semantic_detail) = self
                    .resolve_recovery_command_status_from_result(
                        RecoveryCommandKind::RequestKeyframe,
                        &result,
                        latest_observation_label.as_deref(),
                    );
                if matches!(command_status, CommandResultStatus::Succeeded) {
                    if let Some(action) = self.resolve_recovery_keyframe_action_label() {
                        self.record_recovery_escalation_observation(
                            *observation_id,
                            reason.clone(),
                            action,
                            RecoveryAction::RequestKeyframe,
                        );
                    }
                }
                match &command_status {
                    CommandResultStatus::Deferred { reason } => {
                        RuntimeStatsSink::new(self.runtime_stats.clone())
                            .record_keyframe_request_episode_deferred(requested_at_ms, reason);
                    }
                    CommandResultStatus::Failed { error } => {
                        RuntimeStatsSink::new(self.runtime_stats.clone())
                            .record_keyframe_request_episode_failed(requested_at_ms, error);
                    }
                    CommandResultStatus::Succeeded => {}
                }
                self.record_transport_command_status_with_semantic(
                    command,
                    command_status,
                    semantic_detail,
                );
            }
            TransportCommand::RequestDecoderReset {
                observation_id,
                reason,
            } => {
                let now_ms = crate::transport::rtc::stats::now_ms_f64();
                let family_decision = self.resolve_recovery_command_family_decision(
                    RecoveryCommandKind::RequestDecoderReset,
                    now_ms,
                );
                let family_upgrade_detail = match &family_decision {
                    RecoveryCommandFamilyDecision::Defer { reason } => {
                        self.record_transport_command_status(
                            command,
                            CommandResultStatus::Deferred {
                                reason: reason.clone(),
                            },
                        );
                        return;
                    }
                    RecoveryCommandFamilyDecision::Upgrade { detail } => Some(detail.clone()),
                    RecoveryCommandFamilyDecision::Proceed => None,
                };
                let command_status =
                    match self.request_local_decoder_reset(format!("recoveryCommand:{reason}")) {
                        Ok(()) => CommandResultStatus::Succeeded,
                        Err(error) => CommandResultStatus::Deferred {
                            reason: error.to_string(),
                        },
                    };
                if matches!(command_status, CommandResultStatus::Succeeded) {
                    self.record_recovery_escalation_observation(
                        *observation_id,
                        reason.clone(),
                        "requestDecoderReset".to_string(),
                        RecoveryAction::RequestDecoderReset,
                    );
                }
                self.record_transport_command_status_with_semantic(
                    command,
                    command_status,
                    family_upgrade_detail,
                );
            }
            TransportCommand::SetTargetRembKbps {
                target_kbps,
                reason,
                observation_id,
            } => {
                let result = self
                    .connection
                    .lock()
                    .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))
                    .and_then(|mut connection| {
                        connection.request_target_remb_kbps(*target_kbps, self.runtime_stats)
                    });
                let bwe_mode = self
                    .runtime_config
                    .lock()
                    .ok()
                    .map(|config| config.webrtc.bwe_mode.clone())
                    .unwrap_or_else(|| "fixed-remb".to_string());
                let observed_at_ms = crate::transport::rtc::stats::now_ms_f64();
                let target_kbps = *target_kbps;
                let observation_id = *observation_id;
                let decision_reason = reason.clone();
                RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
                    let twcc = stats.latest_video_twcc_observation.clone();
                    let observed_remb_kbps = stats.video_remb_bps.map(|bps| bps / 1_000);
                    let actual_video_bitrate_kbps = twcc
                        .as_ref()
                        .and_then(|value| value.receive_bitrate_kbps)
                        .or(stats.inbound_video_bitrate_kbps)
                        .unwrap_or(0.0)
                        .max(0.0);
                    stats.video_remb_bps = Some(target_kbps.saturating_mul(1_000));
                    stats.latest_video_bwe_observation = Some(XbxEngineVideoBweObservation {
                        observation_id,
                        mode: bwe_mode.clone(),
                        decision_reason: decision_reason.clone(),
                        target_remb_kbps: target_kbps,
                        observed_remb_kbps,
                        actual_video_bitrate_kbps,
                        loss_ratio: stats.inbound_video_loss_ratio_1s.clamp(0.0, 1.0),
                        rtt_ms: stats.video_rtt_ms,
                        transport_path: stats.transport_path.clone(),
                        twcc_feedback_interval_ms: twcc
                            .as_ref()
                            .and_then(|value| value.feedback_interval_ms),
                        twcc_observed_packet_count: twcc
                            .as_ref()
                            .map(|value| value.observed_packet_count),
                        twcc_covered_sequence_span: twcc
                            .as_ref()
                            .map(|value| value.covered_sequence_span),
                        twcc_receive_bitrate_kbps: twcc
                            .as_ref()
                            .and_then(|value| value.receive_bitrate_kbps),
                        twcc_delivery_ratio: twcc.as_ref().map(|value| value.delivery_ratio),
                        twcc_loss_ratio: twcc.as_ref().map(|value| value.packet_loss_ratio),
                        observed_at_ms,
                    });
                    stats.latest_observation_label =
                        Some("rtcSessionCommandUpdateTargetRemb".to_string());
                    stats.latest_observation_summary = Some(format!(
                        "rtc session command updated target remb={}kbps reason={}",
                        target_kbps, decision_reason
                    ));
                });
                self.record_transport_command_result(command, &result);
            }
        }
    }

    fn queue_local_decoder_reset(&self, reason: String, observed_at_ms: f64) -> bool {
        let local_decoder_handle = self
            .local_decoder_reset_handle
            .lock()
            .ok()
            .and_then(|handle| handle.clone());
        RuntimeStatsSink::update_shared(self.runtime_stats, |stats| {
            stats.latest_observation_label = Some(
                if local_decoder_handle.is_some() {
                    "videoDecoderLocalResetQueued"
                } else {
                    "videoDecoderLocalResetSkipped"
                }
                .to_string(),
            );
            stats.latest_observation_summary = Some(format!(
                "reason={reason} source=recoveryCommand observedAtMs={observed_at_ms:.3}"
            ));
        });
        if let Some(handle) = local_decoder_handle {
            handle.request_local_decoder_reset(reason, observed_at_ms);
            return true;
        }
        false
    }

    pub(crate) fn request_local_decoder_reset(
        &self,
        reason: String,
    ) -> Result<(), XbxEngineRuntimeError> {
        let observed_at_ms = crate::transport::rtc::stats::now_ms_f64();
        if self.queue_local_decoder_reset(reason, observed_at_ms) {
            return Ok(());
        }
        Err(XbxEngineRuntimeError::new(
            "xbxEngineRtcLocalDecoderResetHandleUnavailable",
        ))
    }

    fn record_recovery_escalation_observation(
        &self,
        observation_id: u64,
        reason: String,
        action: String,
        recovery_action: RecoveryAction,
    ) {
        let observed_at_ms = crate::transport::rtc::stats::now_ms_f64();
        let advances_recovery_epoch = self
            .should_advance_transport_recovery_epoch_on_success(recovery_action, reason.as_str());
        RuntimeStatsSink::new(self.runtime_stats.clone()).record_recovery_escalation_success(
            observation_id,
            reason,
            action.as_str(),
            observed_at_ms,
            advances_recovery_epoch,
        );
    }

    fn should_advance_transport_recovery_epoch_on_success(
        &self,
        recovery_action: RecoveryAction,
        reason_label: &str,
    ) -> bool {
        VideoEscalationController::action_success_advances_transport_recovery_epoch(
            recovery_action,
            VideoEscalationReason::from_recovery_reason_label(reason_label),
        )
    }

    fn resolve_recovery_keyframe_action_label(&self) -> Option<String> {
        let latest_observation_label = RuntimeStatsSink::read_shared(self.runtime_stats, |stats| {
            stats.latest_observation_label.clone()
        })
        .flatten();
        match latest_observation_label.as_deref() {
            Some("rtcVideoPliRequested") => Some("requestKeyframe(pli)".to_string()),
            Some("rtcVideoFirRequested") => Some("requestKeyframe(fir)".to_string()),
            Some("rtcVideoRecoverySuppressed") => None,
            Some("rtcVideoPliFallbackControl")
            | Some("rtcVideoFirFallbackControl")
            | Some("rtcControlKeyframeRequested") => Some("requestKeyframe".to_string()),
            Some(_) | None => Some("requestKeyframe".to_string()),
        }
    }

    fn update_recovery_decision_command_result(
        &self,
        command: &TransportCommand,
        status: &CommandResultStatus,
        observed_at_ms: f64,
        semantic_detail: Option<&str>,
    ) {
        let decision_id = match command {
            TransportCommand::RequestKeyframe { observation_id, .. }
            | TransportCommand::RequestDecoderReset { observation_id, .. }
            | TransportCommand::RequestReconnectCandidate { observation_id, .. }
            | TransportCommand::SetTargetRembKbps { observation_id, .. } => *observation_id,
        };
        let command_name = match command {
            TransportCommand::RequestKeyframe { .. } => "requestKeyframe",
            TransportCommand::RequestDecoderReset { .. } => "requestDecoderReset",
            TransportCommand::RequestReconnectCandidate { .. } => "requestReconnectCandidate",
            TransportCommand::SetTargetRembKbps { .. } => "setTargetRembKbps",
        };
        let (result_label, detail) = match status {
            CommandResultStatus::Succeeded => ("succeeded".to_string(), None),
            CommandResultStatus::Deferred { reason } => {
                ("deferred".to_string(), Some(reason.clone()))
            }
            CommandResultStatus::Failed { error } => ("failed".to_string(), Some(error.clone())),
        };
        RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
            let apply_update = |ledger: &mut crate::XbxEngineRecoveryDecisionLedgerObservation| {
                ledger.command_result = Some(result_label.clone());
                ledger.command_detail = Some(self.build_command_detail(
                    command_name,
                    detail.as_deref(),
                    semantic_detail,
                ));
                ledger.observed_at_ms = observed_at_ms;
            };
            if let Some(index) = stats
                .recent_recovery_decision_ledgers
                .iter()
                .rposition(|ledger| ledger.decision_id == decision_id)
            {
                apply_update(&mut stats.recent_recovery_decision_ledgers[index]);
            }
            if let Some(ledger) = stats.latest_recovery_decision_ledger.as_mut() {
                if ledger.decision_id != decision_id {
                    return;
                }
                apply_update(ledger);
            }
        });
    }

    fn build_command_detail(
        &self,
        command_name: &str,
        status_detail: Option<&str>,
        semantic_detail: Option<&str>,
    ) -> String {
        let mut detail = format!("command={command_name}");
        if let Some(raw) = status_detail {
            detail.push_str(" detail=");
            detail.push_str(raw);
        }
        if let Some(semantic) = semantic_detail {
            detail.push_str(" semantic=");
            detail.push_str(semantic);
        }
        detail
    }

    fn record_transport_command_semantic_observation(
        &self,
        command: &TransportCommand,
        status: &CommandResultStatus,
        semantic_detail: Option<&str>,
        observed_at_ms: f64,
    ) {
        let command_name = match command {
            TransportCommand::RequestKeyframe { .. } => "requestKeyframe",
            TransportCommand::RequestDecoderReset { .. } => "requestDecoderReset",
            TransportCommand::RequestReconnectCandidate { .. } => "requestReconnectCandidate",
            TransportCommand::SetTargetRembKbps { .. } => "setTargetRembKbps",
        };
        let status_name = match status {
            CommandResultStatus::Succeeded => "succeeded",
            CommandResultStatus::Deferred { .. } => "deferred",
            CommandResultStatus::Failed { .. } => "failed",
        };
        let status_detail = match status {
            CommandResultStatus::Deferred { reason } => Some(reason.as_str()),
            CommandResultStatus::Failed { error } => Some(error.as_str()),
            CommandResultStatus::Succeeded => None,
        };
        RuntimeStatsSink::new(self.runtime_stats.clone()).record_transport_command_semantic(
            command_name,
            status_name,
            status_detail,
            semantic_detail,
            observed_at_ms,
        );
    }

    fn resolve_recovery_command_status_from_result(
        &self,
        command_kind: RecoveryCommandKind,
        result: &Result<(), XbxEngineRuntimeError>,
        latest_observation_label: Option<&str>,
    ) -> (CommandResultStatus, Option<String>) {
        match result {
            Ok(()) => {
                if matches!(command_kind, RecoveryCommandKind::RequestKeyframe)
                    && latest_observation_label == Some("rtcVideoRecoverySuppressed")
                {
                    return (
                        CommandResultStatus::Deferred {
                            reason: RECOVERY_COMMAND_REASON_SAME_FAMILY_TRANSPORT_STAGE_COALESCED
                                .to_string(),
                        },
                        None,
                    );
                }
                (CommandResultStatus::Succeeded, None)
            }
            Err(error) => {
                let error_text = error.to_string();
                if self.is_control_channel_not_ready_error(command_kind, &error_text) {
                    return (
                        CommandResultStatus::Deferred {
                            reason: RECOVERY_COMMAND_REASON_FAMILY_IN_FLIGHT_CONTROL_PENDING
                                .to_string(),
                        },
                        None,
                    );
                }
                (CommandResultStatus::Failed { error: error_text }, None)
            }
        }
    }

    fn is_control_channel_not_ready_error(
        &self,
        command_kind: RecoveryCommandKind,
        error: &str,
    ) -> bool {
        match command_kind {
            RecoveryCommandKind::RequestKeyframe => {
                error.contains("xbxEngineRtcControlChannelNotReadyForKeyframe")
            }
            RecoveryCommandKind::RequestDecoderReset => {
                error.contains("xbxEngineRtcControlChannelNotReadyForDecoderReset")
            }
        }
    }

    fn resolve_recovery_command_family_decision(
        &self,
        command_kind: RecoveryCommandKind,
        now_ms: f64,
    ) -> RecoveryCommandFamilyDecision {
        let (keyframe_in_flight, decoder_reset_in_flight) =
            RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
                let keyframe_in_flight = stats
                    .latest_keyframe_request_episode
                    .as_ref()
                    .is_some_and(|episode| {
                        let pending_verdict =
                            matches!(episode.response_verdict.as_deref(), None | Some("pending"));
                        let in_flight_status =
                            matches!(episode.status.as_str(), "requested" | "sent")
                                && episode.sent_at_ms.is_some();
                        let anchor_at_ms = episode.sent_at_ms.unwrap_or(episode.requested_at_ms);
                        let within_window = (now_ms - anchor_at_ms).max(0.0)
                            <= RECOVERY_COMMAND_FAMILY_IN_FLIGHT_WINDOW_MS;
                        pending_verdict && in_flight_status && within_window
                    });
                let decoder_reset_in_flight = stats
                    .latest_video_escalation_observation
                    .as_ref()
                    .is_some_and(|obs| {
                        matches!(
                            obs.action.as_str(),
                            "requestDecoderReset"
                                | "requestKeyframe+decoderReset"
                                | "requestKeyframe+decoderReset(startupLowQualityRetry)"
                        ) && (now_ms - obs.observed_at_ms).max(0.0)
                            < DECODER_RESET_CONTROL_MIN_SPACING_MS
                    });
                (keyframe_in_flight, decoder_reset_in_flight)
            })
            .unwrap_or((false, false));

        match command_kind {
            RecoveryCommandKind::RequestKeyframe => {
                if decoder_reset_in_flight {
                    return RecoveryCommandFamilyDecision::Defer {
                        reason: RECOVERY_COMMAND_REASON_FAMILY_IN_FLIGHT_DECODER_RESET.to_string(),
                    };
                }
                if keyframe_in_flight {
                    return RecoveryCommandFamilyDecision::Defer {
                        reason: RECOVERY_COMMAND_REASON_SAME_FAMILY_KEYFRAME_COALESCED.to_string(),
                    };
                }
                RecoveryCommandFamilyDecision::Proceed
            }
            RecoveryCommandKind::RequestDecoderReset => {
                if decoder_reset_in_flight {
                    return RecoveryCommandFamilyDecision::Defer {
                        reason: RECOVERY_COMMAND_REASON_SAME_FAMILY_DECODER_RESET_COALESCED
                            .to_string(),
                    };
                }
                if keyframe_in_flight {
                    return RecoveryCommandFamilyDecision::Upgrade {
                        detail: RECOVERY_COMMAND_SEMANTIC_FAMILY_UPGRADE_KEYFRAME_TO_DECODER_RESET
                            .to_string(),
                    };
                }
                RecoveryCommandFamilyDecision::Proceed
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    use crate::api::runtime::XbxEngineRuntimeConfig;
    use crate::media::video::decode::actor::{DecodeActorHandle, DecodeMsg};
    use crate::transport::rtc::connection::RtcConnectionService;
    use crate::transport::rtc::facts::{CommandResultStatus, TransportCommand};
    use crate::transport::rtc::recovery::escalation::RecoveryAction;
    use crate::transport::rtc::session::actor::SessionActor;
    use crate::transport::rtc::session::clock::SystemSessionClock;
    use crate::transport::rtc::session::policy::RtcSessionPolicy;
    use crate::transport::rtc::stream::RtcMediaService;
    use crate::{
        XbxEngineMediaRuntimeStats, XbxEnginePendingRuntimeRecoveryAction,
        XbxEngineVideoEscalationObservation,
    };

    use super::RtcTransportSessionBridge;

    fn build_bridge(
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        pending_runtime_recovery_action: Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
    ) -> RtcTransportSessionBridge<'static> {
        build_bridge_with_local_decoder_reset_handle(
            runtime_stats,
            pending_runtime_recovery_action,
            Arc::new(Mutex::new(None)),
        )
    }

    fn build_bridge_with_local_decoder_reset_handle(
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        pending_runtime_recovery_action: Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
        local_decoder_reset_handle: Arc<Mutex<Option<Arc<DecodeActorHandle>>>>,
    ) -> RtcTransportSessionBridge<'static> {
        let runtime_stats = Box::leak(Box::new(runtime_stats));
        let runtime_config = Box::leak(Box::new(Arc::new(Mutex::new(
            XbxEngineRuntimeConfig::default(),
        ))));
        let pending_runtime_recovery_action = Box::leak(Box::new(pending_runtime_recovery_action));
        let connection = Box::leak(Box::new(Arc::new(Mutex::new(
            RtcConnectionService::default(),
        ))));
        let media = Box::leak(Box::new(Arc::new(Mutex::new(RtcMediaService::default()))));
        let local_decoder_reset_handle = Box::leak(Box::new(local_decoder_reset_handle));
        let transport_session = Box::leak(Box::new(Arc::new(Mutex::new(SessionActor::new(
            SystemSessionClock,
            RtcSessionPolicy::new(runtime_config.clone(), runtime_stats.clone()),
        )))));
        let transport_fact_sink = Box::leak(Box::new(Arc::new(Mutex::new(Vec::new()))));

        RtcTransportSessionBridge::new(
            runtime_stats,
            runtime_config,
            pending_runtime_recovery_action,
            connection,
            media,
            local_decoder_reset_handle,
            transport_session,
            transport_fact_sink,
        )
    }

    #[test]
    fn queue_local_decoder_reset_marks_skipped_when_handle_missing() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        assert!(!bridge.queue_local_decoder_reset("recoveryCommand:test".to_string(), 321.0,));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            snapshot.latest_observation_label.as_deref(),
            Some("videoDecoderLocalResetSkipped")
        );
        assert!(snapshot
            .latest_observation_summary
            .as_deref()
            .is_some_and(|summary| summary.contains("source=recoveryCommand")));
    }

    #[test]
    fn queue_local_decoder_reset_enqueues_message_when_handle_available() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let (tx, rx) = mpsc::sync_channel(1);
        let handle = Arc::new(DecodeActorHandle::from_test_sender(tx));
        let bridge = build_bridge_with_local_decoder_reset_handle(
            runtime_stats.clone(),
            pending_runtime_recovery_action,
            Arc::new(Mutex::new(Some(handle))),
        );

        assert!(bridge.queue_local_decoder_reset("recoveryCommand:test".to_string(), 654.0,));

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            snapshot.latest_observation_label.as_deref(),
            Some("videoDecoderLocalResetQueued")
        );
        drop(snapshot);
        let msg = rx.recv().expect("local decoder reset message");
        match msg {
            DecodeMsg::LocalDecoderReset {
                reason,
                observed_at_ms,
            } => {
                assert_eq!(reason, "recoveryCommand:test");
                assert_eq!(observed_at_ms, 654.0);
            }
            _ => panic!("unexpected decode message"),
        }
    }

    #[test]
    fn request_decoder_reset_command_enqueues_local_reset_without_connection_support() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let (tx, rx) = mpsc::sync_channel(1);
        let handle = Arc::new(DecodeActorHandle::from_test_sender(tx));
        let bridge = build_bridge_with_local_decoder_reset_handle(
            runtime_stats.clone(),
            pending_runtime_recovery_action,
            Arc::new(Mutex::new(Some(handle))),
        );

        bridge.apply_transport_session_command(TransportCommand::RequestDecoderReset {
            observation_id: 42,
            reason: "transportAwaitRecoveryKeyframe".to_string(),
        });

        let msg = rx.recv().expect("local decoder reset message");
        match msg {
            DecodeMsg::LocalDecoderReset { reason, .. } => {
                assert_eq!(reason, "recoveryCommand:transportAwaitRecoveryKeyframe");
            }
            _ => panic!("unexpected decode message"),
        }
        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            snapshot.latest_observation_label.as_deref(),
            Some("videoDecoderLocalResetQueued")
        );
    }

    #[test]
    fn reconnect_candidate_records_escalation_observation_when_staged() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(TransportCommand::RequestReconnectCandidate {
            observation_id: 42,
            reason: "recovering-stream".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        });

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let escalation = snapshot
            .latest_video_escalation_observation
            .as_ref()
            .expect("escalation should be recorded");
        assert_eq!(escalation.observation_id, 42);
        assert_eq!(escalation.reason, "recovering-stream");
        assert_eq!(escalation.action, "requestReconnectCandidate");
        assert_eq!(
            snapshot.transport_recovery_epoch_at_last_escalation,
            snapshot.transport_recovery_epoch
        );
    }

    #[test]
    fn reconnect_candidate_overwrites_pending_and_records_new_escalation() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(Some(
            XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
                observation_id: 1,
                reason: "existing".to_string(),
                reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
            },
        )));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(TransportCommand::RequestReconnectCandidate {
            observation_id: 43,
            reason: "new-reason".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        });

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let escalation = snapshot
            .latest_video_escalation_observation
            .as_ref()
            .expect("escalation should be recorded");
        assert_eq!(escalation.observation_id, 43);
        assert_eq!(escalation.reason, "new-reason");
        assert_eq!(
            snapshot.latest_observation_label.as_deref(),
            Some("rtcReconnectCandidateStaged")
        );
    }

    #[test]
    fn reconnect_candidate_stage_preserves_reason_domain_in_pending_action() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats, pending_runtime_recovery_action.clone());

        bridge.apply_transport_session_command(TransportCommand::RequestReconnectCandidate {
            observation_id: 44,
            reason: "displaySupplyCritical".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::Local,
        });

        let pending = pending_runtime_recovery_action
            .lock()
            .expect("pending reconnect action lock");
        assert!(matches!(
            pending.as_ref(),
            Some(XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
                observation_id: 44,
                reason,
                reason_domain: crate::XbxEngineRecoveryReasonDomain::Local,
            }) if reason == "displaySupplyCritical"
        ));
    }

    #[test]
    fn reconnect_advances_recovery_epoch_by_contract() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 2;
        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(TransportCommand::RequestReconnectCandidate {
            observation_id: 77,
            reason: "reconnect-needed".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        });

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let escalation = snapshot
            .latest_video_escalation_observation
            .as_ref()
            .expect("escalation should be recorded");
        assert_eq!(escalation.action, "requestReconnectCandidate");
        assert_eq!(snapshot.transport_recovery_epoch, 3);
        assert_eq!(snapshot.transport_recovery_epoch_at_last_escalation, 3);
    }

    #[test]
    fn transport_session_maps_local_decoder_reset_reason_to_non_advancing_epoch_policy() {
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats, pending_runtime_recovery_action);

        assert!(!bridge.should_advance_transport_recovery_epoch_on_success(
            RecoveryAction::RequestDecoderReset,
            "displaySupplyDegraded",
        ));
        assert!(bridge.should_advance_transport_recovery_epoch_on_success(
            RecoveryAction::RequestDecoderReset,
            "transportAwaitRecoveryKeyframe",
        ));
    }

    #[test]
    fn decoder_reset_is_deferred_when_control_reset_observation_is_recent() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 7;
        stats.latest_video_escalation_observation = Some(XbxEngineVideoEscalationObservation {
            observation_id: 101,
            reason: "transportAwaitRecoveryKeyframe".to_string(),
            action: "requestDecoderReset".to_string(),
            recovery_stage: "rebuilding-supply".to_string(),
            recovery_chain_value: "anchor".to_string(),
            recovery_failure_cost: "high".to_string(),
            recovery_window_source: "hard-fallback-window".to_string(),
            observed_at_ms: now_ms,
        });
        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(TransportCommand::RequestDecoderReset {
            observation_id: 202,
            reason: "transportAwaitRecoveryKeyframe".to_string(),
        });

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(snapshot.transport_recovery_epoch, 7);
        assert_eq!(
            snapshot
                .latest_video_escalation_observation
                .as_ref()
                .map(|obs| obs.observation_id),
            Some(101)
        );
    }

    #[test]
    fn unsent_requested_keyframe_does_not_hold_family_gate() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 11,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: None,
                status: "requested".to_string(),
                status_detail: None,
                requested_at_ms: now_ms,
                sent_at_ms: None,
                deadline_at_ms: None,
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("pending".to_string()),
            });
        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.apply_transport_session_command(TransportCommand::RequestKeyframe {
            observation_id: 22,
            reason: "ingressWaitKeyframe".to_string(),
        });

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let episode = snapshot
            .latest_keyframe_request_episode
            .as_ref()
            .expect("new episode should be recorded");
        assert_eq!(episode.episode_id, 22);
        assert_eq!(episode.status, "deferred");
        assert_eq!(
            episode.response_verdict.as_deref(),
            Some("transportDeferred")
        );
    }

    #[test]
    fn command_result_updates_matching_recovery_decision_ledger() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        let matching_ledger = crate::XbxEngineRecoveryDecisionLedgerObservation {
            decision_id: 88,
            state_before: "recovering".to_string(),
            state_after: "reconnecting".to_string(),
            input_signal: "liveness:livenessNoProgressTimeout".to_string(),
            gate_result: "pass".to_string(),
            action_selected: "requestReconnectCandidate".to_string(),
            budget_before: None,
            budget_after: None,
            trigger_observation_label: None,
            trigger_observation_summary: None,
            command_result: None,
            command_detail: None,
            observed_at_ms: 10.0,
        };
        stats.latest_recovery_decision_ledger = Some(matching_ledger.clone());
        stats.recent_recovery_decision_ledgers.push(matching_ledger);
        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.record_transport_command_status(
            TransportCommand::RequestReconnectCandidate {
                observation_id: 88,
                reason: "recovering-stream".to_string(),
                reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
            },
            CommandResultStatus::Deferred {
                reason: "pendingReason=existing".to_string(),
            },
        );

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        let ledger = snapshot
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("decision ledger");
        assert_eq!(ledger.command_result.as_deref(), Some("deferred"));
        assert_eq!(
            ledger.command_detail.as_deref(),
            Some("command=requestReconnectCandidate detail=pendingReason=existing")
        );
        let historical_ledger = snapshot
            .recent_recovery_decision_ledgers
            .iter()
            .find(|ledger| ledger.decision_id == 88)
            .expect("historical decision ledger");
        assert_eq!(
            historical_ledger.command_result.as_deref(),
            Some("deferred")
        );
    }

    #[test]
    fn command_result_updates_historical_ledger_when_latest_has_rotated() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recent_recovery_decision_ledgers.push(
            crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 88,
                state_before: "recovering".to_string(),
                state_after: "reconnecting".to_string(),
                input_signal: "liveness:livenessNoProgressTimeout".to_string(),
                gate_result: "pass".to_string(),
                action_selected: "requestReconnectCandidate".to_string(),
                budget_before: None,
                budget_after: None,
                trigger_observation_label: None,
                trigger_observation_summary: None,
                command_result: None,
                command_detail: None,
                observed_at_ms: 10.0,
            },
        );
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 99,
                state_before: "recovering".to_string(),
                state_after: "recovering".to_string(),
                input_signal: "none".to_string(),
                gate_result: "no-signal".to_string(),
                action_selected: "none".to_string(),
                budget_before: None,
                budget_after: None,
                trigger_observation_label: None,
                trigger_observation_summary: None,
                command_result: None,
                command_detail: None,
                observed_at_ms: 11.0,
            });
        let runtime_stats = Arc::new(Mutex::new(stats));
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let bridge = build_bridge(runtime_stats.clone(), pending_runtime_recovery_action);

        bridge.record_transport_command_status(
            TransportCommand::RequestReconnectCandidate {
                observation_id: 88,
                reason: "recovering-stream".to_string(),
                reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
            },
            CommandResultStatus::Succeeded,
        );

        let snapshot = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            snapshot
                .latest_recovery_decision_ledger
                .as_ref()
                .map(|ledger| ledger.decision_id),
            Some(99)
        );
        assert_eq!(
            snapshot
                .latest_recovery_decision_ledger
                .as_ref()
                .and_then(|ledger| ledger.command_result.as_deref()),
            None
        );
        let historical_ledger = snapshot
            .recent_recovery_decision_ledgers
            .iter()
            .find(|ledger| ledger.decision_id == 88)
            .expect("historical decision ledger");
        assert_eq!(
            historical_ledger.command_result.as_deref(),
            Some("succeeded")
        );
    }
}
