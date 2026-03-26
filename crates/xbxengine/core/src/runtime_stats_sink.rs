use std::sync::{Arc, Mutex};

use crate::diagnostics::observation_bus::{ObservationBus, ObservationEvent};
use crate::{
    XbxEngineMediaRuntimeStats, XbxEngineRemoteAnswerObservation, XbxEngineRtcBuilderObservation,
    XbxEngineTwccExtensionObservation, XbxEngineTwccRemoteStreamObservation,
    XbxEngineVideoFrameDropObservation, XbxEngineVideoNackObservation,
    XbxEngineVideoPacketGapObservation, XbxEngineVideoRtxReinjectObservation,
    XbxEngineVideoTwccObservation,
};

#[derive(Clone)]
pub(crate) struct RuntimeStatsSink {
    // 统一承接 runtime stats 的发布入口，避免热路径散落字段写逻辑。
    observation_bus: ObservationBus,
}

impl RuntimeStatsSink {
    pub(crate) fn new(runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>) -> Self {
        Self {
            observation_bus: ObservationBus::new(runtime_stats),
        }
    }

    pub(crate) fn read_shared<T>(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        project: impl FnOnce(&XbxEngineMediaRuntimeStats) -> T,
    ) -> Option<T> {
        runtime_stats.lock().ok().map(|stats| project(&stats))
    }

    pub(crate) fn update_shared(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        apply: impl FnOnce(&mut XbxEngineMediaRuntimeStats),
    ) {
        if let Ok(mut stats) = runtime_stats.lock() {
            apply(&mut stats);
        }
    }

    pub(crate) fn update(&self, apply: impl FnOnce(&mut XbxEngineMediaRuntimeStats)) {
        self.observation_bus.update(apply);
    }

    pub(crate) fn read<T>(
        &self,
        project: impl FnOnce(&XbxEngineMediaRuntimeStats) -> T,
    ) -> Option<T> {
        self.observation_bus.read(project)
    }

    pub(crate) fn publish(&self, event: ObservationEvent) {
        self.observation_bus.publish(event);
    }

    pub(crate) fn record_frame_arrival(&self, now_ms: f64, frame_count: u64, fps: f64) {
        self.publish(ObservationEvent::FrameArrival {
            now_ms,
            frame_count,
            fps,
        });
    }

    pub(crate) fn record_stream_dimensions(&self, width: u32, height: u32) {
        if width == 0 {
            return;
        }
        self.publish(ObservationEvent::StreamDimensions { width, height });
    }

    pub(crate) fn record_video_rtx_reinject(
        &self,
        observation: XbxEngineVideoRtxReinjectObservation,
    ) {
        self.publish(ObservationEvent::VideoRtxReinject { observation });
    }

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
        inbound_video_bitrate_kbps: f64,
        inbound_primary_video_bytes_total: u64,
    ) {
        self.publish(ObservationEvent::TransportMetrics {
            video_rtt_ms,
            video_rtt_source,
            inbound_video_loss_ratio_5s,
            inbound_video_loss_ratio_1s,
            transport_path,
            inbound_video_bitrate_kbps,
            inbound_primary_video_bytes_total,
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

    pub(crate) fn add_inbound_video_packet_loss_estimate(&self, packet_count: u16) {
        self.publish(ObservationEvent::InboundVideoPacketLossEstimate { packet_count });
    }

    pub(crate) fn add_video_loss_finalized(&self, packet_count: usize) {
        self.publish(ObservationEvent::VideoLossFinalized { packet_count });
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

    pub(crate) fn begin_transport_recovery_episode(&self) -> u64 {
        let mut next_epoch = 0u64;
        self.update(|stats| {
            stats.transport_recovery_epoch = stats.transport_recovery_epoch.saturating_add(1);
            next_epoch = stats.transport_recovery_epoch;
        });
        next_epoch
    }
}
