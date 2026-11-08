use super::*;
use serde_json::json;

fn web_token(expiry_ms: i64) -> Value {
    json!({ "data": {
        "Token": "fixture-token",
        "DisplayClaims": { "xui": [{ "uhs": "fixture-hash" }] },
        "NotAfter": chrono::DateTime::from_timestamp_millis(expiry_ms).unwrap().to_rfc3339(),
    } })
}

#[test]
fn expired_web_credentials_are_excluded_from_data_fallback() {
    let expired_at = parse_datetime_to_timestamp_millis("2026-11-07T16:10:59.0645646Z").unwrap();
    let now = parse_datetime_to_timestamp_millis("2026-11-08T01:04:18Z").unwrap();
    assert!(resolve_valid_web_api_tokens(&web_token(expired_at), now).is_none());
}

#[test]
fn valid_web_credentials_support_wrapped_and_raw_snapshots() {
    let now = 1_794_000_000_000;
    let wrapped = web_token(now + 120_000);
    let expected = Some(("fixture-hash".to_string(), "fixture-token".to_string()));
    assert_eq!(resolve_valid_web_api_tokens(&wrapped, now), expected);
    assert_eq!(
        resolve_valid_web_api_tokens(&wrapped["data"], now),
        expected
    );
}

#[test]
fn credentials_reserve_expiry_margin_for_in_flight_requests() {
    let now = 1_794_000_000_000;
    assert!(resolve_valid_web_api_tokens(&web_token(now + 60_000), now).is_none());
    assert!(resolve_valid_web_api_tokens(&web_token(now + 60_001), now).is_some());
}

#[test]
fn incomplete_credentials_and_unknown_expiry_require_recovery() {
    let now = 1_794_000_000_000;
    let mut value = web_token(now + 120_000);
    value["data"]["NotAfter"] = json!("invalid-time");
    assert!(resolve_valid_web_api_tokens(&value, now).is_none());
    value["data"].as_object_mut().unwrap().remove("NotAfter");
    assert!(resolve_valid_web_api_tokens(&value, now).is_none());
    for (key, invalid) in [("Token", json!(" ")), ("Token", Value::Null)] {
        let mut value = web_token(now + 120_000);
        value["data"][key] = invalid;
        assert!(resolve_valid_web_api_tokens(&value, now).is_none());
    }
    let mut value = web_token(now + 120_000);
    value["data"]["DisplayClaims"]["xui"][0]["uhs"] = json!("");
    assert!(resolve_valid_web_api_tokens(&value, now).is_none());
}

#[test]
fn sisu_reuse_follows_its_expiry_after_downstream_cache_updates() {
    let now = 1_794_000_000_000;
    let mut token: SisuTokenData = serde_json::from_value(json!({
        "DeviceToken": "fixture-device",
        "TitleToken": { "Token": "fixture-title", "DisplayClaims": {} },
        "UserToken": {
            "Token": "fixture-user",
            "NotAfter": chrono::DateTime::from_timestamp_millis(now + 120_000).unwrap().to_rfc3339(),
            "DisplayClaims": {},
        },
        "AuthorizationToken": { "Token": "fixture-auth", "DisplayClaims": {} },
    })).unwrap();
    assert!(can_reuse_sisu_user_token(&token, now));
    assert!(!can_reuse_sisu_user_token(&token, now + 60_000));
    assert!(!can_reuse_sisu_user_token(&token, now + 120_001));
    token.user_token.not_after = None;
    assert!(!can_reuse_sisu_user_token(&token, now));
}
