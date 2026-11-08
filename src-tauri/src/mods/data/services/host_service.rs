use crate::mods::data::session_resolver::resolve_web_token_claims;
use crate::mods::data::types::{DataHostStorageDeviceSummary, DataHostSummary, DataSessionContext};
use serde_json::Value;
use xbox_webapi::{library_u64, SmartglassApi, WebApiError};

pub struct HostService;

impl HostService {
    pub fn new() -> Self {
        Self
    }

    // 主机管理列表复用 Smartglass 协议。
    pub async fn get_hosts(
        &self,
        session: &DataSessionContext,
    ) -> Result<Vec<DataHostSummary>, WebApiError> {
        let Some(claims) = resolve_web_token_claims(&session.web_token) else {
            return Err(WebApiError::auth("登录信息暂不可用，请重新登录"));
        };
        let smartglass = SmartglassApi::new(claims.uhs, claims.user_token);

        let payload = smartglass.get_consoles_list().await?;
        extract_consoles(&payload)
    }
}

fn extract_consoles(raw: &Value) -> Result<Vec<DataHostSummary>, WebApiError> {
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
            if array.is_empty() || !consoles.is_empty() {
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

    visit(raw, 0, &mut visited).ok_or_else(|| WebApiError::parse("invalid console list response"))
}

fn to_host_summary(value: &Value) -> Option<DataHostSummary> {
    let object = value.as_object()?;

    let identity = |key| {
        object
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let id = identity("id");
    let device_id = identity("deviceId");
    let server_id = identity("serverId");
    let name = identity("name");
    let device_name = identity("deviceName");

    // 兼容服务端各版本的主机身份字段。
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
                        free_space_bytes: library_u64(o.get("freeSpaceBytes")),
                        free_bytes: library_u64(o.get("freeBytes")),
                        total_space_bytes: library_u64(o.get("totalSpaceBytes")),
                        total_bytes: library_u64(o.get("totalBytes")),
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

#[cfg(test)]
#[path = "host_service.test.rs"]
mod tests;
