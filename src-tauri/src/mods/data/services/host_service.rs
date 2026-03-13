use crate::mods::data::session_resolver::resolve_web_token_claims;
use crate::mods::data::types::{DataHostStorageDeviceSummary, DataHostSummary, DataSessionContext};
use serde_json::Value;
use xbox_webapi::SmartglassApi;

pub struct HostService;

impl HostService {
    pub fn new() -> Self {
        Self
    }

    // 与 Electron 语义一致：主机列表来自 smartglass provider。
    pub async fn get_hosts(
        &self,
        session: &DataSessionContext,
    ) -> Result<Vec<DataHostSummary>, String> {
        let Some(claims) = resolve_web_token_claims(&session.web_token) else {
            return Ok(Vec::new());
        };
        let smartglass = SmartglassApi::new(claims.uhs, claims.user_token);

        match tokio::time::timeout(
            std::time::Duration::from_secs(8),
            smartglass.get_consoles_list(),
        )
        .await
        {
            Ok(Ok(payload)) => Ok(extract_consoles(&payload)),
            Ok(Err(error)) => {
                // 与迁移前 JS 行为保持一致：网络波动时降级为空数组，避免把 hosts 查询变成致命错误。
                log::warn!(
                    "[Data] load hosts failed, fallback to empty list: {}",
                    error
                );
                Ok(Vec::new())
            }
            Err(_) => {
                log::warn!("[Data] load hosts timeout, fallback to empty list");
                Ok(Vec::new())
            }
        }
    }
}

fn extract_consoles(raw: &Value) -> Vec<DataHostSummary> {
    let mut visited = std::collections::HashSet::new();

    fn visit(
        value: &Value,
        depth: usize,
        visited: &mut std::collections::HashSet<usize>,
    ) -> Option<Vec<DataHostSummary>> {
        if depth > 5 {
            return None;
        }

        let ptr = value as *const Value as usize;
        if visited.contains(&ptr) {
            return None;
        }
        visited.insert(ptr);

        if let Some(array) = value.as_array() {
            let consoles = array.iter().filter_map(to_host_summary).collect::<Vec<_>>();
            if !consoles.is_empty() {
                return Some(consoles);
            }
            return None;
        }

        let Some(object) = value.as_object() else {
            return None;
        };

        let candidates = [
            "results", "result", "devices", "consoles", "items", "data", "response", "body",
        ];
        for key in candidates {
            if let Some(found) = object
                .get(key)
                .and_then(|next| visit(next, depth + 1, visited))
            {
                return Some(found);
            }
        }

        None
    }

    let consoles = visit(raw, 0, &mut visited).unwrap_or_default();
    eprintln!("[data][hosts] parsed consoles count={}", consoles.len());
    consoles
}

fn to_host_summary(value: &Value) -> Option<DataHostSummary> {
    let object = value.as_object()?;

    let id = object.get("id").and_then(Value::as_str).map(str::to_string);
    let device_id = object
        .get("deviceId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let server_id = object
        .get("serverId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string);
    let device_name = object
        .get("deviceName")
        .and_then(Value::as_str)
        .map(str::to_string);

    // 对齐 Electron：只要存在 console identity 字段之一即保留。
    let has_identity = id.is_some()
        || device_id.is_some()
        || server_id.is_some()
        || name.is_some()
        || device_name.is_some();
    if !has_identity {
        return None;
    }

    let storage_devices = object
        .get("storageDevices")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let o = item.as_object()?;
                    Some(DataHostStorageDeviceSummary {
                        storage_device_id: o
                            .get("storageDeviceId")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        storage_device_name: o
                            .get("storageDeviceName")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        id: o.get("id").and_then(Value::as_str).map(str::to_string),
                        name: o.get("name").and_then(Value::as_str).map(str::to_string),
                        free_space_bytes: o.get("freeSpaceBytes").and_then(Value::as_u64),
                        free_bytes: o.get("freeBytes").and_then(Value::as_u64),
                        total_space_bytes: o.get("totalSpaceBytes").and_then(Value::as_u64),
                        total_bytes: o.get("totalBytes").and_then(Value::as_u64),
                    })
                })
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty());

    Some(DataHostSummary {
        id,
        device_id,
        server_id,
        name,
        device_name,
        locale: object
            .get("locale")
            .and_then(Value::as_str)
            .map(str::to_string),
        region: object
            .get("region")
            .and_then(Value::as_str)
            .map(str::to_string),
        power_state: object
            .get("powerState")
            .and_then(Value::as_str)
            .map(str::to_string),
        console_type: object
            .get("consoleType")
            .and_then(Value::as_str)
            .map(str::to_string),
        digital_assistant_remote_control_enabled: object
            .get("digitalAssistantRemoteControlEnabled")
            .and_then(Value::as_bool),
        remote_management_enabled: object
            .get("remoteManagementEnabled")
            .and_then(Value::as_bool),
        console_streaming_enabled: object
            .get("consoleStreamingEnabled")
            .and_then(Value::as_bool),
        wireless_warning: object.get("wirelessWarning").and_then(Value::as_bool),
        out_of_home_warning: object.get("outOfHomeWarning").and_then(Value::as_bool),
        storage_devices,
        console_addrs: None,
    })
}
