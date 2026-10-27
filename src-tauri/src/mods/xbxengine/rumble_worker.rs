use crate::mods::runtime_trace::RuntimeTraceRecorderRef;
use ohmygamepad_protocol::{
    LogicalPadId, OhMyGamepadRumbleEffectDto, OhMyGamepadRumbleRequestDto,
    OhMyGamepadRumbleTargetDto,
};
use tauri::AppHandle;
use xbxengine::XbxEngineRuntimeError;

/// Rust 媒体只上报震动请求，活动设备与后台策略统一由公共输入层决定。
#[derive(Clone)]
pub(super) struct GamepadRumbleWorkerHandle {
    app_handle: AppHandle,
}

impl GamepadRumbleWorkerHandle {
    pub(super) fn new(app_handle: AppHandle, _runtime_trace: RuntimeTraceRecorderRef) -> Self {
        Self { app_handle }
    }

    pub(super) fn submit_request(
        &self,
        request: OhMyGamepadRumbleRequestDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        if !matches!(
            request.target,
            OhMyGamepadRumbleTargetDto::Slot {
                slot: LogicalPadId::Pad0
            }
        ) {
            return Ok(());
        }
        crate::event_bridge::emit_json(
            &self.app_handle,
            "xbxrc:gamepad:stream-rumble",
            &serde_json::json!({ "effect": request.effect }),
        )
        .map_err(XbxEngineRuntimeError::new)
    }

    pub(super) fn clear_pending_requests(&self) -> Result<(), XbxEngineRuntimeError> {
        crate::event_bridge::emit_json(
            &self.app_handle,
            "xbxrc:gamepad:stream-rumble",
            &serde_json::json!({ "effect": OhMyGamepadRumbleEffectDto::default() }),
        )
        .map_err(XbxEngineRuntimeError::new)
    }

    pub(super) fn shutdown(&self) -> Result<(), XbxEngineRuntimeError> {
        self.clear_pending_requests()
    }
}
