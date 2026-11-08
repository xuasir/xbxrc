use super::extract_consoles;
use serde_json::json;

#[test]
fn accepts_empty_lists_in_supported_envelopes() {
    for payload in [
        json!([]),
        json!({"consoles": []}),
        json!({"data": {"devices": []}}),
    ] {
        assert!(extract_consoles(&payload).unwrap().is_empty());
    }
}

#[test]
fn rejects_missing_or_invalid_console_lists() {
    for payload in [
        json!({}),
        json!({"error": "secret-body"}),
        json!({"consoles": null}),
        json!({"results": [null]}),
        json!({"devices": [{"id": " "}]}),
    ] {
        let error = extract_consoles(&payload).unwrap_err();
        assert_eq!(error.safe_summary()["errorKind"], "parse");
        assert!(!error.to_string().contains("secret-body"));
    }
}

#[test]
fn maps_console_identity_and_storage_in_nested_payload() {
    let hosts = extract_consoles(&json!({"data": {"consoles": [{
        "deviceId": " console-1 ", "name": " Living room ", "remoteManagementEnabled": true,
        "storageDevices": [{"storageDeviceId": "disk-1", "freeSpaceBytes": 42}]
    }]}}))
    .unwrap();
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].device_id.as_deref(), Some("console-1"));
    assert_eq!(hosts[0].name.as_deref(), Some("Living room"));
    assert_eq!(hosts[0].remote_management_enabled, Some(true));
    assert_eq!(
        hosts[0].storage_devices.as_ref().unwrap()[0].free_space_bytes,
        Some(42)
    );
}

#[test]
fn maps_real_xccs_ok_envelope_and_float_storage_values() {
    let hosts = extract_consoles(&json!({"status":{"errorCode":"OK"},"result":[{
        "id":"console", "name":"Xbox", "powerState":"ConnectedStandby",
        "remoteManagementEnabled":true,
        "storageDevices":[{"storageDeviceId":"disk","freeSpaceBytes":42.0,"totalSpaceBytes":100.0}]
    }]}))
    .unwrap();
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].power_state.as_deref(), Some("ConnectedStandby"));
    let disk = &hosts[0].storage_devices.as_ref().unwrap()[0];
    assert_eq!(disk.free_space_bytes, Some(42));
    assert_eq!(disk.total_space_bytes, Some(100));
}
