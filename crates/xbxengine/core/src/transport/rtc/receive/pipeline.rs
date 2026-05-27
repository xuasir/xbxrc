use std::sync::Arc;
use std::time::Duration;

use crate::transport::rtc::capability::{ConnectionTransportCapability, RtcTransportCapability};
use crate::transport::rtc::connection::RtcConnectionService;
use crate::transport::rtc::receive::build_rtc_video_frame_source;
use crate::transport::rtc::stream::adapter_types::VideoFramePipelineSources;
use crate::transport::rtc::stream::nack_contract::NackSchedulerConfig;
use crate::XbxEngineMediaRuntimeStats;

/// 新接收主线入口：`RtcReceiveCore`（`ReceiveCoreBody` + `DecodeGate` + ingress）；组帧在 `ReceiveEngine.frame_assembler`。
pub(crate) fn build_rtc_receive_pipeline(
    ingress_capacity: usize,
    runtime_stats: Arc<std::sync::Mutex<XbxEngineMediaRuntimeStats>>,
    connection: Arc<std::sync::Mutex<RtcConnectionService>>,
    max_late_packets: u16,
    jitter_buffer_min_delay: Duration,
    jitter_buffer_max_delay: Duration,
    idle_timeout: std::time::Duration,
    nack_config: NackSchedulerConfig,
    jitter_early_emit_enabled: bool,
) -> (
    Box<dyn crate::transport::rtc::stream::sink::RtcMediaSink>,
    VideoFramePipelineSources,
) {
    let capability: Arc<dyn RtcTransportCapability> = Arc::new(ConnectionTransportCapability::new(
        connection,
        runtime_stats.clone(),
    ));
    let (sink, sources) = build_rtc_video_frame_source(
        ingress_capacity,
        runtime_stats,
        max_late_packets,
        jitter_buffer_min_delay,
        jitter_buffer_max_delay,
        idle_timeout,
        nack_config,
        jitter_early_emit_enabled,
        capability,
    );
    (sink, sources)
}
