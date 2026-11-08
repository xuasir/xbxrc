use super::*;

#[test]
fn protocol_diagnostics_keep_fixed_codes_and_filter_unknown_bodies() {
    assert_eq!(
        WebApiError::parse("invalid remote management status").safe_summary()["protocolErrorCode"],
        "invalidConsoleServiceStatus"
    );
    assert_eq!(
        WebApiError::parse("missing people array").safe_summary()["protocolErrorCode"],
        "missingPeople"
    );
    let summary =
        WebApiError::parse("fixture-secret https://example.com/?xuid=private").safe_summary();
    assert!(summary.get("protocolErrorCode").is_none());
    assert!(!summary.to_string().contains("private"));
    assert!(!summary.to_string().contains("fixture-secret"));
}

#[test]
fn service_rejections_preserve_safe_codes_and_discard_identity_payloads() {
    let error = WebApiError::http_response(
        403,
        r#"{"code":"Forbidden","message":"UnsupportedCountry","token":"fixture-secret-token","url":"https://example.test/?xuid=fixture-identity"}"#,
    );
    assert_eq!(
        error.safe_service_code().as_deref(),
        Some("UnsupportedCountry")
    );
    assert_eq!(error.to_status_code(), Some(403));
    for text in [error.to_string(), error.safe_summary().to_string()] {
        assert!(!text.contains("fixture-secret"));
        assert!(!text.contains("fixture-identity"));
        assert!(!text.contains("https://"));
    }
}

#[test]
fn oauth_and_xbox_error_codes_share_the_privacy_allowlist() {
    assert_eq!(
        WebApiError::http_response(
            400,
            r#"{"error":"invalid_grant","error_description":"fixture-secret"}"#
        )
        .safe_service_code()
        .as_deref(),
        Some("invalid_grant")
    );
    assert_eq!(
        WebApiError::http_response(401, r#"{"XErr":2148916233}"#)
            .safe_service_code()
            .as_deref(),
        Some("XErr:2148916233")
    );
    for body in [
        r#"{"code":"fixture-secret-token"}"#,
        r#"{"XErr":2533274981234567}"#,
        "<html>fixture-secret</html>",
    ] {
        let error = WebApiError::http_response(403, body);
        assert!(error.safe_service_code().is_none());
        assert!(!error.to_string().contains("fixture-secret"));
    }
}

#[test]
fn network_classification_reads_the_cause_chain_without_publishing_it() {
    assert_eq!(
        classify_network_sources(
            &["request failed".into(), "DNS lookup address failed".into()],
            true,
            false
        ),
        NetworkErrorKind::Dns
    );
    assert_eq!(
        classify_network_sources(
            &["request failed".into(), "invalid peer certificate".into()],
            true,
            false
        ),
        NetworkErrorKind::Tls
    );
    assert_eq!(
        classify_network_sources(&["connection refused".into()], true, false),
        NetworkErrorKind::Connect
    );
    assert_eq!(
        classify_network_sources(&["incomplete message".into()], false, true),
        NetworkErrorKind::Body
    );
    assert_eq!(
        WebApiError::network_kind(NetworkErrorKind::Timeout).safe_summary()["networkErrorKind"],
        "timeout"
    );
    assert!(!WebApiError::network_kind(NetworkErrorKind::Tls).is_retriable());
}
