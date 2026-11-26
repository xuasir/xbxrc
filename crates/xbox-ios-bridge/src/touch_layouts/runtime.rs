use crate::XboxBridgeError;

#[uniffi::export]
pub fn touch_layout_protocol_information_json() -> String {
    xbox_touch_layouts::protocol_information_json()
}

#[uniffi::export]
pub fn compile_touch_layout_json(
    document_json: String,
    layout_id: String,
    format: String,
    available_assets: Vec<String>,
    state_json: Option<String>,
    patch_json: Option<String>,
) -> Result<String, XboxBridgeError> {
    xbox_touch_layouts::compile_layout_json(
        &document_json,
        &layout_id,
        &format,
        &available_assets,
        state_json.as_deref(),
        patch_json.as_deref(),
    )
    .map_err(|error| XboxBridgeError::InvalidData(error.to_string()))
}

#[uniffi::export]
pub fn resolve_touch_layout_runtime_json(
    document_json: String,
    state_json: Option<String>,
    patch_json: Option<String>,
) -> Result<String, XboxBridgeError> {
    xbox_touch_layouts::resolve_runtime_document(
        &document_json,
        state_json.as_deref(),
        patch_json.as_deref(),
    )
    .map_err(|error| XboxBridgeError::InvalidData(error.to_string()))
}
