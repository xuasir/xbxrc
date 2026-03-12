use thiserror::Error;

/// `list_active_sessions` 未指定 targetType 时的默认值。
pub const DEFAULT_ACTIVE_TARGET_TYPE: &str = "cloud";

/// 会话路径解析错误：当前仅关心 session id 缺失。
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SessionPathError {
    #[error("Streaming session id is missing")]
    MissingSessionId,
}

/// 解析 `/v5/sessions/{target}/{sessionId}` 里的 session id。
pub fn parse_session_id_from_path(session_path: &str) -> Result<String, SessionPathError> {
    session_path
        .split('/')
        .filter(|segment| !segment.trim().is_empty())
        .nth(3)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or(SessionPathError::MissingSessionId)
}

/// close_session 的判定语义：仅 404 视为“远端不存在”，其余交给上层决定。
pub fn is_remote_session_not_found(status: Option<u16>) -> bool {
    status == Some(404)
}

/// 规范化 active sessions 的 targetType 过滤入参。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveTargetType {
    pub value: String,
    pub used_default: bool,
}

pub fn resolve_active_target_type(target_type: Option<String>) -> ActiveTargetType {
    match target_type {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return ActiveTargetType {
                    value: DEFAULT_ACTIVE_TARGET_TYPE.to_string(),
                    used_default: true,
                };
            }
            ActiveTargetType {
                value: trimmed.to_string(),
                used_default: false,
            }
        }
        None => ActiveTargetType {
            value: DEFAULT_ACTIVE_TARGET_TYPE.to_string(),
            used_default: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_remote_session_not_found, parse_session_id_from_path, resolve_active_target_type,
        SessionPathError, DEFAULT_ACTIVE_TARGET_TYPE,
    };

    #[test]
    fn parse_session_id_from_path_extracts_id() {
        let id = parse_session_id_from_path("/v5/sessions/cloud/session-1").unwrap();
        assert_eq!(id, "session-1");
    }

    #[test]
    fn parse_session_id_from_path_reports_missing_id() {
        let error = parse_session_id_from_path("/v5/sessions/cloud/").unwrap_err();
        assert_eq!(error, SessionPathError::MissingSessionId);
    }

    #[test]
    fn resolve_active_target_type_uses_default_when_missing_or_empty() {
        let from_none = resolve_active_target_type(None);
        assert_eq!(from_none.value, DEFAULT_ACTIVE_TARGET_TYPE);
        assert!(from_none.used_default);

        let from_empty = resolve_active_target_type(Some("".to_string()));
        assert_eq!(from_empty.value, DEFAULT_ACTIVE_TARGET_TYPE);
        assert!(from_empty.used_default);
    }

    #[test]
    fn close_semantics_only_treats_404_as_not_found() {
        assert!(is_remote_session_not_found(Some(404)));
        assert!(!is_remote_session_not_found(Some(500)));
        assert!(!is_remote_session_not_found(None));
    }
}
