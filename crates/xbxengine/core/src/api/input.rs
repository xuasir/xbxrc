use ohmygamepad_host::{GamepadRuntimeHost, GamepadRuntimeHostError};
use ohmygamepad_protocol::{
    LogicalButtonsStateDto, LogicalPadStateDto, OhMyGamepadSamplingConfigDto,
    OhMyGamepadStreamPushModeDto,
};
use std::time::Duration;

use crate::api::runtime::XbxEngineRuntimeError;

const XBXENGINE_VIRTUAL_DEVICE_ID: &str = "virtual:xbxengine-controller";
const XBXENGINE_INPUT_BACKEND_POLL_RATE_HZ: u16 = 500;
const XBXENGINE_INPUT_LOGICAL_SAMPLE_RATE_HZ: u16 = 250;
const XBXENGINE_INPUT_STREAM_PUSH_RATE_HZ: u16 = 120;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct XbxEngineInputStatus {
    pub device_count: usize,
    pub pad_count: usize,
    pub stream_input_active: bool,
}

pub trait XbxEngineInputBackend: Send {
    fn attach_session(
        &mut self,
        session_id: &str,
    ) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError>;
    fn press_controller_button(
        &mut self,
        button: &str,
        duration_ms: u64,
    ) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError>;
    fn snapshot_status(&self) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError>;
    fn stop(&mut self) -> Result<(), XbxEngineRuntimeError>;
}

#[derive(Default)]
pub struct NoopXbxEngineInputBackend;

impl XbxEngineInputBackend for NoopXbxEngineInputBackend {
    fn attach_session(
        &mut self,
        _session_id: &str,
    ) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        Ok(XbxEngineInputStatus::default())
    }

    fn press_controller_button(
        &mut self,
        _button: &str,
        _duration_ms: u64,
    ) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        Ok(XbxEngineInputStatus::default())
    }

    fn snapshot_status(&self) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        Ok(XbxEngineInputStatus::default())
    }

    fn stop(&mut self) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }
}

pub struct OhMyGamepadXbxEngineInputBackend {
    host: Option<GamepadRuntimeHost>,
}

impl OhMyGamepadXbxEngineInputBackend {
    pub fn new() -> Self {
        Self { host: None }
    }

    fn ensure_host(&mut self) -> Result<&GamepadRuntimeHost, XbxEngineRuntimeError> {
        if self.host.is_none() {
            let host = GamepadRuntimeHost::shared()
                .map_err(|error| XbxEngineRuntimeError::new(error.to_string()))?;
            self.host = Some(host);
        }
        self.host
            .as_ref()
            .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineInputHostMissing"))
    }

    fn ensure_stream_sampling(host: &GamepadRuntimeHost) -> Result<(), XbxEngineRuntimeError> {
        host.set_sampling(stable_stream_sampling_config())
            .map_err(|error| {
                XbxEngineRuntimeError::new(format!("updateOhMyGamepadSampling:{error:?}"))
            })
    }
}

impl XbxEngineInputBackend for OhMyGamepadXbxEngineInputBackend {
    fn attach_session(
        &mut self,
        session_id: &str,
    ) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        let host = self.ensure_host()?;
        Self::ensure_stream_sampling(host)?;
        host.set_stream_pad_forwarding(true).map_err(|error| {
            XbxEngineRuntimeError::new(format!("setOhMyGamepadStreamPadForwarding:{error:?}"))
        })?;
        let _ = session_id;
        self.snapshot_status()
    }

    fn press_controller_button(
        &mut self,
        button: &str,
        duration_ms: u64,
    ) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        let host = self.ensure_host()?;
        let pressed_state = logical_state_for_button(button, duration_ms)?;
        host.submit_simulated_state(XBXENGINE_VIRTUAL_DEVICE_ID, pressed_state)
            .map_err(map_gamepad_host_error("submitSimulatedButtonPressed"))?;
        // rust-owned 路径要显式保留按压窗口，否则 Nexus/Home 会被瞬时释放吞掉。
        if duration_ms > 0 {
            std::thread::sleep(Duration::from_millis(duration_ms));
        }
        host.submit_simulated_state(XBXENGINE_VIRTUAL_DEVICE_ID, LogicalPadStateDto::default())
            .map_err(map_gamepad_host_error("submitSimulatedButtonReleased"))?;
        self.snapshot_status()
    }

    fn snapshot_status(&self) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        let host = self
            .host
            .as_ref()
            .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineInputHostMissing"))?;
        let snapshot = host.snapshot().map_err(|error| {
            XbxEngineRuntimeError::new(format!("snapshotOhMyGamepadHost:{error:?}"))
        })?;
        Ok(XbxEngineInputStatus {
            device_count: snapshot.devices.len(),
            pad_count: snapshot.slots.len(),
            stream_input_active: host.stream_pad_forwarding(),
        })
    }

    fn stop(&mut self) -> Result<(), XbxEngineRuntimeError> {
        if let Some(host) = self.host.take() {
            host.set_stream_pad_forwarding(false).map_err(|error| {
                XbxEngineRuntimeError::new(format!("resetOhMyGamepadStreamPadForwarding:{error:?}"))
            })?;
        }
        Ok(())
    }
}

fn map_gamepad_host_error(
    action: &'static str,
) -> impl FnOnce(GamepadRuntimeHostError) -> XbxEngineRuntimeError {
    move |error| XbxEngineRuntimeError::new(format!("{action}:{error:?}"))
}

fn stable_stream_sampling_config() -> OhMyGamepadSamplingConfigDto {
    OhMyGamepadSamplingConfigDto {
        backend_poll_rate_hz: XBXENGINE_INPUT_BACKEND_POLL_RATE_HZ,
        logical_pad_sample_rate_hz: XBXENGINE_INPUT_LOGICAL_SAMPLE_RATE_HZ,
        ui_push_rate_hz: 60,
        stream_push_mode: OhMyGamepadStreamPushModeDto::FixedRate,
        stream_push_rate_hz: Some(XBXENGINE_INPUT_STREAM_PUSH_RATE_HZ),
    }
}

fn logical_state_for_button(
    button: &str,
    _duration_ms: u64,
) -> Result<LogicalPadStateDto, XbxEngineRuntimeError> {
    let mut buttons = LogicalButtonsStateDto::default();
    match button {
        "south" | "a" => buttons.south = 1.0,
        "east" | "b" => buttons.east = 1.0,
        "west" | "x" => buttons.west = 1.0,
        "north" | "y" => buttons.north = 1.0,
        "l1" | "lb" => buttons.l1 = 1.0,
        "r1" | "rb" => buttons.r1 = 1.0,
        "l2" | "lt" => buttons.l2 = 1.0,
        "r2" | "rt" => buttons.r2 = 1.0,
        "l3" => buttons.l3 = 1.0,
        "r3" => buttons.r3 = 1.0,
        "view" | "back" => buttons.view = 1.0,
        "menu" | "start" => buttons.menu = 1.0,
        "home" | "nexus" | "guide" => buttons.home = 1.0,
        "dpad-up" => buttons.dpad_up = 1.0,
        "dpad-down" => buttons.dpad_down = 1.0,
        "dpad-left" => buttons.dpad_left = 1.0,
        "dpad-right" => buttons.dpad_right = 1.0,
        _ => {
            return Err(XbxEngineRuntimeError::new(format!(
                "unsupportedXbxEngineControllerButton:{button}"
            )))
        }
    }

    Ok(LogicalPadStateDto {
        buttons,
        ..LogicalPadStateDto::default()
    })
}
