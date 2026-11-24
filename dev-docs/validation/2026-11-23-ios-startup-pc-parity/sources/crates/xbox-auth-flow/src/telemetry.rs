//! 认证步骤仅向宿主提供固定名称、耗时和脱敏错误事实。
use serde_json::{json, Value};
use std::future::Future;
use std::time::Instant;
use xbox_webapi::WebApiError;

pub trait AuthFlowObserver: Send + Sync {
    fn stage(&self, stage: &'static str, facts: Value);
}

pub(crate) async fn observe_request<T>(
    observer: Option<&dyn AuthFlowObserver>,
    stage: &'static str,
    request: impl Future<Output = Result<T, WebApiError>>,
) -> Result<T, WebApiError> {
    let started = Instant::now();
    if let Some(observer) = observer {
        observer.stage("authRequestStarted", json!({ "requestStage": stage }));
    }
    // 各层仅持有堆上的请求 future，保留 Swift 调用线程的小栈余量。
    let result = Box::pin(request).await;
    if let Some(observer) = observer {
        let mut facts = result
            .as_ref()
            .err()
            .map(WebApiError::safe_summary)
            .unwrap_or_else(|| json!({}));
        facts["requestStage"] = json!(stage);
        facts["elapsedMs"] = json!(started.elapsed().as_millis() as u64);
        observer.stage(
            if result.is_ok() {
                "authRequestSucceeded"
            } else {
                "authRequestFailed"
            },
            facts,
        );
    }
    result
}

pub(crate) fn flow_error_facts(error: &crate::AuthFlowError) -> Value {
    error
        .upstream_error()
        .map(WebApiError::safe_summary)
        .unwrap_or_else(|| json!({ "errorKind": "protocol", "retriable": false }))
}
