use xbox_streaming_protocol::parse_xbox_touch_control_command;

#[derive(Clone, Debug, uniffi::Record)]
pub struct XboxStreamTouchLayoutCommand {
    pub kind: String,
    pub layout_id: Option<String>,
    pub patch_base64: Option<String>,
}

#[uniffi::export]
pub fn parse_stream_touch_layout_command(payload: String) -> Option<XboxStreamTouchLayoutCommand> {
    parse_xbox_touch_control_command(&payload).map(|c| XboxStreamTouchLayoutCommand {
        kind: c.kind,
        layout_id: c.layout_id,
        patch_base64: c.patch_base64,
    })
}

#[uniffi::export]
pub fn stream_touch_capability_payloads(enabled: bool) -> Vec<String> {
    xbox_streaming_protocol::build_xbox_touch_capability_payloads(enabled)
}
