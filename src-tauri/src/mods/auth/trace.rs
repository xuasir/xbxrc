use crate::error::{AppError, AppResult};
use crate::mods::auth::types::AuthState;
use crate::mods::runtime_trace::{
    RuntimeTraceCategory, RuntimeTraceRecorderRef, TraceDimension, TraceImportance, TraceMetadata,
};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use xbox_auth_flow::AuthFlowError;

static AUTH_OPERATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub(super) fn record_auth_event(recorder: &RuntimeTraceRecorderRef, event: &str, payload: Value) {
    recorder.record_with_metadata(
        RuntimeTraceCategory::Event,
        "auth",
        event,
        None,
        payload,
        TraceMetadata::new(TraceDimension::Lifecycle, TraceImportance::Key),
    );
}

pub(super) fn state_summary(state: &AuthState) -> Value {
    json!({
        "isAuthenticated": state.is_authenticated,
        "isAuthenticating": state.is_authenticating,
        "appLevel": state.app_level,
    })
}

pub(super) fn token_summary(tokens: &Value) -> Value {
    let mut diagnostics = serde_json::Map::new();
    for offering in ["xgpuweb", "xgpuwebf2p"] {
        let error = &tokens["_diagnostics"]["xCloudToken"][offering];
        if error.is_object() {
            let mut summary = json!({
                "statusCode": error["statusCode"].as_u64().filter(|status| (100..=599).contains(status)),
                "timeout": error["timeout"].as_bool(),
                "retriable": error["retriable"].as_bool(),
            });
            if let Some(code) = error["serviceCode"]
                .as_str()
                .and_then(xbox_webapi::sanitize_service_code)
            {
                summary["serviceCode"] = json!(code);
            }
            if let Some(kind) = error["networkErrorKind"].as_str().filter(|kind| {
                matches!(
                    *kind,
                    "dns" | "connect" | "tls" | "timeout" | "body" | "request"
                )
            }) {
                summary["networkErrorKind"] = json!(kind);
            }
            diagnostics.insert(offering.to_string(), summary);
        }
    }
    json!({
        "hasHomeToken": tokens.get("xHomeToken").is_some_and(|value| !value.is_null()),
        "hasCloudToken": tokens.get("xCloudToken").is_some_and(|value| !value.is_null()),
        "cloudDiagnostics": diagnostics,
    })
}

pub(super) fn flow_error_summary(error: &AuthFlowError) -> Value {
    // 错误正文可能携带请求URL或服务端身份，只记录固定分类与状态码。
    match error {
        AuthFlowError::Upstream(error) => error.safe_summary(),
        _ => {
            json!({ "errorKind": "protocol", "statusCode": null, "retriable": error.is_retriable() })
        }
    }
}

pub(super) fn app_error_summary(error: &AppError) -> Value {
    match error {
        AppError::WebApi(error) => error.safe_summary(),
        _ => json!({ "errorCode": error.code() }),
    }
}

pub(super) struct AuthOperationTrace {
    recorder: RuntimeTraceRecorderRef,
    id: String,
    operation: &'static str,
    started_at: Instant,
}

impl AuthOperationTrace {
    pub fn new(recorder: &RuntimeTraceRecorderRef, operation: &'static str) -> Self {
        let trace = Self {
            recorder: recorder.clone(),
            id: format!(
                "auth-{}",
                AUTH_OPERATION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ),
            operation,
            started_at: Instant::now(),
        };
        trace.record("authOperationStarted", json!({}));
        trace
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    fn record(&self, event: &str, details: Value) {
        record_auth_event(
            &self.recorder,
            event,
            json!({
                "operationId": self.id,
                "operation": self.operation,
                "elapsedMs": self.started_at.elapsed().as_millis(),
                "details": details,
            }),
        );
    }

    pub fn stage(&self, stage: &str, details: Value) {
        self.record(
            "authOperationStage",
            json!({ "stage": stage, "facts": details }),
        );
    }

    pub fn finish<T>(&self, result: &AppResult<T>, state: &AuthState) {
        self.record(
            "authOperationFinished",
            json!({
                "outcome": if result.is_ok() { "success" } else { "failed" },
                "errorCode": result.as_ref().err().map(|error| error.code()),
                "state": state_summary(state),
            }),
        );
    }
}

#[cfg(test)]
#[path = "trace.test.rs"]
mod tests;
