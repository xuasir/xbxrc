use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TouchProtocolIssue {
    pub severity: String,
    pub path: String,
    pub message: String,
    pub capability: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CompiledAction {
    Gamepad {
        name: String,
    },
    Layer {
        target: String,
    },
    Group {
        items: Vec<CompiledAction>,
    },
    Segmented {
        items: Vec<CompiledAction>,
    },
    Turbo {
        action: Box<CompiledAction>,
        interval: f64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompiledCurve {
    pub kind: String,
    pub lower: f64,
    pub upper: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledAxis {
    pub input: String,
    pub output: String,
    pub deadzone: f64,
    pub radial: bool,
    pub sensitivity: f64,
    pub response_curve: Option<CompiledCurve>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TouchCompiledControl {
    pub id: String,
    pub kind: String,
    pub action: Option<CompiledAction>,
    pub pull_action: Option<CompiledAction>,
    pub axis: Vec<CompiledAxis>,
    pub axis_up: Vec<CompiledAxis>,
    pub axis_down: Vec<CompiledAxis>,
    pub activation_type: String,
    pub styles: Value,
    pub children: BTreeMap<String, TouchCompiledControl>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledTouchLayout {
    pub ir_version: u32,
    pub protocol_revision: String,
    pub id: String,
    pub name: String,
    pub author: Option<String>,
    pub format: String,
    pub schema: String,
    pub content: Value,
    pub controls: BTreeMap<String, TouchCompiledControl>,
    pub color_palette: Value,
    pub available_assets: Vec<String>,
    pub runtime_document: String,
    pub required_capabilities: Vec<String>,
    pub control_types: Vec<String>,
    pub behaviors: Vec<String>,
    pub issues: Vec<TouchProtocolIssue>,
    pub is_runnable: bool,
    pub is_fully_compatible: bool,
    pub uses_standard_fallback: bool,
}
