use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use xbxengine_protocol::{
    XbxEngineRuntimeEventDto, XbxEngineRuntimePhaseDto, XbxEngineTransportStateDto,
};

pub const AUTH_SESSION_READY_CHANNEL: &str = "xbxrc:auth:session-ready";
pub const GAMEPAD_RUNTIME_SNAPSHOT_CHANNEL: &str = "xbxrc:gamepad:runtime-snapshot";
pub const GAMEPAD_DEVICES_CHANGED_CHANNEL: &str = "xbxrc:gamepad:devices-changed";
pub const GAMEPAD_PAD_SNAPSHOT_CHANNEL: &str = "xbxrc:gamepad:pad-snapshot";
pub const GAMEPAD_ROUTE_CHANGED_CHANNEL: &str = "xbxrc:gamepad:route-changed";
pub const STREAMING_XBXENGINE_RUNTIME_EVENT_CHANNEL: &str = "streaming:xbxengine-runtime-event";

fn normalize_provider(provider: &str) -> &str {
    if provider == "xal" {
        return "xal";
    }
    "xal"
}

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

pub fn emit_auth_session_ready(
    app_handle: &AppHandle,
    provider: &str,
    app_level: u32,
) -> Result<(), String> {
    app_handle
        .emit(
            AUTH_SESSION_READY_CHANNEL,
            json!({
                "provider": normalize_provider(provider),
                "appLevel": app_level,
                "at": chrono::Utc::now().to_rfc3339()
            }),
        )
        .map_err(|error| error.to_string())
}

pub fn emit_gamepad_runtime_snapshot(
    app_handle: &AppHandle,
    snapshot: &Value,
) -> Result<(), String> {
    app_handle
        .emit(GAMEPAD_RUNTIME_SNAPSHOT_CHANNEL, snapshot)
        .map_err(|error| error.to_string())
}

pub fn emit_gamepad_devices_changed(app_handle: &AppHandle, devices: &Value) -> Result<(), String> {
    app_handle
        .emit(GAMEPAD_DEVICES_CHANGED_CHANNEL, devices)
        .map_err(|error| error.to_string())
}

pub fn emit_gamepad_pad_snapshot(
    app_handle: &AppHandle,
    pad_snapshot: &Value,
) -> Result<(), String> {
    app_handle
        .emit(GAMEPAD_PAD_SNAPSHOT_CHANNEL, pad_snapshot)
        .map_err(|error| error.to_string())
}

pub fn emit_gamepad_route_changed(
    app_handle: &AppHandle,
    route_target: &Value,
) -> Result<(), String> {
    app_handle
        .emit(GAMEPAD_ROUTE_CHANGED_CHANNEL, route_target)
        .map_err(|error| error.to_string())
}

pub fn map_xbxengine_runtime_event(event: &XbxEngineRuntimeEventDto) -> Option<Value> {
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
