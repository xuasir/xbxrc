use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::session::store::SessionRuntimeRecord;

/// 会话监控循环的间隔建议值（毫秒）。
pub const SESSION_MONITOR_INTERVAL_MS: u64 = 1000;

/// 会话监控所需的最小动态快照。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionRuntimeSnapshot {
    pub stream_state: Option<String>,
    pub player_state: String,
    pub queue: Option<QueueSnapshot>,
    pub error_details: Option<SessionErrorDetails>,
}

/// 会话监控元数据：供 adapter 持久保存并在每次 tick 后回写。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionMonitorMetadata {
    pub created_at_ms: u64,
    pub last_observed_state: Option<String>,
    pub state_observed_at_ms: Option<u64>,
    pub repeated_state_count: u32,
    pub monitor_attempt_count: u32,
    /// 会话是否曾经观测到 `Provisioned`。
    ///
    /// 该事实一旦成立就不再回退：启动期「卡在 Provisioning / ReadyToConnect」
    /// 的超时门槛只对还没 Provisioned 过的会话成立。播放期服务端把状态退回
    /// `Provisioning` / `ReadyToConnect`（重新准备或重新握手）时，本地媒体链路
    /// 往往仍然在正常出图，此时判成启动超时只会制造「报错但画面还在动」。
    #[serde(default)]
    pub reached_provisioned: bool,
}

/// 会话监控的内部状态，由运行时快照与元数据组合而成。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionMonitorState {
    pub runtime: SessionRuntimeSnapshot,
    pub metadata: SessionMonitorMetadata,
}

/// 一次监控 tick 的输入数据。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionMonitorInput {
    pub now_ms: u64,
    /// Provisioning / ReadyToConnect 的卡住判定窗口，来自 session schedule。
    pub state_timeout_ms: u64,
    pub stream_state: Option<String>,
    pub error_details: Option<SessionErrorDetails>,
    /// 仅当状态为 WaitingForResources 时生效。
    pub waiting_queue: Option<QueueDetails>,
}

/// 一次监控 tick 的输出结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionMonitorResult {
    pub runtime: SessionRuntimeSnapshot,
    pub metadata: SessionMonitorMetadata,
    pub should_continue: bool,
    pub should_send_connect_token: bool,
}

/// 将业务快照绑定为可读写 `SessionRuntimeSnapshot`。
pub trait SessionRuntimeBinding {
    fn runtime_snapshot(&self) -> SessionRuntimeSnapshot;
    fn replace_runtime_snapshot(&mut self, runtime: SessionRuntimeSnapshot);
}

/// record 级 monitor 执行结果，仅暴露编排层关心的控制信号。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionMonitorControl {
    pub should_continue: bool,
    pub should_send_connect_token: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionErrorDetails {
    pub code: Option<Value>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct QueueDetails {
    pub estimated_total_wait_time_in_seconds: Option<u64>,
    pub estimated_allocation_time_in_seconds: Option<u64>,
    pub estimated_provisioning_time_in_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueueSnapshot {
    pub details: QueueDetails,
}

/// 纯状态机：根据当前状态与观测结果推进 session 状态。
pub fn apply_monitor_tick(
    mut state: SessionMonitorState,
    input: SessionMonitorInput,
) -> SessionMonitorResult {
    state.metadata.monitor_attempt_count += 1;

    if state.metadata.last_observed_state == input.stream_state {
        state.metadata.repeated_state_count += 1;
    } else {
        state.metadata.last_observed_state = input.stream_state.clone();
        state.metadata.state_observed_at_ms = Some(input.now_ms);
        state.metadata.repeated_state_count = 1;
    }

    // 启动期超时门槛只对「还没进入过 Provisioned」的会话成立。
    // 一旦会话已经 Provisioned 过（也就是已经可以出图），后续再把
    // Provisioning / ReadyToConnect 判成启动超时就是误报：播放期状态回退
    // 由下面的 match 分支吸收，不再升级成 failed。
    let timeout_error = if state.metadata.reached_provisioned {
        None
    } else {
        get_state_timeout_error(
            &state,
            input.now_ms,
            input.stream_state.as_deref(),
            input.state_timeout_ms,
        )
    };
    if let Some(timeout_error) = timeout_error {
        state.runtime.player_state = "failed".to_string();
        state.runtime.stream_state = input.stream_state;
        state.runtime.error_details = Some(timeout_error);
        return SessionMonitorResult {
            runtime: state.runtime,
            metadata: state.metadata,
            should_continue: false,
            should_send_connect_token: false,
        };
    }

    match input.stream_state.as_deref() {
        Some("Provisioned") => {
            state.metadata.reached_provisioned = true;
            state.runtime.player_state = "started".to_string();
            state.runtime.stream_state = input.stream_state;
            state.runtime.queue = None;
            state.runtime.error_details = None;
            SessionMonitorResult {
                runtime: state.runtime,
                metadata: state.metadata,
                should_continue: false,
                should_send_connect_token: false,
            }
        }
        // 播放期状态回退：会话已经 Provisioned 过，服务端又退回 Provisioning /
        // ReadyToConnect，说明它正在重新准备或等待重新握手，而不是「启动卡死」。
        // 这里不做降级也不结束监控，只把真实状态写回快照供 trace 复盘；
        // 播放期的媒体链路与 session 状态机是两条独立的路径，不能因为一个
        // 状态字符串就把已经在出图的会话判成 failed。
        Some("Provisioning" | "ReadyToConnect") if state.metadata.reached_provisioned => {
            state.runtime.stream_state = input.stream_state;
            SessionMonitorResult {
                runtime: state.runtime,
                metadata: state.metadata,
                should_continue: true,
                should_send_connect_token: false,
            }
        }
        Some("Provisioning") => {
            state.runtime.player_state = "pending".to_string();
            state.runtime.stream_state = input.stream_state;
            state.runtime.error_details = None;
            SessionMonitorResult {
                runtime: state.runtime,
                metadata: state.metadata,
                should_continue: true,
                should_send_connect_token: false,
            }
        }
        Some("ReadyToConnect") => {
            state.runtime.player_state = "pending".to_string();
            state.runtime.stream_state = input.stream_state;
            state.runtime.error_details = None;
            SessionMonitorResult {
                runtime: state.runtime,
                metadata: state.metadata,
                should_continue: true,
                should_send_connect_token: true,
            }
        }
        Some("WaitingForResources") => {
            state.runtime.player_state = "queued".to_string();
            state.runtime.stream_state = input.stream_state;
            state.runtime.queue = Some(QueueSnapshot {
                details: input.waiting_queue.unwrap_or_default(),
            });
            state.runtime.error_details = None;
            SessionMonitorResult {
                runtime: state.runtime,
                metadata: state.metadata,
                should_continue: true,
                should_send_connect_token: false,
            }
        }
        Some("Failed") => {
            state.runtime.player_state = "failed".to_string();
            state.runtime.stream_state = input.stream_state;
            state.runtime.error_details = input.error_details;
            SessionMonitorResult {
                runtime: state.runtime,
                metadata: state.metadata,
                should_continue: false,
                should_send_connect_token: false,
            }
        }
        _ => {
            // 播放期拿到空 body / 未知枚举时不要降级：会话已经出图，写回 pending
            // 只会让 UI 莫名退回「等待会话就绪」。Closing / Closed / Recovering
            // 是真实阶段信号，仍然照常驱动 phase。
            let keeps_playback_phase = state.metadata.reached_provisioned
                && !matches!(
                    input.stream_state.as_deref(),
                    Some("Closing" | "Closed" | "Recovering")
                );
            if !keeps_playback_phase {
                state.runtime.player_state = "pending".to_string();
            }
            state.runtime.stream_state = input.stream_state;
            SessionMonitorResult {
                runtime: state.runtime,
                metadata: state.metadata,
                should_continue: true,
                should_send_connect_token: false,
            }
        }
    }
}

/// 对运行态 record 执行一次 monitor tick，并原地回写快照与元数据。
pub fn apply_monitor_tick_to_record<S>(
    record: &mut SessionRuntimeRecord<S>,
    input: SessionMonitorInput,
) -> SessionMonitorControl
where
    S: SessionRuntimeBinding + Clone,
{
    let state = SessionMonitorState {
        runtime: record.snapshot.runtime_snapshot(),
        metadata: record.metadata.clone(),
    };
    let result = apply_monitor_tick(state, input);

    record.snapshot.replace_runtime_snapshot(result.runtime);
    record.metadata = result.metadata;

    SessionMonitorControl {
        should_continue: result.should_continue,
        should_send_connect_token: result.should_send_connect_token,
    }
}

fn get_state_timeout_error(
    state: &SessionMonitorState,
    now_ms: u64,
    stream_state: Option<&str>,
    state_timeout_ms: u64,
) -> Option<SessionErrorDetails> {
    if stream_state != Some("Provisioning") && stream_state != Some("ReadyToConnect") {
        return None;
    }

    let started = state
        .metadata
        .state_observed_at_ms
        .unwrap_or(state.metadata.created_at_ms);
    let elapsed = now_ms.saturating_sub(started);
    if elapsed < state_timeout_ms.max(1_000) {
        return None;
    }

    Some(SessionErrorDetails {
        code: Some(Value::String("SessionStateTimeout".to_string())),
        message: Some(format!(
            "Streaming session stayed in {} for {}ms.",
            stream_state.unwrap_or("unknown"),
            elapsed
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        apply_monitor_tick, apply_monitor_tick_to_record, QueueDetails, SessionMonitorInput,
        SessionMonitorMetadata, SessionMonitorState, SessionRuntimeBinding, SessionRuntimeSnapshot,
    };
    use crate::session::store::SessionRuntimeRecord;

    const TEST_TIMEOUT_MS: u64 = 45_000;

    fn sample_state() -> SessionMonitorState {
        SessionMonitorState {
            runtime: SessionRuntimeSnapshot {
                stream_state: None,
                player_state: "pending".to_string(),
                queue: None,
                error_details: None,
            },
            metadata: SessionMonitorMetadata {
                created_at_ms: 1000,
                last_observed_state: None,
                state_observed_at_ms: None,
                repeated_state_count: 0,
                monitor_attempt_count: 0,
                reached_provisioned: false,
            },
        }
    }

    #[derive(Debug, Clone)]
    struct SnapshotBindingStub {
        runtime: SessionRuntimeSnapshot,
    }

    impl SessionRuntimeBinding for SnapshotBindingStub {
        fn runtime_snapshot(&self) -> SessionRuntimeSnapshot {
            self.runtime.clone()
        }

        fn replace_runtime_snapshot(&mut self, runtime: SessionRuntimeSnapshot) {
            self.runtime = runtime;
        }
    }

    #[test]
    fn waiting_for_resources_sets_queued_state_and_queue_snapshot() {
        let state = sample_state();
        let result = apply_monitor_tick(
            state,
            SessionMonitorInput {
                now_ms: 2000,
                state_timeout_ms: TEST_TIMEOUT_MS,
                stream_state: Some("WaitingForResources".to_string()),
                error_details: None,
                waiting_queue: Some(QueueDetails {
                    estimated_total_wait_time_in_seconds: Some(10),
                    estimated_allocation_time_in_seconds: None,
                    estimated_provisioning_time_in_seconds: None,
                }),
            },
        );

        assert_eq!(result.runtime.player_state, "queued");
        assert!(result.runtime.queue.is_some());
        assert!(result.should_continue);
    }

    #[test]
    fn ready_to_connect_requests_connect_token() {
        let state = sample_state();
        let result = apply_monitor_tick(
            state,
            SessionMonitorInput {
                now_ms: 2000,
                state_timeout_ms: TEST_TIMEOUT_MS,
                stream_state: Some("ReadyToConnect".to_string()),
                error_details: None,
                waiting_queue: None,
            },
        );

        assert!(result.should_continue);
        assert!(result.should_send_connect_token);
    }

    #[test]
    fn provisioning_timeout_marks_failed_and_stops() {
        let mut state = sample_state();
        state.metadata.state_observed_at_ms = Some(1000);
        state.metadata.last_observed_state = Some("Provisioning".to_string());

        let result = apply_monitor_tick(
            state,
            SessionMonitorInput {
                now_ms: 1000 + TEST_TIMEOUT_MS + 1,
                state_timeout_ms: TEST_TIMEOUT_MS,
                stream_state: Some("Provisioning".to_string()),
                error_details: None,
                waiting_queue: None,
            },
        );

        assert_eq!(result.runtime.player_state, "failed");
        assert!(!result.should_continue);
        assert!(result.runtime.error_details.is_some());
    }

    /// 会话已经 Provisioned 过（已经出图）后的监控状态。
    fn started_state() -> SessionMonitorState {
        let mut state = sample_state();
        state.runtime.player_state = "started".to_string();
        state.runtime.stream_state = Some("Provisioned".to_string());
        state.metadata.last_observed_state = Some("Provisioned".to_string());
        state.metadata.state_observed_at_ms = Some(1000);
        state.metadata.reached_provisioned = true;
        state
    }

    #[test]
    fn ready_to_connect_after_provisioned_does_not_fail_playback() {
        // 真机现象：串流游玩一会后报「会话状态超时」，但画面仍在正常出。
        // 根因是播放期服务端把已 Provisioned 的会话退回 ReadyToConnect，
        // 启动期超时门槛继续生效，把已经在出图的会话判成 failed。
        let state = started_state();
        let result = apply_monitor_tick(
            state,
            SessionMonitorInput {
                now_ms: 1000 + TEST_TIMEOUT_MS * 2,
                state_timeout_ms: TEST_TIMEOUT_MS,
                stream_state: Some("ReadyToConnect".to_string()),
                error_details: None,
                waiting_queue: None,
            },
        );

        assert_eq!(result.runtime.player_state, "started");
        assert!(result.runtime.error_details.is_none());
        assert!(result.should_continue);
        assert!(!result.should_send_connect_token);
        assert_eq!(
            result.runtime.stream_state.as_deref(),
            Some("ReadyToConnect")
        );
    }

    #[test]
    fn provisioning_after_provisioned_does_not_regress_to_pending() {
        let state = started_state();
        let result = apply_monitor_tick(
            state,
            SessionMonitorInput {
                now_ms: 1000 + TEST_TIMEOUT_MS * 2,
                state_timeout_ms: TEST_TIMEOUT_MS,
                stream_state: Some("Provisioning".to_string()),
                error_details: None,
                waiting_queue: None,
            },
        );

        assert_eq!(result.runtime.player_state, "started");
        assert!(result.should_continue);
    }

    #[test]
    fn empty_state_after_provisioned_keeps_playback_phase() {
        let state = started_state();
        let result = apply_monitor_tick(
            state,
            SessionMonitorInput {
                now_ms: 2000,
                state_timeout_ms: TEST_TIMEOUT_MS,
                stream_state: None,
                error_details: None,
                waiting_queue: None,
            },
        );

        assert_eq!(result.runtime.player_state, "started");
        assert!(result.should_continue);
    }

    #[test]
    fn closed_after_provisioned_still_drives_phase() {
        let state = started_state();
        let result = apply_monitor_tick(
            state,
            SessionMonitorInput {
                now_ms: 2000,
                state_timeout_ms: TEST_TIMEOUT_MS,
                stream_state: Some("Closed".to_string()),
                error_details: None,
                waiting_queue: None,
            },
        );

        assert_eq!(result.runtime.player_state, "pending");
        assert_eq!(result.runtime.stream_state.as_deref(), Some("Closed"));
    }

    #[test]
    fn apply_monitor_tick_to_record_updates_record_fields() {
        let snapshot = SnapshotBindingStub {
            runtime: SessionRuntimeSnapshot {
                stream_state: None,
                player_state: "pending".to_string(),
                queue: None,
                error_details: None,
            },
        };
        let mut record = SessionRuntimeRecord::new(snapshot, crate::policy::Plan::default(), 1000);

        let control = apply_monitor_tick_to_record(
            &mut record,
            SessionMonitorInput {
                now_ms: 2000,
                state_timeout_ms: TEST_TIMEOUT_MS,
                stream_state: Some("Provisioned".to_string()),
                error_details: None,
                waiting_queue: None,
            },
        );

        assert!(!control.should_continue);
        assert!(!control.should_send_connect_token);
        assert_eq!(record.snapshot.runtime.player_state, "started");
        assert_eq!(
            record.metadata.last_observed_state.as_deref(),
            Some("Provisioned")
        );
        assert_eq!(record.metadata.monitor_attempt_count, 1);
    }
}
