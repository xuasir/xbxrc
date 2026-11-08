use super::*;
use crate::error::AppError;
use crate::mods::runtime_trace::RuntimeTraceRecorder;
use std::sync::Arc;

#[test]
fn projects_token_availability_and_cloud_status_without_credentials() {
    let summary = token_summary(&json!({
        "xHomeToken": { "gsToken": "secret-token", "userId": "secret-account" },
        "_diagnostics": { "xCloudToken": {
            "xgpuweb": { "statusCode": 403, "timeout": false, "retriable": false, "message": "secret-body" },
            "xgpuwebf2p": { "statusCode": "secret-status", "url": "https://example.test/?token=secret-token" },
        }},
    }));
    assert_eq!(summary["hasHomeToken"], true);
    assert_eq!(summary["hasCloudToken"], false);
    assert_eq!(summary["cloudDiagnostics"]["xgpuweb"]["statusCode"], 403);
    assert!(summary["cloudDiagnostics"]["xgpuwebf2p"]["statusCode"].is_null());
    assert!(!summary.to_string().contains("secret"));
    assert!(!summary.to_string().contains("https://"));
}

#[test]
fn projects_http_failure_without_upstream_body() {
    let error = AuthFlowError::from(xbox_webapi::WebApiError::http(
        403,
        "secret-account-and-token",
    ));
    assert_eq!(
        flow_error_summary(&error),
        json!({ "errorKind": "http", "statusCode": 403, "retriable": false })
    );
    let error = AuthFlowError::Protocol("secret-protocol-body".to_string());
    assert_eq!(
        flow_error_summary(&error),
        json!({ "errorKind": "protocol", "statusCode": null, "retriable": false })
    );
}

#[test]
fn keeps_whitelisted_access_and_network_codes_and_discards_untrusted_diagnostics() {
    let summary = token_summary(&json!({"_diagnostics": {"xCloudToken": {
        "xgpuweb": {"statusCode": 403, "serviceCode": "UnsupportedCountry", "networkErrorKind": "tls"},
        "xgpuwebf2p": {"statusCode": 403, "serviceCode": "secret-account", "networkErrorKind": "https://secret.example/"}
    }}}));
    assert_eq!(
        summary["cloudDiagnostics"]["xgpuweb"]["serviceCode"],
        "UnsupportedCountry"
    );
    assert_eq!(
        summary["cloudDiagnostics"]["xgpuweb"]["networkErrorKind"],
        "tls"
    );
    assert!(summary["cloudDiagnostics"]["xgpuwebf2p"]["serviceCode"].is_null());
    assert!(summary["cloudDiagnostics"]["xgpuwebf2p"]["networkErrorKind"].is_null());
    assert!(!summary.to_string().contains("secret"));
    assert_eq!(
        flow_error_summary(
            &xbox_webapi::WebApiError::network_kind(xbox_webapi::NetworkErrorKind::Dns).into()
        )["networkErrorKind"],
        "dns"
    );
    let error = AppError::Data("secret-token".into());
    assert_eq!(
        app_error_summary(&error),
        json!({"errorCode": "DATA_ERROR"})
    );
}

#[test]
fn production_and_dev_keep_correlated_auth_stages_and_terminal_error() {
    for profile in ["production", "dev"] {
        let recorder = Arc::new(RuntimeTraceRecorder::new_with_mode(profile).unwrap());
        let trace = AuthOperationTrace::new(&recorder, "checkAuthentication");
        trace.stage("sessionSnapshotReadStarted", json!({}));
        let result: AppResult<()> = Err(AppError::Data("secret-token-in-error-body".to_string()));
        trace.finish(
            &result,
            &AuthState {
                provider: "secret-account".to_string(),
                is_authenticated: false,
                is_authenticating: true,
                app_level: 0,
            },
        );
        let contents = std::fs::read_to_string(recorder.path().unwrap()).unwrap();
        let rows: Vec<Value> = contents
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|row| row["domain"] == "auth")
            .collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["event"], "authOperationStarted");
        assert_eq!(rows[2]["event"], "authOperationFinished");
        for row in &rows {
            assert_eq!(row["payload"]["operationId"], trace.id());
            assert_eq!(row["dimension"], "lifecycle");
            assert_eq!(row["importance"], "key");
            assert_eq!(row["traceProfile"], profile);
        }
        assert_eq!(rows[2]["payload"]["details"]["errorCode"], "DATA_ERROR");
        assert_eq!(
            rows[2]["payload"]["details"]["state"]["isAuthenticating"],
            true
        );
        assert!(!contents.contains("secret"));
    }
}
