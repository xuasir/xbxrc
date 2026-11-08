use super::{console_query_url, validate_remote_management_response};
use serde_json::json;

#[test]
fn encodes_console_identity_in_query_and_path() {
    let query = console_query_url("lists/installedApps", " Console:one&extra=two ", true).unwrap();
    assert_eq!(
        query.query_pairs().collect::<Vec<_>>(),
        vec![("deviceId".into(), "Console:one&extra=two".into())]
    );
    let path = console_query_url("consoles", "one/two?secret=x", false).unwrap();
    assert_eq!(path.path(), "/consoles/one%2Ftwo%3Fsecret=x");
    assert!(path.query().is_none());
    assert!(console_query_url("consoles", " ", false).is_err());
}

#[test]
fn interprets_service_status_before_accepting_http_success() {
    for payload in [
        json!({}),
        json!({"status":{"errorCode":0}}),
        json!({"status":{"errorCode":"Success"}}),
        json!({"status":{"errorCode":"OK"},"result":[{"id":"console","name":"Xbox","powerState":"ConnectedStandby"}]}),
        json!({"data":{"status":{"errorCode":"ok"},"result":[]}}),
    ] {
        validate_remote_management_response(&payload).unwrap();
    }
    for (code, expected) in [
        ("401", 401),
        ("Unauthorized", 401),
        ("403", 403),
        ("Forbidden", 403),
        ("TooManyRequests", 429),
    ] {
        let error = validate_remote_management_response(
            &json!({"data": {"status":{"errorCode":code,"errorMessage":"secret-body"}}}),
        )
        .unwrap_err();
        assert_eq!(error.to_status_code(), Some(expected));
        assert!(!error.to_string().contains("secret-body"));
    }
    assert!(
        validate_remote_management_response(&json!({"status":{"errorCode":"secret-code"}}))
            .is_err()
    );
    assert!(validate_remote_management_response(&json!({"status":{"errorCode":true}})).is_err());
}
