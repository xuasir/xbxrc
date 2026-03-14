use serde_json::{json, Value};
use xbxengine_protocol::{
    XbxEngineRuntimeEventDto, XbxEngineRuntimePhaseDto, XbxEngineTransportStateDto,
};

pub const STREAMING_XBXENGINE_RUNTIME_EVENT_CHANNEL: &str = "streaming:xbxengine-runtime-event";

fn map_runtime_phase(phase: &XbxEngineRuntimePhaseDto) -> &'static str {
    match phase {
        XbxEngineRuntimePhaseDto::Binding => "binding",
        XbxEngineRuntimePhaseDto::ExchangingOffer => "exchangingOffer",
        XbxEngineRuntimePhaseDto::GatheringIce => "gatheringIce",
        XbxEngineRuntimePhaseDto::ExchangingIce => "exchangingIce",
        XbxEngineRuntimePhaseDto::Connecting => "connecting",
        XbxEngineRuntimePhaseDto::Reconnecting => "reconnecting",
    }
}

fn map_transport_state(state: &XbxEngineTransportStateDto) -> &'static str {
    match state {
        XbxEngineTransportStateDto::New => "new",
        XbxEngineTransportStateDto::Connecting => "connecting",
        XbxEngineTransportStateDto::Connected => "connected",
        XbxEngineTransportStateDto::Disconnected => "disconnected",
        XbxEngineTransportStateDto::Failed => "failed",
        XbxEngineTransportStateDto::Closed => "closed",
    }
}

/// xbxengine runtime 事件到 shared 约定 payload 的映射。
pub fn map_runtime_event(event: &XbxEngineRuntimeEventDto) -> Option<Value> {
    match event {
        XbxEngineRuntimeEventDto::RuntimePhaseChanged { phase } => Some(json!({
            "type": "runtime.phaseChanged",
            "phase": map_runtime_phase(phase)
        })),
        XbxEngineRuntimeEventDto::TransportConnectionStateChanged { state } => Some(json!({
            "type": "transport.connectionState",
            "state": map_transport_state(state)
        })),
        XbxEngineRuntimeEventDto::ChatStateChanged { capturing, paused } => Some(json!({
            "type": "chat.stateChanged",
            "capturing": capturing,
            "paused": paused
        })),
        XbxEngineRuntimeEventDto::MediaVideoReady { width, height } => Some(json!({
            "type": "media.videoReady",
            "width": width,
            "height": height
        })),
        XbxEngineRuntimeEventDto::MediaSurfaceReady { surface_id } => Some(json!({
            "type": "media.surfaceReady",
            "surfaceId": surface_id
        })),
        XbxEngineRuntimeEventDto::StatsVideoFrameProcessed {
            first_frame_packet_arrival_time_ms,
            frame_decoded_time_ms,
            frame_rendered_time_ms,
        } => Some(json!({
            "type": "stats.videoFrameProcessed",
            "firstFramePacketArrivalTimeMs": first_frame_packet_arrival_time_ms,
            "frameDecodedTimeMs": frame_decoded_time_ms,
            "frameRenderedTimeMs": frame_rendered_time_ms
        })),
        XbxEngineRuntimeEventDto::ErrorReported { code, message } => Some(json!({
            "type": "error",
            "code": code,
            "message": message
        })),
        // diagnostics 脉冲当前不在 shared contract 中，先不向 renderer 广播。
        XbxEngineRuntimeEventDto::DiagnosticsPulse { .. } => None,
    }
}
