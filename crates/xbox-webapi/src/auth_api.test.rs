use super::*;
use serde_json::json;

#[test]
fn transfer_responses_accept_access_and_legacy_tokens_and_preserve_refresh_rotation() {
    for (response, token, refresh) in [
        (
            json!({"access_token":"access", "refresh_token":"rotated"}),
            "access",
            Some("rotated"),
        ),
        (json!({"lpt":"legacy"}), "legacy", None),
        (
            json!({"access_token":" ", "lpt":"legacy", "refresh_token":""}),
            "legacy",
            None,
        ),
    ] {
        let parsed = parse_cloud_transfer_response(&response).unwrap();
        assert_eq!(parsed.transfer_token, token);
        assert_eq!(parsed.refresh_token.as_deref(), refresh);
    }
}

#[test]
fn invalid_transfer_responses_return_a_retryable_error_with_safe_details() {
    for response in [
        json!({"access_token":" ", "refresh_token":"private-refresh"}),
        json!({"access_token":null, "lpt":"", "refresh_token":"private-refresh"}),
    ] {
        let error = parse_cloud_transfer_response(&response).unwrap_err();
        assert!(matches!(error, WebApiError::Parse { .. }));
        assert!(!error.to_string().contains("private-refresh"));
        assert!(!error.safe_summary().to_string().contains("private-refresh"));
    }
}
