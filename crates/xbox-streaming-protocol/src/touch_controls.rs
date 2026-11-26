#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XboxTouchControlCommand {
    pub kind: String,
    pub layout_id: Option<String>,
    pub patch_base64: Option<String>,
}

pub fn build_xbox_touch_capability_payloads(enabled: bool) -> Vec<String> {
    vec![
        crate::stream_data_channel::build_message_payload(
            "/streaming/characteristics/clientdevicecapabilities",
            serde_json::json!({"maxTouchBundleLayoutVersion":"4.1.0.0","maxTouchBundleManifestVersion":"2.0.0.0"}),
        ),
        crate::stream_data_channel::build_message_payload(
            "/streaming/characteristics/touchinputenabledchanged",
            serde_json::json!({"touchInputEnabled":enabled}),
        ),
    ]
}

pub fn parse_xbox_touch_control_command(payload: &str) -> Option<XboxTouchControlCommand> {
    if payload.len() > 128 * 1024 {
        return None;
    }
    let envelope: serde_json::Value = serde_json::from_str(payload).ok()?;
    if envelope["type"] != "Message" {
        return None;
    }
    let target = envelope["target"]
        .as_str()?
        .strip_prefix("/streaming/touchcontrols/")?;
    let content = envelope["content"].as_str()?;
    let mut result = XboxTouchControlCommand {
        kind: target.into(),
        layout_id: None,
        patch_base64: None,
    };
    match target {
        "showlayout" => result.layout_id = Some(content.into()),
        "showlayoutv2" | "patchstate" => {
            let data: serde_json::Value = serde_json::from_str(content).ok()?;
            if target == "showlayoutv2" {
                result.layout_id = Some(data["layoutId"].as_str()?.into());
            }
            result.patch_base64 = data["patch"].as_str().map(str::to_owned);
            if target == "patchstate" && result.patch_base64.is_none() {
                return None;
            }
        }
        "showtitledefault" | "hide" => {}
        _ => return None,
    }
    if result.layout_id.as_ref().is_some_and(|id| id.len() > 256)
        || result
            .patch_base64
            .as_ref()
            .is_some_and(|patch| patch.len() > 90 * 1024)
    {
        return None;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_commands_require_known_targets_and_message_envelopes() {
        let message = serde_json::json!({"type":"Message","target":"/streaming/touchcontrols/showlayoutv2","content":serde_json::json!({"layoutId":"gameplay","patch":"W10="}).to_string()});
        let command = parse_xbox_touch_control_command(&message.to_string()).unwrap();
        assert_eq!(command.layout_id.as_deref(), Some("gameplay"));
        assert_eq!(command.patch_base64.as_deref(), Some("W10="));
        assert!(parse_xbox_touch_control_command("{}").is_none());
        assert!(parse_xbox_touch_control_command(
            &message.to_string().replace("Message", "Transaction")
        )
        .is_none());
    }
}
