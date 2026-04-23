use serde_json::{json, Value};
use xbxengine_protocol::{
    XbxEnginePresentationMilestoneDto, XbxEngineRuntimeEventDto, XbxEngineRuntimePhaseDto,
    XbxEngineTransportStateDto,
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

fn map_presentation_milestone(state: &XbxEnginePresentationMilestoneDto) -> &'static str {
    match state {
        XbxEnginePresentationMilestoneDto::Idle => "idle",
        XbxEnginePresentationMilestoneDto::Connected => "connected",
        XbxEnginePresentationMilestoneDto::MediaReady => "mediaReady",
        XbxEnginePresentationMilestoneDto::Degraded => "degraded",
        XbxEnginePresentationMilestoneDto::Failed => "failed",
        XbxEnginePresentationMilestoneDto::Closed => "closed",
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
        XbxEngineRuntimeEventDto::PresentationMilestoneChanged {
            milestone,
            connected_at_ms,
            media_ready_at_ms,
            stage,
        } => Some(json!({
            "type": "presentation.milestoneChanged",
            "milestone": map_presentation_milestone(milestone),
            "connectedAtMs": connected_at_ms,
            "mediaReadyAtMs": media_ready_at_ms,
            "stage": stage
        })),
        XbxEngineRuntimeEventDto::MediaVideoTrackStatusChanged { status } => Some(json!({
            "type": "media.videoTrackStatusChanged",
            "status": status
        })),
        XbxEngineRuntimeEventDto::MediaSurfaceReady { surface_id } => Some(json!({
            "type": "media.surfaceReady",
            "surfaceId": surface_id
        })),
        XbxEngineRuntimeEventDto::StatsVideoFrameRendered {
            first_frame_packet_arrival_time_ms,
            frame_decoded_time_ms,
            renderer_frame_time_ms,
        } => Some(json!({
            "type": "stats.videoFrameRendered",
            "firstFramePacketArrivalTimeMs": first_frame_packet_arrival_time_ms,
            "frameDecodedTimeMs": frame_decoded_time_ms,
            // `StatsVideoFrameRendered` 来自 renderer 处理时钟，不代表 host present 完成时间。
            "rendererFrameTimeMs": renderer_frame_time_ms
        })),
        XbxEngineRuntimeEventDto::FirstFrameLatencyObserved {
            connected_at_ms,
            first_packet_at_ms,
            first_decode_at_ms,
            first_render_at_ms,
            from_connected_to_first_render_ms,
            from_first_packet_to_first_render_ms,
            from_first_decode_to_first_render_ms,
        } => Some(json!({
            "type": "stats.firstFrameLatency",
            "connectedAtMs": connected_at_ms,
            "firstPacketAtMs": first_packet_at_ms,
            "firstDecodeAtMs": first_decode_at_ms,
            "firstRenderAtMs": first_render_at_ms,
            "fromConnectedToFirstRenderMs": from_connected_to_first_render_ms,
            "fromFirstPacketToFirstRenderMs": from_first_packet_to_first_render_ms,
            "fromFirstDecodeToFirstRenderMs": from_first_decode_to_first_render_ms
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
