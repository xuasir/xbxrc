use super::types::TouchProtocolIssue;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Registry {
    pub ir_version: u32,
    pub revision: String,
    pub schema: Value,
    pub icon_names: BTreeSet<String>,
    pub system_colors: BTreeMap<String, String>,
    pub control_definitions: BTreeMap<String, String>,
    pub control_styles: BTreeMap<String, String>,
    pub gamepad_actions: BTreeSet<String>,
    pub axis_inputs: BTreeSet<String>,
    pub axis_outputs: BTreeSet<String>,
    pub versions: BTreeMap<String, BTreeSet<String>>,
}

pub(super) fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        serde_json::from_str(include_str!("../../protocol/contract.json"))
            .expect("TAK generated protocol contract")
    })
}

pub fn protocol_information_json() -> String {
    let catalog = registry();
    json!({"irVersion":catalog.ir_version,"revision":catalog.revision,
        "iconNames":catalog.icon_names,"systemColors":catalog.system_colors})
    .to_string()
}

pub fn protocol_revision() -> &'static str {
    &registry().revision
}

impl Registry {
    pub fn definition(&self, name: &str) -> &Value {
        &self.schema["$defs"][name]
    }

    pub fn properties(&self, rule: &Value, discriminator: Option<&str>) -> Map<String, Value> {
        self.properties_at(rule, discriminator, 0)
    }

    fn properties_at(
        &self,
        rule: &Value,
        discriminator: Option<&str>,
        depth: usize,
    ) -> Map<String, Value> {
        if depth > 32 {
            return Map::new();
        }
        if let Some(reference) = rule["$ref"]
            .as_str()
            .and_then(|s| s.strip_prefix("#/$defs/"))
        {
            return self.properties_at(self.definition(reference), discriminator, depth + 1);
        }
        let mut result = rule["properties"].as_object().cloned().unwrap_or_default();
        for field in ["anyOf", "oneOf", "allOf"] {
            for branch in rule[field].as_array().into_iter().flatten() {
                let properties = self.properties_at(branch, discriminator, depth + 1);
                if let (Some(expected), Some(actual)) = (
                    discriminator,
                    properties.get("type").and_then(|r| r["const"].as_str()),
                ) {
                    if expected != actual {
                        continue;
                    }
                }
                result.extend(properties);
            }
        }
        result.remove("$ref");
        result
    }

    pub fn accepts(&self, rule: &Value, value: &Value) -> bool {
        self.accepts_at(rule, value, 0)
    }

    fn accepts_at(&self, rule: &Value, value: &Value, depth: usize) -> bool {
        if depth > 32 {
            return false;
        }
        if let Some(reference) = rule["$ref"]
            .as_str()
            .and_then(|s| s.strip_prefix("#/$defs/"))
        {
            return self.accepts_at(self.definition(reference), value, depth + 1);
        }
        if let Some(variants) = rule["anyOf"]
            .as_array()
            .or_else(|| rule["oneOf"].as_array())
        {
            return variants
                .iter()
                .any(|r| self.accepts_at(r, value, depth + 1));
        }
        if let Some(expected) = rule.get("const") {
            if expected != value {
                return false;
            }
        }
        if let Some(values) = rule["enum"].as_array() {
            if !values.contains(value) {
                return false;
            }
        }
        if let Some(kind) = rule["type"].as_str() {
            let valid = match kind {
                "object" => value.is_object(),
                "array" => value.is_array(),
                "string" => value.is_string(),
                "boolean" => value.is_boolean(),
                "number" => value.as_f64().is_some_and(f64::is_finite),
                "integer" => value
                    .as_f64()
                    .is_some_and(|n| n.is_finite() && n.fract() == 0.0),
                "null" => value.is_null(),
                _ => true,
            };
            if !valid {
                return false;
            }
        }
        if let Some(number) = value.as_f64() {
            if rule["minimum"].as_f64().is_some_and(|min| number < min)
                || rule["maximum"].as_f64().is_some_and(|max| number > max)
                || rule["exclusiveMinimum"]
                    .as_f64()
                    .is_some_and(|min| number <= min)
                || rule["exclusiveMaximum"]
                    .as_f64()
                    .is_some_and(|max| number >= max)
            {
                return false;
            }
        }
        true
    }
}

pub(super) fn warning(
    issues: &mut Vec<TouchProtocolIssue>,
    path: &str,
    capability: &str,
    message: &str,
) {
    issues.push(TouchProtocolIssue {
        severity: "warning".into(),
        path: path.into(),
        message: message.into(),
        capability: capability.into(),
    });
}
