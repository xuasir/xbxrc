// 由 `runtime_stats_sink` 模块拆分；采集面只写事实，不驱动控制决策。

use crate::diagnostics::observation_bus::ObservationEvent;
use crate::{
    XbxEngineAnchorCandidateFailureReason, XbxEngineAnchorCandidateLedger,
    XbxEngineAnchorCandidateState, XbxEngineFrameRecoveryObservation,
    XbxEngineRemoteAnswerObservation, XbxEngineRtcBuilderObservation,
    XbxEngineTwccExtensionObservation, XbxEngineTwccRemoteStreamObservation,
    XbxEngineVideoFrameDropObservation, XbxEngineVideoNackObservation,
    XbxEngineVideoPacketGapObservation, XbxEngineVideoTimelineObservation,
    XbxEngineVideoTwccObservation,
};

use super::support::*;
use super::RuntimeStatsSink;

impl RuntimeStatsSink {
    pub(crate) fn record_host_video_timing(
        &self,
        host_display_interval_ms: Option<f64>,
        host_frame_age_budget_ms: Option<f64>,
    ) {
        self.publish(ObservationEvent::HostVideoTiming {
            host_display_interval_ms,
            host_frame_age_budget_ms,
        });
    }

    pub(crate) fn record_transport_metrics(
        &self,
        video_rtt_ms: Option<f64>,
        video_rtt_source: Option<String>,
        inbound_video_loss_ratio_5s: f64,
        inbound_video_loss_ratio_1s: f64,
        transport_path: Option<String>,
        transport_candidate_pair: Option<String>,
        transport_protocol: Option<String>,
        transport_address_family: Option<String>,
        inbound_video_bitrate_kbps: f64,
        inbound_primary_video_bytes_total: u64,
    ) {
        self.publish(ObservationEvent::TransportMetrics {
            video_rtt_ms,
            video_rtt_source,
            inbound_video_loss_ratio_5s,
            inbound_video_loss_ratio_1s,
            transport_path,
            transport_candidate_pair,
            transport_protocol,
            transport_address_family,
            inbound_video_bitrate_kbps,
            inbound_primary_video_bytes_total,
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_ice_connectivity_probe(
        &self,
        candidate_pair_count: u16,
        nominated_pair_count: u16,
        succeeded_pair_count: u16,
        in_progress_pair_count: u16,
        failed_pair_count: u16,
        max_requests_sent: u64,
        max_responses_received: u64,
        responses_received_total: u64,
        has_selected_or_nominated_pair: bool,
        direct_checks_without_response: bool,
        local_candidate_type_summary: String,
        remote_candidate_type_summary: String,
        address_family_summary: String,
        observed_at_ms: f64,
    ) {
        self.publish(ObservationEvent::IceConnectivityProbe {
            candidate_pair_count,
            nominated_pair_count,
            succeeded_pair_count,
            in_progress_pair_count,
            failed_pair_count,
            max_requests_sent,
            max_responses_received,
            responses_received_total,
            has_selected_or_nominated_pair,
            direct_checks_without_response,
            local_candidate_type_summary,
            remote_candidate_type_summary,
            address_family_summary,
            observed_at_ms,
        });
    }

    pub(crate) fn record_rtc_builder_observation(
        &self,
        observation: XbxEngineRtcBuilderObservation,
    ) {
        self.publish(ObservationEvent::RtcBuilderConfigured { observation });
    }

    pub(crate) fn record_twcc_remote_stream_observation(
        &self,
        observation: XbxEngineTwccRemoteStreamObservation,
    ) {
        self.publish(ObservationEvent::TwccRemoteStreamBound { observation });
    }

    pub(crate) fn record_remote_answer_observation(
        &self,
        observation: XbxEngineRemoteAnswerObservation,
    ) {
        self.publish(ObservationEvent::RemoteAnswerApplied { observation });
    }

    pub(crate) fn record_twcc_inbound_extension_observation(
        &self,
        observation: XbxEngineTwccExtensionObservation,
    ) {
        self.publish(ObservationEvent::TwccInboundExtensionObserved { observation });
    }

    pub(crate) fn record_video_frame_drop(&self, observation: XbxEngineVideoFrameDropObservation) {
        self.publish(ObservationEvent::VideoFrameDrop { observation });
    }

    pub(crate) fn record_frame_recovery_observation(
        &self,
        observation: XbxEngineFrameRecoveryObservation,
    ) {
        self.publish(ObservationEvent::FrameRecovery { observation });
    }

    pub(crate) fn add_inbound_video_packet_loss_estimate(&self, packet_count: u16) {
        self.publish(ObservationEvent::InboundVideoPacketLossEstimate { packet_count });
    }

    pub(crate) fn set_video_pending_missing_packets(&self, pending_count: usize) {
        self.publish(ObservationEvent::VideoPendingMissingPackets { pending_count });
    }

    pub(crate) fn record_nack_sent(&self, batch_len: usize, pending_count: usize) {
        self.publish(ObservationEvent::NackSent {
            batch_len,
            pending_count,
        });
    }

    pub(crate) fn record_latest_video_nack_observation(
        &self,
        observation: XbxEngineVideoNackObservation,
    ) {
        self.publish(ObservationEvent::LatestVideoNackObservation { observation });
    }

    pub(crate) fn record_latest_video_twcc_observation(
        &self,
        observation: XbxEngineVideoTwccObservation,
    ) {
        self.publish(ObservationEvent::LatestVideoTwccObservation { observation });
    }

    pub(crate) fn record_nack_recovered(
        &self,
        was_late: bool,
        recovery_time_ms: f64,
        pending_count: usize,
        observation: XbxEngineVideoNackObservation,
    ) {
        self.publish(ObservationEvent::NackRecovered {
            was_late,
            recovery_time_ms,
            pending_count,
            observation,
        });
    }

    pub(crate) fn record_latest_video_packet_gap(
        &self,
        observation: XbxEngineVideoPacketGapObservation,
        latest_sequence: u16,
    ) {
        self.publish(ObservationEvent::LatestVideoPacketGap {
            observation,
            latest_sequence,
        });
    }

    pub(crate) fn record_video_timeline_observation(
        &self,
        observation: XbxEngineVideoTimelineObservation,
    ) {
        self.publish(ObservationEvent::VideoTimelineObserved { observation });
    }

    pub(crate) fn record_video_receiver_observation(
        &self,
        observation: crate::XbxEngineVideoReceiverObservation,
    ) {
        self.publish(ObservationEvent::VideoReceiverObserved { observation });
    }

    pub(crate) fn record_anchor_candidate_ledger(
        &self,
        recovery_epoch: u64,
        frame_rtp_timestamp: Option<u32>,
        state: XbxEngineAnchorCandidateState,
        source_event: &str,
        failure_reason: Option<XbxEngineAnchorCandidateFailureReason>,
        observed_at_ms: f64,
    ) {
        self.update(|stats| {
            let ledger = XbxEngineAnchorCandidateLedger {
                recovery_epoch,
                frame_rtp_timestamp,
                state,
                source_event: source_event.to_string(),
                failure_reason,
                observed_at_ms,
            };
            stats.latest_anchor_candidate_ledger = Some(ledger.clone());
            let bound_episode = select_episode_snapshot_for_anchor_ledger(stats, &ledger);
            emit_picture_recovery_closure_probe(
                &*stats,
                "anchor-candidate",
                observed_at_ms,
                bound_episode.as_ref(),
                Some(&ledger),
            );
        });
    }
}
