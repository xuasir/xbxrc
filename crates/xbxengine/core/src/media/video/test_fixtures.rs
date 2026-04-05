use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;

use crate::api::backend::XbxEngineMediaRuntimeStats;
use crate::media::video::h264::inspection::H264AccessUnitInspector;
use crate::media::video::types::{
    AssembledVideoFrame, FrameRecoveryDisposition, FrameValue, VideoCodec,
};
use crate::transport::rtc::stream::adapter_types::TransportObservation;
use crate::transport::rtc::stream::nack_scheduler::NackSchedulerConfig;
use crate::transport::rtc::stream::packet_types::{
    RtcRtpPacketMeta, RtcVideoIngressKind, RtcVideoRtpPacket,
};
use crate::transport::rtc::stream::sink::RtcRtcpSendPort;
use crate::transport::rtc::stream::video_source::RtcVideoFrameSource;

#[derive(Clone, Default)]
pub(crate) struct NoopRtcpPort;

impl RtcRtcpSendPort for NoopRtcpPort {
    fn send_rtcp(&self, _payload: &[u8]) {}
}

pub(crate) fn bootstrap_sps_nalu() -> &'static [u8] {
    &hex_literal::hex!(
        "67 64 00 0A AC 72 84 44 26 84 00 00
         03 00 04 00 00 03 00 CA 3C 48 96 11 80"
    )
}

pub(crate) fn bootstrap_pps_nalu() -> &'static [u8] {
    &hex_literal::hex!("68 E8 43 8F 13 21 30")
}

pub(crate) fn bootstrap_idr_nalu() -> &'static [u8] {
    &hex_literal::hex!("65 88 81 00 05 4E 7F 87 DF")
}

pub(crate) fn bootstrap_annexb_access_unit() -> &'static [u8] {
    &hex_literal::hex!(
        "00 00 00 01 67 64 00 0A AC 72 84 44 26 84 00 00
         03 00 04 00 00 03 00 CA 3C 48 96 11 80 00 00 00
         01 68 E8 43 8F 13 21 30 00 00 01 65 88 81 00 05
         4E 7F 87 DF"
    )
}

pub(crate) fn make_video_rtp_packet(
    sequence_number: u16,
    timestamp: u32,
    marker: bool,
    payload: &[u8],
) -> RtcVideoRtpPacket {
    RtcVideoRtpPacket {
        payload: payload.to_vec(),
        meta: RtcRtpPacketMeta {
            ssrc: 42,
            payload_type: 124,
            sequence_number,
            timestamp,
            marker,
        },
        ingress_kind: RtcVideoIngressKind::Primary,
    }
}

pub(crate) async fn send_bootstrap_access_unit(
    tx: &tokio::sync::mpsc::Sender<RtcVideoRtpPacket>,
    start_seq: u16,
    timestamp: u32,
) {
    tx.send(make_video_rtp_packet(
        start_seq,
        timestamp,
        false,
        bootstrap_sps_nalu(),
    ))
    .await
    .expect("sps packet should enqueue");
    tx.send(make_video_rtp_packet(
        start_seq + 1,
        timestamp,
        false,
        bootstrap_pps_nalu(),
    ))
    .await
    .expect("pps packet should enqueue");
    tx.send(make_video_rtp_packet(
        start_seq + 2,
        timestamp,
        true,
        bootstrap_idr_nalu(),
    ))
    .await
    .expect("idr packet should enqueue");
}

pub(crate) fn make_bootstrap_assembled_frame(rtp_timestamp: u32) -> AssembledVideoFrame {
    let payload = bootstrap_annexb_access_unit();
    let inspector = H264AccessUnitInspector::new();
    let inspection = inspector
        .inspect_access_unit(payload)
        .expect("bootstrap inspection");
    AssembledVideoFrame {
        codec: VideoCodec::H264,
        is_keyframe: true,
        config_changed: inspection.config_changed,
        value: FrameValue::new(true, inspection.config_changed, payload.len()),
        budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
        width: inspection.width.unwrap_or(2560),
        height: inspection.height.unwrap_or(1440),
        rtp_timestamp,
        frame_playout_deadline_at_ms: None,
        frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
        frame_unrecoverable_reason: None,
        assembled_at: Instant::now(),
        h264: inspection,
        payload: Bytes::from(payload.to_vec()),
    }
}

pub(crate) fn make_video_source_for_test() -> (
    tokio::sync::mpsc::Sender<RtcVideoRtpPacket>,
    tokio::sync::mpsc::UnboundedReceiver<TransportObservation>,
    RtcVideoFrameSource,
) {
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let (transport_observation_tx, transport_observation_rx) =
        tokio::sync::mpsc::unbounded_channel();
    let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(NoopRtcpPort);
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let source = RtcVideoFrameSource::new(
        rx,
        transport_observation_tx,
        rtcp_port,
        runtime_stats,
        16,
        Duration::from_millis(10),
        Duration::from_millis(20),
        Duration::from_millis(200),
        NackSchedulerConfig {
            max_age_ms: 1_000,
            frame_deadline_ms: 120,
            burst_count: 2,
            retry_interval_ms: 20,
            max_retry_count: 3,
        },
    );
    (tx, transport_observation_rx, source)
}
