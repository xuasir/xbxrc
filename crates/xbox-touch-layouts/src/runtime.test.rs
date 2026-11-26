use super::*;
use serde_json::json;

fn document() -> Value {
    json!({"_xbxrcRuntime": {
        "layout": {"content":{"right":{"inner":[{
            "type":"button","action":"gamepadA",
            "visible":{"$ref":"runtime-context.json#/state/fireVisible"}
        }]}},"styles":{"colors":{"default":{"accent":"#0f0"}}}},
        "context":{"state":{"fireVisible":true,"score":1},"styles":{"colors":{"default":{"base":"#f00","accent":"#fff"}}}},
        "assets":{}
    }})
}

#[test]
fn state_patch_keeps_successful_operations_and_merges_layout_colors() {
    let result = resolve_runtime_document(
        &document().to_string(),
        None,
        Some(
            &json!([
                {"op":"replace","path":"fireVisible","value":false},
                {"op":"remove","path":"/missing"},
                {"op":"copy","from":"/score","path":"/copy"},
                {"op":"move","from":"/copy","path":"/moved"},
                {"op":"test","path":"/moved","value":1}
            ])
            .to_string(),
        ),
    )
    .unwrap();
    let result: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["content"]["right"]["inner"][0]["visible"], false);
    assert_eq!(result["styles"]["colors"]["default"]["accent"], "#0f0");
    assert_eq!(result["styles"]["colors"]["default"]["base"], "#f00");
    assert_eq!(result["_xbxrcRuntime"]["context"]["state"]["moved"], 1);
    assert_eq!(
        result["_xbxrcRuntime"]["warnings"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn applying_current_state_to_a_second_layout_and_invalid_patches_are_bounded() {
    let result = resolve_runtime_document(
        &document().to_string(),
        Some(r#"{"fireVisible":false}"#),
        None,
    )
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&result).unwrap()["content"]["right"]["inner"][0]["visible"],
        false
    );
    assert!(resolve_runtime_document(&document().to_string(), None, Some("{}")).is_err());
    assert!(resolve_runtime_document(&document().to_string(), Some("{}"), None).is_err());
    assert!(
        resolve_runtime_document(&document().to_string(), None, Some(&" ".repeat(65537))).is_err()
    );
}
