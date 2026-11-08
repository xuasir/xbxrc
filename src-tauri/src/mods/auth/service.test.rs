use super::{
    auth_flow_app_error, core_refresh_required, requires_interactive_login, AuthRuntimeState,
};
use crate::error::AppError;
use xbox_auth_flow::AuthFlowError;
use xbox_webapi::{NetworkErrorKind, WebApiError};

#[test]
fn interrupted_unauthorized_recovery_keeps_core_refresh_required_until_commit() {
    let runtime = AuthRuntimeState::new("xal".into(), true, 1);
    let cached = serde_json::json!({"data": {"Token": "rejected"}});
    assert!(!core_refresh_required(None, Some(&cached), &runtime));
    let previous = runtime.current_state();
    runtime.reject_web_token("rejected").unwrap();
    runtime
        .finish_check_transient_failure(&previous, false, 0)
        .unwrap();
    assert!(core_refresh_required(None, Some(&cached), &runtime));
    assert!(core_refresh_required(Some("rejected"), None, &runtime));
    runtime
        .commit_authentication(runtime.generation().unwrap(), 1, || Ok(()))
        .unwrap();
    assert!(!core_refresh_required(None, Some(&cached), &runtime));
}

#[test]
fn clears_credentials_only_for_explicit_authentication_rejection() {
    for error in [
        WebApiError::http(401, "expired"),
        WebApiError::http_with_code(400, "invalid_grant", "revoked"),
    ] {
        assert!(requires_interactive_login(&auth_flow_app_error(
            error.into()
        )));
    }
    for error in [
        WebApiError::network_kind(NetworkErrorKind::Tls),
        WebApiError::network_kind(NetworkErrorKind::Timeout),
        WebApiError::http(403, "permission"),
        WebApiError::http(429, "rate limited"),
        WebApiError::http(503, "unavailable"),
        WebApiError::parse("invalid response"),
    ] {
        assert!(!requires_interactive_login(&auth_flow_app_error(
            error.into()
        )));
    }
}

#[test]
fn preserves_typed_service_errors_and_hides_protocol_details() {
    let error = auth_flow_app_error(
        WebApiError::http_with_code(403, "UnsupportedCountry", "rejected").into(),
    );
    assert!(
        matches!(error, AppError::WebApi(ref error) if error.to_status_code() == Some(403)
        && error.safe_service_code().as_deref() == Some("UnsupportedCountry"))
    );
    let error = auth_flow_app_error(AuthFlowError::Protocol("secret-token".into()));
    assert!(!error.to_string().contains("secret"));
    assert!(!requires_interactive_login(&error));
    for error in [
        WebApiError::http_with_code(403, "secret-code", "secret-body"),
        WebApiError::parse("secret-token in invalid response"),
        WebApiError::auth("secret-account"),
    ] {
        let mapped = auth_flow_app_error(error.into());
        assert!(!mapped.to_string().contains("secret"));
    }
}
