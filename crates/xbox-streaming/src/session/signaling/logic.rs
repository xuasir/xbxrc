use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::session::signaling::ice::IceCandidate;

/// offer/ice 轮询建议间隔（毫秒）。
pub const SESSION_SIGNALING_POLL_INTERVAL_MS: u64 = 1000;

/// 一次轮询后的控制决策。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PollDecision {
    Continue,
    SessionMissing,
    Completed,
}

/// SDP 轮询决策：拿到 answer 即完成，否则依赖会话是否仍存在。
pub fn decide_offer_poll(answer_available: bool, session_exists: bool) -> PollDecision {
    if answer_available {
        return PollDecision::Completed;
    }
    if !session_exists {
        return PollDecision::SessionMissing;
    }
    PollDecision::Continue
}

/// ICE 轮询决策：拿到可用候选即完成，否则依赖会话是否仍存在。
pub fn decide_ice_poll(candidates: Option<&[IceCandidate]>, session_exists: bool) -> PollDecision {
    if let Some(candidates) = candidates {
        if has_usable_ice_candidates(candidates) {
            return PollDecision::Completed;
        }
    }
    if !session_exists {
        return PollDecision::SessionMissing;
    }
    PollDecision::Continue
}

/// ICE 候选可用性判定：过滤空串与 end-of-candidates 标记。
pub fn has_usable_ice_candidates(candidates: &[IceCandidate]) -> bool {
    candidates.iter().any(|candidate| {
        let normalized = candidate.candidate.trim();
        !normalized.is_empty() && normalized != "a=end-of-candidates"
    })
}

const ERROR_CODE_UNEXPECTED_STATE: &str = "SessionUnexpectedState";
const ERROR_CODE_SESSION_IN_UNEXPECTED_STATE: &str = "SessionInUnexpectedState";
const ERROR_MESSAGE_SDP_EXCHANGE_SENT: &str = "ServerSdpExchangeCommandSent";
const ERROR_MESSAGE_UNEXPECTED_STATE: &str = "UnexpectedState";
const ERROR_MESSAGE_KEEPALIVE_FAILED: &str = "KeepAlive";

/// keepalive 错误忽略策略：404 直接忽略；400 仅忽略已知状态冲突。
pub fn should_ignore_keepalive_error(status: Option<u16>, body: Option<&str>) -> bool {
    if status == Some(404) {
        return true;
    }
    if status != Some(400) {
        return false;
    }

    let Some(body) = body else {
        return false;
    };

    let parsed = match serde_json::from_str::<Value>(body) {
        Ok(value) => value,
        Err(_) => Value::Null,
    };
    let code = parsed.get("code").and_then(Value::as_str).unwrap_or("");
    let message = parsed.get("message").and_then(Value::as_str).unwrap_or("");

    match code {
        ERROR_CODE_UNEXPECTED_STATE => {
            message.contains(ERROR_MESSAGE_SDP_EXCHANGE_SENT)
                || message.contains(ERROR_MESSAGE_UNEXPECTED_STATE)
        }
        ERROR_CODE_SESSION_IN_UNEXPECTED_STATE => message.contains(ERROR_MESSAGE_KEEPALIVE_FAILED),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decide_ice_poll, decide_offer_poll, has_usable_ice_candidates,
        should_ignore_keepalive_error, PollDecision,
    };
    use crate::session::signaling::ice::IceCandidate;

    #[test]
    fn offer_poll_returns_completed_when_answer_available() {
        assert_eq!(decide_offer_poll(true, true), PollDecision::Completed);
    }

    #[test]
    fn offer_poll_returns_missing_when_session_gone() {
        assert_eq!(
            decide_offer_poll(false, false),
            PollDecision::SessionMissing
        );
    }

    #[test]
    fn ice_poll_returns_completed_when_usable_candidates_exist() {
        let candidates = vec![IceCandidate {
            candidate: "a=candidate:foo 1 UDP 1234 10.0.0.1 9000 typ host".to_string(),
            ..Default::default()
        }];
        assert_eq!(
            decide_ice_poll(Some(&candidates), true),
            PollDecision::Completed
        );
    }

    #[test]
    fn has_usable_candidates_ignores_end_marker() {
        let candidates = vec![IceCandidate {
            candidate: "a=end-of-candidates".to_string(),
            ..Default::default()
        }];
        assert!(!has_usable_ice_candidates(&candidates));
    }

    #[test]
    fn keepalive_error_policy_matches_legacy_behavior() {
        assert!(should_ignore_keepalive_error(Some(404), Some("not found")));
        assert!(should_ignore_keepalive_error(
            Some(400),
            Some("{\"code\":\"SessionUnexpectedState\",\"message\":\"ServerSdpExchangeCommandSent\"}")
        ));
        assert!(should_ignore_keepalive_error(
            Some(400),
            Some("{\"code\":\"SessionInUnexpectedState\",\"statusCode\":400,\"message\":\"KeepAlive : Failed\"}")
        ));
        assert!(!should_ignore_keepalive_error(Some(400), Some("{}")));
        assert!(!should_ignore_keepalive_error(Some(500), Some("boom")));
    }
}
