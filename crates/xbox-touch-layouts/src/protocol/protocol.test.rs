use super::*;
use serde_json::{json, Value};
use std::io::{Cursor, Read};

fn compile(value: Value) -> CompiledTouchLayout {
    compile_layout(&value.to_string(), "test", "official", &[], None, None).unwrap()
}

#[test]
fn complete_frozen_community_library_uses_one_compiler() {
    let bytes = include_bytes!("../../../../dev-docs/validation/2026-11-26-ios-tak-protocol-engine-plan/community-source.zip");
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut templates = 0;
    let mut layouts = 0;
    let mut gta = 0;
    let mut inventory = Vec::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).unwrap();
        let name = file.name().to_owned();
        if !name.starts_with("touch-layouts/layouts/") {
            continue;
        }
        let mut text = String::new();
        file.read_to_string(&mut text).unwrap();
        let root: Value = serde_json::from_str(&text).unwrap();
        templates += 1;
        for (id, value) in root["layouts"].as_object().unwrap() {
            let layout =
                compile_layout(&value.to_string(), id, "community", &[], None, None).unwrap();
            inventory.push(json!({
                "source":name,"id":id,"protocolRevision":layout.protocol_revision,
                "controls":layout.controls.len(),"controlTypes":layout.control_types,
                "requiredCapabilities":layout.required_capabilities,"behaviors":layout.behaviors,
                "isRunnable":layout.is_runnable,"isFullyCompatible":layout.is_fully_compatible,
                "usesStandardFallback":layout.uses_standard_fallback,"issues":layout.issues
            }));
            assert!(layout.is_runnable, "{name}/{id}: {:?}", layout.issues);
            assert!(
                !layout.issues.iter().any(|i| i.capability == "render.icon"),
                "{name}/{id}: {:?}",
                layout.issues
            );
            assert!(
                !layout.uses_standard_fallback,
                "{name}/{id}: {:?}",
                layout.issues
            );
            if name.ends_with("gta-vi.json") {
                gta += 1;
                assert_eq!(layout.controls.len(), 15);
                assert!(layout
                    .controls
                    .values()
                    .any(|c| c.styles.to_string().contains("leftTrigger")));
            }
            layouts += 1;
        }
    }
    assert_eq!((templates, layouts, gta), (75, 85, 1));
    println!(
        "TAK corpus: {templates} templates / {layouts} layouts; GTA6 uses the common compiler"
    );
    println!(
        "TAK_PROTOCOL_CORPUS {}",
        json!({"templates":templates,"layouts":layouts,"items":inventory})
    );
}

#[test]
fn visual_extensions_fall_back_and_known_inputs_survive() {
    let layout = compile(json!({"content":{"right":{"inner":[
        {"type":"button","action":["gamepadA","mouseLeft"],"styles":{"default":{"faceImage":{"type":"icon","value":"futureGlyph"},"futureEffect":true}}},
        {"type":"futureControl"}, {"type":"button","action":"gamepadB"}
    ]}}}));
    assert!(layout.is_runnable);
    assert!(!layout.is_fully_compatible);
    assert!(!layout.uses_standard_fallback);
    assert!(layout.issues.iter().any(|i| i.capability == "render.icon"));
    assert!(layout.controls.values().any(|c| c
        .action
        .as_ref()
        .is_some_and(|a| serde_json::to_string(a).unwrap().contains("gamepadA"))));
    assert_eq!(layout.content["right"]["inner"][1]["type"], "blank");
}

#[test]
fn only_deferred_outputs_use_standard_fallback() {
    let layout =
        compile(json!({"content":{"right":{"inner":[{"type":"button","action":"mouseLeft"}]}}}));
    assert!(layout.is_runnable && layout.uses_standard_fallback);
    assert!(layout.controls.values().any(|c| c.kind == "joystick"));
}

#[test]
fn historical_fields_convert_before_typed_input_compilation() {
    let layout = compile(
        json!({"$schema":"https://schema.play.xbox.com/layout/v1.2/layout.json",
        "content":{"left":{"inner":[{"type":"joystick","axis":{"input":"axisXY","output":"leftJoystick"},"icon":"leftTrigger","threshold":0.2,"action":"rightJoystickUp"}]},
        "upper":{"left":[{"type":"button","action":"gamepadA"}],"right":[{"type":"button","action":"menu"}]}}}),
    );
    assert!(layout.is_runnable);
    assert_eq!(layout.content["left"]["inner"][0]["actionThreshold"], 0.2);
    assert_eq!(layout.content["upper"]["right"][0]["action"], "menu");
    let control = &layout.controls["/content/left/inner/0"];
    assert!(serde_json::to_string(&control.action)
        .unwrap()
        .contains("rightJoystickDown"));
    assert_eq!(
        control.styles["default"]["knob"]["faceImage"]["value"],
        "leftTrigger"
    );
}

#[test]
fn invalid_dynamic_input_state_keeps_transaction_gate() {
    let source = json!({"_xbxrcRuntime":{"layout":{"content":{"right":{"inner":[
        {"type":"button","action":"gamepadA","visible":{"$ref":"runtime-context.json#/state/show"}}
    ]}}},"context":{"state":{"show":true}},"assets":{}}});
    let patched = compile_layout(
        &source.to_string(),
        "test",
        "official",
        &[],
        None,
        Some(r#"[{"op":"replace","path":"/show","value":"yes"}]"#),
    )
    .unwrap();
    assert!(!patched.is_runnable);
    assert!(patched
        .issues
        .iter()
        .any(|i| i.path.ends_with("/visible") && i.severity == "unsupported"));
}

#[test]
fn protocol_numeric_domains_and_defaults_match_tak() {
    let layout = compile(json!({"content":{"left":{"inner":[
        {"type":"joystick","action":"gamepadA","actionThreshold":2.5,
         "axis":{"input":"axisXY","output":"leftJoystick",
                 "deadzone":{"threshold":-0.3,"radial":false},"sensitivity":200}},
        {"type":"joystick","action":"gamepadB",
         "axis":{"input":"axisXY","output":"rightJoystick"}}
    ]}}}));
    assert!(layout.is_fully_compatible, "{:?}", layout.issues);
    assert_eq!(layout.content["left"]["inner"][0]["actionThreshold"], 2.5);
    assert_eq!(layout.content["left"]["inner"][1]["actionThreshold"], 0.0);
    let axis = &layout.controls["/content/left/inner/0"].axis[0];
    assert_eq!(axis.deadzone, -0.3);
    assert_eq!(axis.sensitivity, 200.0);
    let invalid = compile(json!({"content":{"left":{"inner":[
        {"type":"joystick","axis":{"input":"axisXY","output":"leftJoystick","sensitivity":0}}
    ]}}}));
    assert_eq!(
        invalid.controls["/content/left/inner/0"].axis[0].sensitivity,
        1.0
    );
    assert!(invalid
        .issues
        .iter()
        .any(|issue| issue.path.ends_with("/sensitivity")));
}

#[test]
fn resource_and_nested_document_budgets_are_enforced() {
    let mut value = json!({});
    for _ in 0..65 {
        value = json!({"child":value});
    }
    assert!(compile_layout(&value.to_string(), "test", "official", &[], None, None).is_err());
    assert!(compile_layout(
        &" ".repeat(crate::MAX_JSON_BYTES + 1),
        "test",
        "official",
        &[],
        None,
        None
    )
    .is_err());
    let info: Value = serde_json::from_str(&protocol_information_json()).unwrap();
    assert_eq!(info["iconNames"].as_array().unwrap().len(), 234);
}

#[test]
fn optional_references_preserve_other_input_bindings_and_original_fields() {
    let layout = compile(
        json!({"definitions":{"action":"gamepadA"},"content":{"right":{"inner":[{
            "type":"button","action":{"$ref":"#/definitions/action"},
            "extension":{"$ref":"#/definitions/missing"}
        }]}}}),
    );
    assert!(layout.is_runnable && !layout.uses_standard_fallback);
    assert!(layout.controls.values().any(|c| c.action.is_some()));
    assert!(layout.issues.iter().any(|i| i.capability == "reference"));
    assert!(layout.content["right"]["inner"][0]
        .get("extension")
        .is_none());
}

#[test]
fn pull_only_buttons_and_draw_only_buttons_follow_the_protocol() {
    let layout = compile(json!({"content":{"right":{"inner":[
        {"type":"button","pullAction":"gamepadA"},
        {"type":"button","styles":{"default":{"faceImage":{"type":"icon","value":"leftTrigger"}}}}
    ]}}}));
    assert!(layout.is_runnable && !layout.uses_standard_fallback);
    assert_eq!(layout.content["right"]["inner"][0]["visible"], true);
    assert_eq!(layout.content["right"]["inner"][1]["visible"], true);
    assert!(layout.controls["/content/right/inner/0"]
        .pull_action
        .is_some());
}
