use super::*;
use serde_json::json;

#[test]
fn configured_cloud_sign_in_sends_forwarded_region_for_both_offerings() {
    for offering in ["xgpuweb", "xgpuwebf2p"] {
        let payload = json!({"token":"fixture-token","offeringId":offering});
        let headers = streaming_login_headers(offering, " 4.2.2.2 ", &payload).unwrap();
        let request = reqwest::Client::new()
            .post(format!(
                "https://{offering}.gssv-play-prod.xboxlive.com/v2/login/user"
            ))
            .headers(headers)
            .json(&payload)
            .build()
            .unwrap();
        assert_eq!(request.headers()["X-Forwarded-For"], "4.2.2.2");
        assert_eq!(request.headers()["x-cosmos-ip"], "4.2.2.2");
        assert_eq!(request.headers()["x-gssv-client"], "XboxComBrowser");
        assert_eq!(
            request.body().unwrap().as_bytes().unwrap(),
            serde_json::to_vec(&payload).unwrap()
        );
    }
}

#[test]
fn automatic_cloud_and_explicit_home_keep_their_existing_header_scope() {
    let payload = json!({"token":"fixture-token"});
    let automatic = streaming_login_headers("xgpuweb", " \n ", &payload).unwrap();
    assert!(!automatic.contains_key("X-Forwarded-For"));
    assert!(!automatic.contains_key("x-cosmos-ip"));
    let home = streaming_login_headers("xhome", "4.2.2.2", &payload).unwrap();
    assert!(!home.contains_key("X-Forwarded-For"));
    assert_eq!(home["x-cosmos-ip"], "4.2.2.2");
    assert!(
        streaming_login_headers("xgpuweb", "4.2.2.2\r\nprivate-header: secret", &payload).is_err()
    );
}

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
