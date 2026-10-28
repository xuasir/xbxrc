//! 会话、启动、信令与错误投影；保持领域事实与页面展示合同的映射。
use super::*;

pub(super) fn map_domain_progress_snapshot(
    progress: SessionProgressSnapshot,
) -> StreamingSessionProgressSnapshot {
    let phase = map_domain_session_phase(progress.phase);
    let error = build_progress_error(
        &phase,
        progress.error_code.as_deref(),
        progress.error_message.as_deref(),
        progress.error_hint.as_ref(),
    );
    StreamingSessionProgressSnapshot {
        session_id: progress.session_id,
        phase,
        runtime_launch_state: map_domain_runtime_launch_state(progress.runtime_launch_state),
        status_text_key: progress.status_text_key,
        queue_seconds: progress.queue_seconds,
        queue: progress.queue.map(Into::into),
        error_code: progress.error_code,
        error_message: progress.error_message,
        error,
    }
}

pub(super) fn build_fallback_progress_snapshot(
    mut progress: StreamingSessionProgressSnapshot,
) -> StreamingSessionProgressSnapshot {
    progress.error = build_progress_error(
        &progress.phase,
        progress.error_code.as_deref(),
        progress.error_message.as_deref(),
        None,
    );
    progress
}

pub(super) fn map_domain_session_phase(phase: DomainSessionPhase) -> StreamingSessionPhase {
    match phase {
        DomainSessionPhase::Creating => StreamingSessionPhase::Creating,
        DomainSessionPhase::WaitingSessionReady => StreamingSessionPhase::WaitingSessionReady,
        DomainSessionPhase::RuntimeStarting => StreamingSessionPhase::RuntimeStarting,
        DomainSessionPhase::SessionReady => StreamingSessionPhase::SessionReady,
        DomainSessionPhase::Recovering => StreamingSessionPhase::Recovering,
        DomainSessionPhase::Closing => StreamingSessionPhase::Closing,
        DomainSessionPhase::Closed => StreamingSessionPhase::Closed,
        DomainSessionPhase::Failed => StreamingSessionPhase::Failed,
    }
}

pub(super) fn map_domain_runtime_launch_state(
    state: DomainRuntimeLaunchState,
) -> StreamingRuntimeLaunchState {
    match state {
        DomainRuntimeLaunchState::Blocked => StreamingRuntimeLaunchState::Blocked,
        DomainRuntimeLaunchState::Ready => StreamingRuntimeLaunchState::Ready,
        DomainRuntimeLaunchState::Closed => StreamingRuntimeLaunchState::Closed,
        DomainRuntimeLaunchState::Failed => StreamingRuntimeLaunchState::Failed,
    }
}

pub(super) fn build_progress_error(
    phase: &StreamingSessionPhase,
    error_code: Option<&str>,
    error_message: Option<&str>,
    error_hint: Option<&DomainStartupErrorHint>,
) -> Option<StreamingSessionError> {
    let raw_message = error_message
        .filter(|value| !value.trim().is_empty())
        .or_else(|| error_code.filter(|value| !value.trim().is_empty()))
        .map(str::to_string)?;
    let bounded_retry = build_progress_bounded_retry(raw_message.as_str());

    if let Some(hint) = error_hint {
        let error_kind = map_domain_startup_error_kind(hint);
        return Some(StreamingSessionError {
            error_kind: error_kind.clone(),
            user_message_key: startup_error_message_key(&error_kind).to_string(),
            diagnostic_summary: hint.diagnostic_summary.clone(),
            raw_message,
            retryable: hint.retryable,
            bounded_retry,
        });
    }

    let error_kind = classify_progress_error_kind_fallback(phase, &raw_message);
    Some(StreamingSessionError {
        error_kind: error_kind.clone(),
        user_message_key: startup_error_message_key(&error_kind).to_string(),
        diagnostic_summary: build_progress_diagnostic_summary_fallback(
            phase,
            error_code,
            error_message,
            &error_kind,
        ),
        raw_message,
        retryable: is_progress_error_retryable_fallback(&error_kind),
        bounded_retry,
    })
}

pub(super) fn to_domain_ice_candidate(candidate: &StreamingIceCandidate) -> DomainIceCandidate {
    DomainIceCandidate {
        candidate: candidate.candidate.clone(),
        sdp_m_line_index: candidate.sdp_m_line_index,
        sdp_mid: candidate.sdp_mid.clone(),
        username_fragment: candidate.username_fragment.clone(),
        message_type: candidate.message_type.clone(),
    }
}

pub(super) fn from_domain_ice_candidate(candidate: DomainIceCandidate) -> StreamingIceCandidate {
    StreamingIceCandidate {
        candidate: candidate.candidate,
        sdp_m_line_index: candidate.sdp_m_line_index,
        sdp_mid: candidate.sdp_mid,
        username_fragment: candidate.username_fragment,
        message_type: candidate.message_type,
    }
}

pub(super) fn from_domain_answer_payload(payload: DomainAnswerPayload) -> StreamingAnswerPayload {
    StreamingAnswerPayload {
        sdp: payload.sdp,
        message_type: payload.message_type,
    }
}

pub(super) fn normalize_optional(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(super) fn map_startup_phase(phase: DomainStartupPhase) -> StreamingStartupPhase {
    match phase {
        DomainStartupPhase::ResolvingContext => StreamingStartupPhase::ResolvingContext,
        DomainStartupPhase::CreatingSession => StreamingStartupPhase::CreatingSession,
        DomainStartupPhase::WaitingSessionReady => StreamingStartupPhase::WaitingSessionReady,
        DomainStartupPhase::StartingRuntime => StreamingStartupPhase::StartingRuntime,
        DomainStartupPhase::Ready => StreamingStartupPhase::Ready,
        DomainStartupPhase::Failed => StreamingStartupPhase::Failed,
    }
}

pub(super) fn map_startup_phase_status(
    status: DomainStartupPhaseStatus,
) -> StreamingStartupPhaseStatus {
    match status {
        DomainStartupPhaseStatus::Entered => StreamingStartupPhaseStatus::Entered,
        DomainStartupPhaseStatus::Succeeded => StreamingStartupPhaseStatus::Succeeded,
        DomainStartupPhaseStatus::Failed => StreamingStartupPhaseStatus::Failed,
    }
}

pub(super) fn map_startup_bounded_retry_status(
    status: DomainStartupBoundedRetryStatus,
) -> StreamingStartupBoundedRetryStatus {
    match status {
        DomainStartupBoundedRetryStatus::Retrying => StreamingStartupBoundedRetryStatus::Retrying,
        DomainStartupBoundedRetryStatus::Exhausted => StreamingStartupBoundedRetryStatus::Exhausted,
    }
}

pub(super) fn map_startup_bounded_retry_reason(
    reason: DomainStartupBoundedRetryReason,
) -> StreamingStartupBoundedRetryReason {
    match reason {
        DomainStartupBoundedRetryReason::WaitingForServerRegistration => {
            StreamingStartupBoundedRetryReason::WaitingForServerRegistration
        }
    }
}

pub(super) fn map_startup_bounded_retry(
    bounded_retry: &DomainStartupBoundedRetrySnapshot,
) -> StreamingStartupBoundedRetry {
    StreamingStartupBoundedRetry {
        reason: map_startup_bounded_retry_reason(bounded_retry.reason),
        status: map_startup_bounded_retry_status(bounded_retry.status),
        retry_count: bounded_retry.retry_count,
        retry_limit: bounded_retry.retry_limit,
    }
}

pub(super) fn startup_phase_summary(
    phase: &StreamingStartupPhase,
    details: Option<&str>,
) -> String {
    let details_suffix = details
        .filter(|value| !value.is_empty())
        .map(|value| format!(" ({value})"))
        .unwrap_or_default();
    match phase {
        StreamingStartupPhase::ResolvingContext => format!("resolvingContext{details_suffix}"),
        StreamingStartupPhase::CreatingSession => format!("creatingSession{details_suffix}"),
        StreamingStartupPhase::WaitingSessionReady => {
            format!("waitingSessionReady{details_suffix}")
        }
        StreamingStartupPhase::StartingRuntime => format!("startingRuntime{details_suffix}"),
        StreamingStartupPhase::Ready => format!("ready{details_suffix}"),
        StreamingStartupPhase::Failed => format!("failed{details_suffix}"),
    }
}

pub(super) fn startup_bounded_retry_summary(
    phase: &StreamingStartupPhase,
    bounded_retry: &StreamingStartupBoundedRetry,
) -> String {
    let detail = match bounded_retry.status {
        StreamingStartupBoundedRetryStatus::Retrying => "boundedRetry",
        StreamingStartupBoundedRetryStatus::Exhausted => "boundedRetryExhausted",
    };
    startup_phase_summary(phase, Some(detail))
}

pub(super) fn build_startup_bounded_retry_details(
    bounded_retry: &StreamingStartupBoundedRetry,
) -> String {
    format!(
        "reason={:?};status={:?};retryCount={};retryLimit={}",
        bounded_retry.reason,
        bounded_retry.status,
        bounded_retry.retry_count,
        bounded_retry.retry_limit,
    )
}

pub(super) fn map_domain_startup_error_kind(
    hint: &DomainStartupErrorHint,
) -> StreamingStartupErrorKind {
    match hint.kind {
        DomainStartupErrorKind::SessionCreate => StreamingStartupErrorKind::SessionCreate,
        DomainStartupErrorKind::SessionReady => StreamingStartupErrorKind::SessionReady,
        DomainStartupErrorKind::Runtime => StreamingStartupErrorKind::Runtime,
        DomainStartupErrorKind::Network => StreamingStartupErrorKind::Network,
        DomainStartupErrorKind::Auth => StreamingStartupErrorKind::Auth,
        DomainStartupErrorKind::Target => StreamingStartupErrorKind::Target,
        DomainStartupErrorKind::HostRemotePlayUnavailable => {
            StreamingStartupErrorKind::HostRemotePlayUnavailable
        }
        DomainStartupErrorKind::HostRegistrationRetryExhausted => {
            StreamingStartupErrorKind::HostRegistrationRetryExhausted
        }
        DomainStartupErrorKind::Unknown => StreamingStartupErrorKind::Unknown,
    }
}

pub(super) fn classify_progress_error_kind_fallback(
    phase: &StreamingSessionPhase,
    raw_message: &str,
) -> StreamingStartupErrorKind {
    let normalized = raw_message.to_ascii_lowercase();
    if is_home_server_registration_retry_exhausted_message(&normalized)
        || is_server_registration_retry_signal_message(&normalized)
    {
        return StreamingStartupErrorKind::HostRegistrationRetryExhausted;
    }
    if normalized.contains("remoteconsolenotready") {
        return StreamingStartupErrorKind::HostRemotePlayUnavailable;
    }
    if normalized.contains("streamingstarttimeout") {
        return StreamingStartupErrorKind::SessionReady;
    }
    // monitor 的「卡在 Provisioning / ReadyToConnect」超时（错误码 `SessionStateTimeout`，
    // 消息形如 "Streaming session stayed in ReadyToConnect for 90000ms."）。
    // 这不是播放器自身失败：会话一直没能进入 Provisioned，播放器只是在空转等待。
    // 必须归到 SessionReady，否则会落到 `phase=Failed → Runtime` 分支，
    // 给用户显示「串流播放器启动失败」，把排查方向带偏。
    if normalized.contains("sessionstatetimeout") || normalized.contains("session stayed in") {
        return StreamingStartupErrorKind::SessionReady;
    }
    if normalized.contains("targetmissing") {
        return StreamingStartupErrorKind::Target;
    }
    if normalized.contains("unauthorized")
        || normalized.contains("forbidden")
        || normalized.contains("authentication")
        || normalized.contains("auth")
    {
        return StreamingStartupErrorKind::Auth;
    }
    if normalized.contains("network")
        || normalized.contains("reconnect")
        || normalized.contains("recover")
    {
        return StreamingStartupErrorKind::Network;
    }

    match phase {
        StreamingSessionPhase::Failed
        | StreamingSessionPhase::Closed
        | StreamingSessionPhase::Recovering => StreamingStartupErrorKind::Runtime,
        _ => StreamingStartupErrorKind::Unknown,
    }
}

pub(super) fn classify_startup_error_kind_fallback(
    phase: &StreamingStartupPhase,
    error: &SessionFlowError,
) -> StreamingStartupErrorKind {
    let normalized = error.message.to_ascii_lowercase();
    if is_home_remote_play_not_ready_message(&normalized) {
        return StreamingStartupErrorKind::HostRemotePlayUnavailable;
    }
    if is_home_server_registration_retry_exhausted_message(&normalized) {
        return StreamingStartupErrorKind::HostRegistrationRetryExhausted;
    }
    if normalized.contains("target missing") {
        return StreamingStartupErrorKind::Target;
    }
    if normalized.contains("auth") || normalized.contains("token missing") {
        return StreamingStartupErrorKind::Auth;
    }
    if normalized.contains("network error") {
        return StreamingStartupErrorKind::Network;
    }
    match phase {
        StreamingStartupPhase::ResolvingContext => StreamingStartupErrorKind::Unknown,
        StreamingStartupPhase::CreatingSession => StreamingStartupErrorKind::SessionCreate,
        StreamingStartupPhase::WaitingSessionReady => StreamingStartupErrorKind::SessionReady,
        StreamingStartupPhase::StartingRuntime => StreamingStartupErrorKind::Runtime,
        StreamingStartupPhase::Ready => StreamingStartupErrorKind::Runtime,
        StreamingStartupPhase::Failed => StreamingStartupErrorKind::Unknown,
    }
}

pub(super) fn startup_error_message_key(kind: &StreamingStartupErrorKind) -> &'static str {
    match kind {
        StreamingStartupErrorKind::SessionCreate => "streamPage.errors.sessionCreateFailed",
        StreamingStartupErrorKind::SessionReady => "streamPage.errors.sessionReadyFailed",
        StreamingStartupErrorKind::Runtime => "streamPage.errors.runtimeStartFailed",
        StreamingStartupErrorKind::Network => "streamPage.errors.networkFailed",
        StreamingStartupErrorKind::Auth => "streamPage.errors.authFailed",
        StreamingStartupErrorKind::Target => "streamPage.errors.targetMissing",
        StreamingStartupErrorKind::HostRemotePlayUnavailable => {
            "streamPage.errors.hostRemotePlayUnavailable"
        }
        StreamingStartupErrorKind::HostRegistrationRetryExhausted => {
            "streamPage.errors.hostRegistrationRetryExhausted"
        }
        StreamingStartupErrorKind::Unknown => "streamPage.errors.unknown",
    }
}

pub(super) fn build_progress_diagnostic_summary_fallback(
    phase: &StreamingSessionPhase,
    error_code: Option<&str>,
    error_message: Option<&str>,
    kind: &StreamingStartupErrorKind,
) -> String {
    let error_code = error_code.unwrap_or("none");
    let error_message = error_message.unwrap_or("none");
    let hint = match kind {
        StreamingStartupErrorKind::SessionCreate => "sessionCreateFailed",
        StreamingStartupErrorKind::SessionReady => "streamingStartTimeout",
        StreamingStartupErrorKind::Runtime => "runtimeFailed",
        StreamingStartupErrorKind::Network => "networkFailed",
        StreamingStartupErrorKind::Auth => "authFailed",
        StreamingStartupErrorKind::Target => "targetMissing",
        StreamingStartupErrorKind::HostRemotePlayUnavailable => "hostRemotePlayUnavailable",
        StreamingStartupErrorKind::HostRegistrationRetryExhausted => {
            "hostRegistrationRetryExhausted"
        }
        StreamingStartupErrorKind::Unknown => "unknown",
    };
    format!("phase={phase:?}; errorCode={error_code}; errorMessage={error_message}; hint={hint}")
}

pub(super) fn is_progress_error_retryable_fallback(kind: &StreamingStartupErrorKind) -> bool {
    matches!(
        kind,
        StreamingStartupErrorKind::SessionCreate
            | StreamingStartupErrorKind::SessionReady
            | StreamingStartupErrorKind::Runtime
            | StreamingStartupErrorKind::Network
    )
}

pub(super) fn build_startup_diagnostic_summary_fallback(
    phase: &StreamingStartupPhase,
    error: &SessionFlowError,
) -> String {
    if let Some(summary) = build_home_remote_play_not_ready_summary(&error.message) {
        return format!("phase={phase:?}; {summary}");
    }
    if let Some(summary) = build_host_registration_retry_exhausted_summary(error) {
        return format!("phase={phase:?}; {summary}");
    }
    if let Some(summary) = build_remote_console_wake_circuit_summary(&error.message) {
        return format!("phase={phase:?}; {summary}");
    }
    let detail = error
        .body
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or(&error.message);
    format!("phase={phase:?}; detail={detail}")
}

pub(super) fn is_startup_error_retryable_fallback(
    kind: &StreamingStartupErrorKind,
    error: &SessionFlowError,
) -> bool {
    if is_home_remote_play_not_ready_message(&error.message.to_ascii_lowercase()) {
        return false;
    }
    if is_remote_console_wake_circuit_open_message(&error.message) {
        return false;
    }
    matches!(
        kind,
        StreamingStartupErrorKind::SessionCreate
            | StreamingStartupErrorKind::SessionReady
            | StreamingStartupErrorKind::Network
    ) || error.status.is_some_and(|status| status >= 500)
}

pub(super) fn is_home_remote_play_not_ready_message(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("homeremoteplaynotready")
}

pub(super) fn is_home_server_registration_retry_exhausted_message(message: &str) -> bool {
    message
        .to_ascii_lowercase()
        .contains("homesessionboundedretryexhausted")
}

pub(super) fn is_server_registration_retry_signal_message(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("serverneverregistered")
        || normalized.contains("waitingforservertoregister")
}

pub(super) fn is_remote_console_wake_circuit_open_message(message: &str) -> bool {
    message
        .to_ascii_lowercase()
        .contains("remoteconsolewakecircuitopen")
}

pub(super) fn build_home_remote_play_not_ready_summary(message: &str) -> Option<String> {
    if !is_home_remote_play_not_ready_message(message) {
        return None;
    }

    let target_id = extract_preflight_field(message, "targetId").unwrap_or("unknown");
    let power_state = extract_preflight_field(message, "powerState").unwrap_or("unknown");
    let remote_management_enabled =
        extract_preflight_field(message, "remoteManagementEnabled").unwrap_or("unknown");
    let console_streaming_enabled =
        extract_preflight_field(message, "consoleStreamingEnabled").unwrap_or("unknown");
    let console_addrs_count =
        extract_preflight_field(message, "consoleAddrsCount").unwrap_or("unknown");
    let attempts = extract_preflight_field(message, "attempts").unwrap_or("unknown");
    let elapsed_ms = extract_preflight_field(message, "elapsedMs").unwrap_or("unknown");

    Some(format!(
        "targetId={target_id}; powerState={power_state}; remoteManagementEnabled={remote_management_enabled}; consoleStreamingEnabled={console_streaming_enabled}; consoleAddrsCount={console_addrs_count}; attempts={attempts}; elapsedMs={elapsed_ms}; hint=hostRemotePlayUnavailable"
    ))
}

pub(super) fn build_remote_console_wake_circuit_summary(message: &str) -> Option<String> {
    if !is_remote_console_wake_circuit_open_message(message) {
        return None;
    }

    let target_id = extract_circuit_open_field(message, "targetId").unwrap_or("unknown");
    let power_state = extract_circuit_open_field(message, "powerState").unwrap_or("unknown");
    let wake_failure_count =
        extract_circuit_open_field(message, "wakeFailureCount").unwrap_or("unknown");
    Some(format!(
        "targetId={target_id}; powerState={power_state}; wakeFailureCount={wake_failure_count}; hint=hostRemotePlayUnavailable"
    ))
}

pub(super) fn build_host_registration_retry_exhausted_summary(
    error: &SessionFlowError,
) -> Option<String> {
    if !is_home_server_registration_retry_exhausted_message(&error.message) {
        return None;
    }

    let target_id = extract_bounded_retry_field(&error.message, "targetId").unwrap_or("unknown");
    let reason = extract_bounded_retry_field(&error.message, "reason").unwrap_or("unknown");
    let retry_count =
        extract_bounded_retry_field(&error.message, "retryCount").unwrap_or("unknown");
    let retry_limit =
        extract_bounded_retry_field(&error.message, "retryLimit").unwrap_or("unknown");
    let last_error = error.body.as_deref().unwrap_or("unknown");

    Some(format!(
        "targetId={target_id}; reason={reason}; retryCount={retry_count}; retryLimit={retry_limit}; lastError={last_error}; hint=hostRegistrationRetryExhausted"
    ))
}

pub(super) fn build_progress_bounded_retry(message: &str) -> Option<StreamingStartupBoundedRetry> {
    if !is_home_server_registration_retry_exhausted_message(message) {
        return None;
    }

    let retry_count = extract_bounded_retry_field(message, "retryCount")
        .and_then(|value| value.parse::<u8>().ok())?;
    let retry_limit = extract_bounded_retry_field(message, "retryLimit")
        .and_then(|value| value.parse::<u8>().ok())?;
    Some(StreamingStartupBoundedRetry {
        reason: StreamingStartupBoundedRetryReason::WaitingForServerRegistration,
        status: StreamingStartupBoundedRetryStatus::Exhausted,
        retry_count,
        retry_limit,
    })
}

pub(super) fn extract_preflight_field<'a>(message: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    message
        .split(';')
        .find_map(|segment| segment.trim().strip_prefix(&prefix))
}

pub(super) fn extract_bounded_retry_field<'a>(message: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    message
        .split(';')
        .find_map(|segment| segment.trim().strip_prefix(&prefix))
}

pub(super) fn extract_circuit_open_field<'a>(message: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    message
        .split(';')
        .find_map(|segment| segment.trim().strip_prefix(&prefix))
}
