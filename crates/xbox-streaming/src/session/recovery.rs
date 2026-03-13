use serde::{Deserialize, Serialize};

/// runtime 恢复原因，session 侧作为统一判定结果对外暴露。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeReconnectReason {
    NetworkLost,
    IceFailed,
    MediaStalled,
}

impl RuntimeReconnectReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NetworkLost => "network-lost",
            Self::IceFailed => "ice-failed",
            Self::MediaStalled => "media-stalled",
        }
    }
}

/// runtime 上报给 session 的运行事实，用于统一恢复判定入口。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeFact<'a> {
    TransportConnectionState(&'a str),
    MediaHealth {
        connection_state: &'a str,
        connected_elapsed_ms: u64,
        inactivity_elapsed_ms: u64,
    },
    MediaStalled,
}

/// connected 后首帧等待窗口，小于该值时不判定卡流。
const MEDIA_STALL_FIRST_FRAME_GRACE_MS: u64 = 10_000;
/// 连续无媒体活动超过该阈值时，触发 media-stalled 恢复。
const MEDIA_STALL_THRESHOLD_MS: u64 = 15_000;

/// 统一恢复判定入口：session 根据 runtime 事实决定是否下发 reconnect。
pub fn decide_runtime_recovery(
    fact: RuntimeFact<'_>,
    is_closing: bool,
) -> Option<RuntimeReconnectReason> {
    if is_closing {
        return None;
    }

    match fact {
        RuntimeFact::TransportConnectionState(connection_state) => {
            map_transport_recovery_reason(connection_state)
        }
        RuntimeFact::MediaHealth {
            connection_state,
            connected_elapsed_ms,
            inactivity_elapsed_ms,
        } => map_media_health_recovery_reason(
            connection_state,
            connected_elapsed_ms,
            inactivity_elapsed_ms,
        ),
        RuntimeFact::MediaStalled => Some(RuntimeReconnectReason::MediaStalled),
    }
}

/// 兼容旧入口：transport 连接态判定继续复用统一恢复函数。
pub fn decide_transport_recovery(
    connection_state: &str,
    is_closing: bool,
) -> Option<RuntimeReconnectReason> {
    decide_runtime_recovery(
        RuntimeFact::TransportConnectionState(connection_state),
        is_closing,
    )
}

fn map_transport_recovery_reason(connection_state: &str) -> Option<RuntimeReconnectReason> {
    match connection_state {
        "failed" => Some(RuntimeReconnectReason::IceFailed),
        "closed" => Some(RuntimeReconnectReason::NetworkLost),
        _ => None,
    }
}

fn map_media_health_recovery_reason(
    connection_state: &str,
    connected_elapsed_ms: u64,
    inactivity_elapsed_ms: u64,
) -> Option<RuntimeReconnectReason> {
    if connection_state != "connected" {
        return None;
    }

    if connected_elapsed_ms < MEDIA_STALL_FIRST_FRAME_GRACE_MS {
        return None;
    }

    if inactivity_elapsed_ms < MEDIA_STALL_THRESHOLD_MS {
        return None;
    }

    Some(RuntimeReconnectReason::MediaStalled)
}

#[cfg(test)]
mod tests {
    use super::{
        decide_runtime_recovery, decide_transport_recovery, RuntimeFact, RuntimeReconnectReason,
    };

    #[test]
    fn decide_transport_recovery_respects_closing_flag() {
        let reason = decide_transport_recovery("failed", true);
        assert_eq!(reason, None);
    }

    #[test]
    fn decide_transport_recovery_maps_failed_and_closed() {
        assert_eq!(
            decide_transport_recovery("failed", false),
            Some(RuntimeReconnectReason::IceFailed)
        );
        assert_eq!(
            decide_transport_recovery("closed", false),
            Some(RuntimeReconnectReason::NetworkLost)
        );
    }

    #[test]
    fn decide_runtime_recovery_maps_media_stalled_event() {
        let reason = decide_runtime_recovery(RuntimeFact::MediaStalled, false);
        assert_eq!(reason, Some(RuntimeReconnectReason::MediaStalled));
    }

    #[test]
    fn decide_runtime_recovery_ignores_unknown_transport_state() {
        let reason =
            decide_runtime_recovery(RuntimeFact::TransportConnectionState("connecting"), false);
        assert_eq!(reason, None);
    }

    #[test]
    fn decide_runtime_recovery_uses_media_health_threshold() {
        let reason = decide_runtime_recovery(
            RuntimeFact::MediaHealth {
                connection_state: "connected",
                connected_elapsed_ms: 15_000,
                inactivity_elapsed_ms: 15_000,
            },
            false,
        );
        assert_eq!(reason, Some(RuntimeReconnectReason::MediaStalled));
    }

    #[test]
    fn decide_runtime_recovery_ignores_media_health_before_grace_or_threshold() {
        let reason_before_grace = decide_runtime_recovery(
            RuntimeFact::MediaHealth {
                connection_state: "connected",
                connected_elapsed_ms: 9_000,
                inactivity_elapsed_ms: 20_000,
            },
            false,
        );
        assert_eq!(reason_before_grace, None);

        let reason_before_threshold = decide_runtime_recovery(
            RuntimeFact::MediaHealth {
                connection_state: "connected",
                connected_elapsed_ms: 20_000,
                inactivity_elapsed_ms: 10_000,
            },
            false,
        );
        assert_eq!(reason_before_threshold, None);
    }
}
