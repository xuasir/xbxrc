use std::sync::{Arc, Mutex};

use crate::api::backend::{XbxEngineMediaRuntimeStats, XbxEnginePendingRuntimeRecoveryAction};
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::connection::RtcConnectionService;
use crate::transport::rtc::executor::peer::stage_reconnect_candidate;
use crate::transport::rtc::facts::{
    CommandResultFact, CommandResultStatus, TimerFact, TransportCommand, TransportFact,
};
use crate::transport::rtc::session::actor::SessionActor;
use crate::transport::rtc::session::clock::SystemSessionClock;
use crate::transport::rtc::session::policy::RtcSessionPolicy;
use crate::transport::rtc::stream::RtcMediaService;
use crate::XbxEngineRuntimeError;

// 负责把 transport fact/command 和 connection/media 副作用桥接起来，
// 让 stack.rs 只保留编排入口。
pub(crate) struct RtcTransportSessionBridge<'a> {
    runtime_stats: &'a Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    pending_runtime_recovery_action: &'a Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
    connection: &'a Arc<Mutex<RtcConnectionService>>,
    media: &'a Arc<Mutex<RtcMediaService>>,
    transport_session: &'a Arc<Mutex<SessionActor<SystemSessionClock, RtcSessionPolicy>>>,
    transport_fact_sink: &'a Arc<Mutex<Vec<TransportFact>>>,
}

impl<'a> RtcTransportSessionBridge<'a> {
    pub(crate) fn new(
        runtime_stats: &'a Arc<Mutex<XbxEngineMediaRuntimeStats>>,
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
            *session = SessionActor::new(SystemSessionClock, RtcSessionPolicy::default());
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
        self.record_transport_fact(TransportFact::CommandResult(CommandResultFact {
            command,
            status,
            observed_at_ms: crate::transport::rtc::stats::now_ms_f64(),
        }));
    }

    pub(crate) fn apply_transport_session_command(&self, command: TransportCommand) {
        match &command {
            TransportCommand::RequestReconnectCandidate {
                observation_id,
                reason,
            } => {
                let result = self
                    .pending_runtime_recovery_action
                    .lock()
                    .map_err(|_| {
                        XbxEngineRuntimeError::new("xbxEngineRtcPendingRecoveryActionLockFailed")
                    })
                    .map(|mut pending| {
                        stage_reconnect_candidate(&mut pending, *observation_id, reason.clone());
                    });
                self.record_transport_command_result(command, &result.map(|_| ()));
            }
            TransportCommand::RequestKeyframe { .. } => {
                let result = self
                    .connection
                    .lock()
                    .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))
                    .and_then(|mut connection| {
                        connection.request_video_keyframe(self.runtime_stats)
                    });
                self.record_transport_command_result(command, &result);
            }
            TransportCommand::RequestDecoderReset { .. } => {
                let result = self
                    .connection
                    .lock()
                    .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))
                    .and_then(|mut connection| {
                        connection.request_decoder_reset(self.runtime_stats)
                    });
                self.record_transport_command_result(command, &result);
            }
            TransportCommand::SetTargetRembKbps {
                target_kbps,
                reason,
                ..
            } => {
                RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
                    stats.video_remb_bps = Some(target_kbps.saturating_mul(1_000));
                    stats.latest_observation_label =
                        Some("rtcSessionCommandUpdateTargetRemb".to_string());
                    stats.latest_observation_summary = Some(format!(
                        "rtc session command updated target remb={}kbps reason={reason}",
                        target_kbps
                    ));
                });
                self.record_transport_command_result(command, &Ok(()));
            }
        }
    }
}
