//! 原生连接事务：所有 reconnect 来源共用授权、动作预算和新帧观察。
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::json;
use xbxengine_protocol::{XbxEngineReconnectReasonDto, XbxEngineTransportStateDto};

use super::{
    XbxEngineEventSink, XbxEngineHostBridge, XbxEngineReconnectTriggerSource, XbxEngineRuntime,
    XbxEngineRuntimeError,
};
use crate::{XbxEngineMediaBackend, XbxEngineMediaRuntimeStats};

const RECOVERY_TIMEOUT: Duration = Duration::from_secs(120);
const STABLE_WINDOW: Duration = Duration::from_secs(5);
const MAX_FRAME_GAP: Duration = Duration::from_millis(1_500);
static ADMISSION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct Transaction {
    id: String,
    started: Instant,
    deadline: Instant,
    observing: bool,
    stable_since: Option<Instant>,
    last_frame_at: Option<Instant>,
    last_frame: Option<(u64, f64)>,
}

#[derive(Clone, Default)]
pub(super) struct ConnectionRecoveryState {
    active: Option<Transaction>,
    actions: u8,
}

impl ConnectionRecoveryState {
    fn begin(&mut self, now: Instant) -> Result<bool, XbxEngineRuntimeError> {
        if self.active.is_some() {
            return Ok(false);
        }
        if self.actions >= 2 {
            return Err(XbxEngineRuntimeError::new(
                "connectionRecoveryEpisodeExhausted",
            ));
        }
        self.active = Some(Transaction {
            id: format!(
                "native-connection-{}",
                ADMISSION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ),
            started: now,
            deadline: now + RECOVERY_TIMEOUT,
            observing: false,
            stable_since: None,
            last_frame_at: None,
            last_frame: None,
        });
        Ok(true)
    }

    pub(super) fn ensure_active(&self) -> Result<(), XbxEngineRuntimeError> {
        if self
            .active
            .as_ref()
            .is_some_and(|transaction| Instant::now() >= transaction.deadline)
        {
            return Err(XbxEngineRuntimeError::new("connectionRecoveryTimeout"));
        }
        Ok(())
    }

    fn observe(&mut self, now: Instant, frame: Option<(u64, f64)>) -> bool {
        let Some(transaction) = self
            .active
            .as_mut()
            .filter(|transaction| transaction.observing)
        else {
            return false;
        };
        let Some(frame) = frame else {
            transaction.stable_since = None;
            return false;
        };
        if transaction
            .last_frame
            .is_some_and(|last| last.0 == frame.0 || last.1 == frame.1)
        {
            return false;
        }
        transaction.last_frame = Some(frame);
        if transaction
            .last_frame_at
            .is_none_or(|last| now.duration_since(last) > MAX_FRAME_GAP)
        {
            transaction.stable_since = Some(now);
        }
        transaction.last_frame_at = Some(now);
        transaction
            .stable_since
            .is_some_and(|since| now.duration_since(since) >= STABLE_WINDOW)
    }
}

impl<H: XbxEngineHostBridge, E: XbxEngineEventSink, M: XbxEngineMediaBackend>
    XbxEngineRuntime<H, E, M>
{
    pub fn request_reconnect(
        &mut self,
        reason: XbxEngineReconnectReasonDto,
        source: XbxEngineReconnectTriggerSource,
    ) -> Result<(), XbxEngineRuntimeError> {
        let session_id = self.require_session_id()?;
        if !self.connection_recovery.begin(Instant::now())? {
            self.trace_connection_recovery(
                "recoveryArbiterSuppressed",
                json!({"suppressedBy": "singleFlight"}),
            );
            return Ok(());
        }
        let epoch = self.operation_epoch();
        if let Some(deadline) = self.host_bridge.connection_recovery_deadline() {
            if let Some(transaction) = self.connection_recovery.active.as_mut() {
                transaction.deadline = transaction.deadline.min(deadline);
            }
        }
        self.host_bridge.set_operation_epoch(Some(epoch));
        let deadline = self
            .connection_recovery
            .active
            .as_ref()
            .map(|transaction| transaction.deadline);
        self.host_bridge.set_connection_recovery_deadline(deadline);
        self.trace_connection_recovery(
            "operationStarted",
            json!({"deadlineMs": RECOVERY_TIMEOUT.as_millis(), "clockDomain": "monotonic"}),
        );
        self.trace_connection_recovery(
            "recoveryProposed",
            json!({"reason": format!("{reason:?}"), "triggerSource": source.as_str()}),
        );
        let result = (|| {
            self.ensure_operation_active(epoch)?;
            if !self.host_bridge.allows_connection_recovery(&session_id)? {
                return Err(XbxEngineRuntimeError::new("xbxEngineRuntimeCancelled"));
            }
            self.ensure_operation_active(epoch)?;
            self.trace_connection_recovery(
                "recoveryGranted",
                json!({"reason": format!("{reason:?}")}),
            );
            self.connection_recovery.actions += 1;
            self.trace_connection_recovery("recoveryCommitted", json!({"reason": format!("{reason:?}"), "actions": self.connection_recovery.actions}));
            self.perform_connection_reconnect(reason, source, epoch)?;
            self.ensure_operation_active(epoch)
        })();
        self.host_bridge.set_connection_recovery_deadline(None);
        self.host_bridge.set_operation_epoch(None);
        match result {
            Ok(()) => {
                // 动作完成时的媒体快照仅作基线，后续新画面才进入稳定观察。
                let baseline = self
                    .media_backend
                    .snapshot_runtime_stats()
                    .ok()
                    .and_then(|stats| fresh_frame_identity(&stats));
                if let Some(transaction) = self.connection_recovery.active.as_mut() {
                    transaction.observing = true;
                    transaction.last_frame = baseline;
                }
                self.trace_connection_recovery(
                    "reconnectResult",
                    json!({"result": "actionCompleted"}),
                );
                Ok(())
            }
            Err(error) => {
                let (event, result) = if error.is_cancelled() {
                    ("operationCancelled", "cancelled")
                } else if error.to_string().contains("Timeout")
                    || error.to_string().contains("timed out")
                {
                    ("operationTimedOut", "timedOut")
                } else {
                    ("operationFinished", "failed")
                };
                self.finish_connection_recovery(event, result);
                Err(error)
            }
        }
    }

    pub(super) fn observe_connection_recovery(&mut self, stats: &XbxEngineMediaRuntimeStats) {
        if self
            .connection_recovery
            .observe(Instant::now(), fresh_frame_identity(stats))
        {
            self.trace_connection_recovery(
                "recoveryObserved",
                json!({"result": "stableMedia", "actions": self.connection_recovery.actions}),
            );
            self.finish_connection_recovery("operationFinished", "observed");
            self.connection_recovery.actions = 0;
        }
    }

    pub(super) fn expire_connection_recovery(&mut self) -> bool {
        if self.connection_recovery.ensure_active().is_ok() {
            return false;
        }
        self.finish_connection_recovery("operationTimedOut", "timedOut");
        self.emit_error(
            "connectionRecoveryTimeout",
            "connection recovery produced no stable new frames before deadline".to_owned(),
        );
        self.stop();
        true
    }

    pub(super) fn reset_connection_recovery(&mut self) {
        self.finish_connection_recovery("operationCancelled", "cancelled");
        self.connection_recovery = Default::default();
        self.host_bridge.set_connection_recovery_deadline(None);
    }

    fn finish_connection_recovery(&mut self, event: &str, result: &str) {
        self.trace_connection_recovery(event, json!({"result": result}));
        self.connection_recovery.active = None;
    }

    fn trace_connection_recovery(&self, event: &str, payload: serde_json::Value) {
        let Some(transaction) = self.connection_recovery.active.as_ref() else {
            return;
        };
        let Some(session_id) = self.current_session_id() else {
            return;
        };
        let mut payload = payload;
        payload["operationId"] = json!(transaction.id);
        payload["admissionId"] = json!(transaction.id);
        payload["stage"] = json!("connectionRecovery");
        payload["elapsedMs"] = json!(transaction.started.elapsed().as_millis());
        payload["remainingMs"] = json!(transaction
            .deadline
            .saturating_duration_since(Instant::now())
            .as_millis());
        self.host_bridge
            .on_connection_recovery_event(event, session_id, payload);
    }
}

fn fresh_frame_identity(stats: &XbxEngineMediaRuntimeStats) -> Option<(u64, f64)> {
    if stats.transport_state != XbxEngineTransportStateDto::Connected {
        return None;
    }
    Some((
        stats.last_displayed_frame_seq?,
        stats.latest_video_decode_ok_time_ms?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_action_and_observation_share_single_flight() {
        let mut state = ConnectionRecoveryState::default();
        let now = Instant::now();
        assert!(state.begin(now).unwrap());
        assert!(!state.begin(now).unwrap());
        state.actions = 1;
        state.active.as_mut().unwrap().observing = true;
        assert!(!state.begin(now).unwrap());
        assert!(!state.observe(now, Some((1, 1.0))));
        // 同一画面刷新持续十秒也保留未恢复状态。
        assert!(!state.observe(now + Duration::from_secs(10), Some((1, 1.0))));
        assert!(!state.observe(now + Duration::from_secs(10), Some((1, 100.0))));
        assert!(!state.observe(now + Duration::from_secs(11), Some((2, 2.0))));
        for tick in 12..16 {
            assert!(!state.observe(now + Duration::from_secs(tick), Some((tick, tick as f64))));
        }
        assert!(state.observe(now + Duration::from_secs(16), Some((16, 16.0))));
    }

    #[test]
    fn episode_actions_and_total_deadline_are_bounded() {
        let mut state = ConnectionRecoveryState::default();
        let now = Instant::now();
        state.actions = 2;
        assert!(state.begin(now).is_err());
        state.actions = 0;
        state.begin(now - RECOVERY_TIMEOUT).unwrap();
        assert!(state.ensure_active().is_err());
    }
}
