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

fn stream_token(created_at: i64, seconds: i64, expiry_ms: Option<i64>) -> Value {
    let gs_token = expiry_ms
        .map(|expiry| {
            let payload =
                URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json!({"exp": expiry / 1000})).unwrap());
            format!("header.{payload}.signature")
        })
        .unwrap_or_else(|| "opaque-fixture-token".into());
    json!({"_objectCreateTime": created_at, "data": {"durationInSeconds": seconds, "gsToken": gs_token}})
}

#[test]
fn each_scope_requires_its_own_valid_credentials() {
    let now = 1_794_000_000_000;
    let web = web_token(now + 300_000);
    let tokens = json!({
        "xHomeToken": stream_token(now - 14_500_000, 14_400, None),
        "xCloudToken": stream_token(now, 14_400, None),
    });
    assert!(resolve_valid_session_for(&web, &tokens, AuthTokenScope::Home, now).is_none());
    assert_eq!(
        resolve_valid_session_for(&web, &tokens, AuthTokenScope::Cloud, now)
            .unwrap()
            .app_level,
        2
    );
    assert!(resolve_valid_session_for(&web, &tokens, AuthTokenScope::Web, now).is_some());
    assert!(resolve_valid_session_for(&web, &json!({}), AuthTokenScope::Home, now).is_none());
    assert_eq!(
        resolve_valid_session_for(&web, &json!({}), AuthTokenScope::Web, now)
            .unwrap()
            .app_level,
        0
    );
    let expired_web = web_token(now - 1000);
    for scope in [
        AuthTokenScope::Web,
        AuthTokenScope::Home,
        AuthTokenScope::Cloud,
    ] {
        assert!(resolve_valid_session_for(&expired_web, &tokens, scope, now).is_none());
    }
}

#[test]
fn stream_validity_uses_earliest_known_expiry_and_reserves_request_margin() {
    let now = 1_794_000_000_000;
    for token in [
        stream_token(now, 120, Some(now + 60_000)),
        stream_token(now, 60, Some(now + 120_000)),
    ] {
        assert_eq!(resolve_stream_token_expiry_ms(&token), Some(now + 60_000));
        assert!(!is_stream_token_valid_at(&token, now));
    }
    assert!(is_stream_token_valid_at(
        &stream_token(now, 61, Some(now + 120_000)),
        now
    ));
    let jwt_only =
        json!({"gsToken": stream_token(now, 120, Some(now + 120_000))["data"]["gsToken"]});
    assert!(is_stream_token_valid_at(&jwt_only, now));
    for gs_token in [Value::Null, json!(""), json!(" ")] {
        let mut token = stream_token(now, 120, None);
        token["data"]["gsToken"] = gs_token;
        assert!(!is_stream_token_valid_at(&token, now));
    }
    assert!(!is_stream_token_valid_at(
        &json!({"gsToken": "opaque"}),
        now
    ));
}

#[test]
fn scoped_refresh_preserves_other_target_credentials_and_replaces_failed_target() {
    let previous = json!({"xHomeToken": "old-home", "xCloudToken": "cloud", "_diagnostics": {"xCloudToken": "old-failure"}});
    let home = merge_stream_tokens(
        Some(previous.clone()),
        &json!({"xHomeToken": "new-home"}),
        AuthTokenScope::Home,
        true,
    );
    assert_eq!(home["xHomeToken"], "new-home");
    assert_eq!(home["xCloudToken"], "cloud");
    let web = merge_stream_tokens(
        Some(previous.clone()),
        &json!({}),
        AuthTokenScope::Web,
        true,
    );
    assert_eq!(web, previous);
    let cloud = merge_stream_tokens(
        Some(previous.clone()),
        &json!({"_diagnostics": {"xCloudToken": "new-failure"}}),
        AuthTokenScope::Cloud,
        true,
    );
    assert!(cloud.get("xCloudToken").is_none());
    assert_eq!(cloud["xHomeToken"], "old-home");
    assert_eq!(cloud["_diagnostics"]["xCloudToken"], "new-failure");
    let switched = merge_stream_tokens(
        Some(previous),
        &json!({"xHomeToken": "new-account-home"}),
        AuthTokenScope::Home,
        false,
    );
    assert_eq!(switched, json!({"xHomeToken": "new-account-home"}));
}

#[test]
fn cache_merging_uses_account_claims_instead_of_rotating_access_token() {
    let mut previous = web_token(1_794_000_120_000);
    previous["data"]["DisplayClaims"]["xui"][0]["xid"] = json!("account-one");
    let mut next = previous.clone();
    next["data"]["Token"] = json!("rotated-access");
    next["data"]["DisplayClaims"]["xui"][0]["uhs"] = json!("new-hash");
    assert!(same_web_account(&previous, &next));
    next["data"]["DisplayClaims"]["xui"][0]["xid"] = json!("account-two");
    assert!(!same_web_account(&previous, &next));
}
