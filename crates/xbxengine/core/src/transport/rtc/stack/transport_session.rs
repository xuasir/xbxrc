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
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationController};
use crate::transport::rtc::session::actor::SessionActor;
use crate::transport::rtc::session::clock::SystemSessionClock;
use crate::transport::rtc::session::policy::RtcSessionPolicy;
use crate::transport::rtc::stream::RtcMediaService;
use crate::XbxEngineRuntimeError;

/// 控制面 decoder reset 最短间隔：短窗内多条决策会共用一次 RTC 请求，避免反复 advance recovery epoch。
const DECODER_RESET_CONTROL_MIN_SPACING_MS: f64 = 600.0;

// 负责把 transport fact/command 和 connection/media 副作用桥接起来，
// 让 stack.rs 只保留编排入口。
pub(crate) struct RtcTransportSessionBridge<'a> {
    runtime_stats: &'a Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    runtime_config: &'a Arc<Mutex<XbxEngineRuntimeConfig>>,
    pending_runtime_recovery_action: &'a Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
    connection: &'a Arc<Mutex<RtcConnectionService>>,
    media: &'a Arc<Mutex<RtcMediaService>>,
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
        transport_session: &'a Arc<Mutex<SessionActor<SystemSessionClock, RtcSessionPolicy>>>,
        transport_fact_sink: &'a Arc<Mutex<Vec<TransportFact>>>,
    ) -> Self {
        Self {
            runtime_stats,
            runtime_config,
            pending_runtime_recovery_action,
            connection,
            media,
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
        self.update_recovery_decision_command_result(&command, &status, observed_at_ms);
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
                if result.is_ok() {
                    if let Some(action) = self.resolve_recovery_keyframe_action_label() {
                        self.record_recovery_escalation_observation(
                            *observation_id,
                            reason.clone(),
                            action,
                            RecoveryAction::RequestKeyframe,
                        );
                    }
                }
                self.record_transport_command_result(command, &result);
            }
            TransportCommand::RequestDecoderReset {
                observation_id,
                reason,
            } => {
                let now_ms = crate::transport::rtc::stats::now_ms_f64();
                let coalesce_recent_control_reset = RuntimeStatsSink::read_shared(
                    self.runtime_stats.as_ref(),
                    |stats| {
                        stats
                            .latest_video_escalation_observation
                            .as_ref()
                            .is_some_and(|obs| {
                                obs.action == "requestDecoderReset"
                                    && (now_ms - obs.observed_at_ms).max(0.0)
                                        < DECODER_RESET_CONTROL_MIN_SPACING_MS
                            })
                    },
                )
                .unwrap_or(false);
                if coalesce_recent_control_reset {
                    self.record_transport_command_status(
                        command,
                        CommandResultStatus::Deferred {
                            reason: "coalescedRecentDecoderResetControl".to_string(),
                        },
                    );
                    return;
                }
                let result = self
                    .connection
                    .lock()
                    .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))
                    .and_then(|mut connection| {
                        connection.request_decoder_reset(self.runtime_stats)
                    });
                if result.is_ok() {
                    self.record_recovery_escalation_observation(
                        *observation_id,
                        reason.clone(),
                        "requestDecoderReset".to_string(),
                        RecoveryAction::RequestDecoderReset,
                    );
                }
                self.record_transport_command_result(command, &result);
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

    fn record_recovery_escalation_observation(
        &self,
        observation_id: u64,
        reason: String,
        action: String,
        recovery_action: RecoveryAction,
    ) {
        let observed_at_ms = crate::transport::rtc::stats::now_ms_f64();
        let contract = VideoEscalationController::action_contract(recovery_action);
        RuntimeStatsSink::new(self.runtime_stats.clone()).record_recovery_escalation_success(
            observation_id,
            reason,
            action.as_str(),
            observed_at_ms,
            contract.advances_recovery_epoch_on_success,
        );
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
            if let Some(ledger) = stats.latest_recovery_decision_ledger.as_mut() {
                if ledger.decision_id != decision_id {
                    return;
                }
                ledger.command_result = Some(result_label.clone());
                ledger.command_detail = Some(match detail.as_deref() {
                    Some(raw) => format!("command={command_name} detail={raw}"),
                    None => format!("command={command_name}"),
                });
                ledger.observed_at_ms = observed_at_ms;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::api::runtime::XbxEngineRuntimeConfig;
    use crate::transport::rtc::connection::RtcConnectionService;
    use crate::transport::rtc::facts::{CommandResultStatus, TransportCommand};
    use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationController};
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
        let runtime_stats = Box::leak(Box::new(runtime_stats));
        let runtime_config = Box::leak(Box::new(Arc::new(Mutex::new(
            XbxEngineRuntimeConfig::default(),
        ))));
        let pending_runtime_recovery_action = Box::leak(Box::new(pending_runtime_recovery_action));
        let connection = Box::leak(Box::new(Arc::new(Mutex::new(
            RtcConnectionService::default(),
        ))));
        let media = Box::leak(Box::new(Arc::new(Mutex::new(RtcMediaService::default()))));
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
            transport_session,
            transport_fact_sink,
        )
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
    fn epoch_advance_contract_is_defined_by_recovery_owner_layer() {
        let keyframe = VideoEscalationController::action_contract(RecoveryAction::RequestKeyframe);
        assert!(!keyframe.advances_recovery_epoch_on_success);

        let reset = VideoEscalationController::action_contract(RecoveryAction::RequestDecoderReset);
        assert!(reset.advances_recovery_epoch_on_success);

        let reconnect =
            VideoEscalationController::action_contract(RecoveryAction::RequestReconnectCandidate);
        assert!(reconnect.advances_recovery_epoch_on_success);
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
    fn command_result_updates_matching_recovery_decision_ledger() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.latest_recovery_decision_ledger =
            Some(crate::XbxEngineRecoveryDecisionLedgerObservation {
                decision_id: 88,
                state_before: "recovering".to_string(),
                state_after: "reconnecting".to_string(),
                input_signal: "liveness:livenessNoProgressTimeout".to_string(),
                gate_result: "pass".to_string(),
                action_selected: "requestReconnectCandidate".to_string(),
                budget_before: None,
                budget_after: None,
                command_result: None,
                command_detail: None,
                observed_at_ms: 10.0,
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
    }
}
