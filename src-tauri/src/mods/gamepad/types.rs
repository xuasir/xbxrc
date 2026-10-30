use ohmygamepad_protocol::{LogicalPadStateDto, OhMyGamepadRuntimeSnapshotDto};
use serde::Serialize;

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamepadRuntimeSnapshotDto {
    pub devices: Vec<GamepadDeviceDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamepadDeviceDto {
    pub device_id: String,
    pub name: String,
    pub supports_rumble: bool,
    pub state: LogicalPadStateDto,
}

impl From<OhMyGamepadRuntimeSnapshotDto> for GamepadRuntimeSnapshotDto {
    fn from(snapshot: OhMyGamepadRuntimeSnapshotDto) -> Self {
        let mut samples: std::collections::HashMap<_, _> = snapshot
            .device_samples
            .into_iter()
            .map(|sample| (sample.device_id, sample.state))
            .collect();
        Self {
            devices: snapshot
                .devices
                .into_iter()
                .filter(|device| device.connected)
                .map(|device| {
                    let state = samples.remove(&device.device_id).unwrap_or_default();
                    GamepadDeviceDto {
                        device_id: device.device_id,
                        name: device.name,
                        supports_rumble: device.sdl3_capabilities.supports_rumble,
                        state,
                    }
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ohmygamepad_protocol::{GamepadDeviceSampleDto, OhMyGamepadDeviceDto};

    #[test]
    fn desktop_snapshot_contains_connected_standard_devices_only() {
        let mut state = LogicalPadStateDto::default();
        state.left_stick.x = 0.02;
        let mut connected = OhMyGamepadDeviceDto::default();
        connected.device_id = "sdl:1".into();
        connected.name = "Xbox".into();
        connected.connected = true;
        connected.sdl3_capabilities.supports_rumble = true;
        let snapshot = OhMyGamepadRuntimeSnapshotDto {
            devices: vec![connected, OhMyGamepadDeviceDto::default()],
            device_samples: vec![GamepadDeviceSampleDto {
                device_id: "sdl:1".into(),
                state,
            }],
            ..Default::default()
        };
        let projected = serde_json::to_value(GamepadRuntimeSnapshotDto::from(snapshot)).unwrap();
        assert_eq!(projected.as_object().unwrap().len(), 1);
        let devices = projected["devices"].as_array().unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].as_object().unwrap().len(), 4);
        assert_eq!(devices[0]["supportsRumble"], true);
        assert_eq!(
            devices[0]["state"]["leftStick"]["x"],
            serde_json::json!(0.02f32)
        );
    }
}
