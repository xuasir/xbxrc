use std::time::{Duration, Instant};

use crate::transport::rtc::capability::{
    KeyframeRequestKind, KeyframeSendOutcome, RtcTransportCapability,
};

use super::timing::ReceiveTimingProfile;

/// receiver-local PLI/FIR 调度：失败只更新本地退避，不进入全局 recovery。
pub struct KeyframeRequester {
    timing: ReceiveTimingProfile,
    last_pli_at: Option<Instant>,
    consecutive_pli_without_idr: u8,
    pli_retry_interval: Duration,
    fir_after_pli_count: u8,
}

impl KeyframeRequester {
    pub fn new(timing: ReceiveTimingProfile) -> Self {
        Self {
            timing,
            last_pli_at: None,
            consecutive_pli_without_idr: 0,
            pli_retry_interval: Duration::from_millis(timing.keyframe_fallback_ms / 2),
            fir_after_pli_count: 3,
        }
    }

    pub fn on_idr_received(&mut self) {
        self.consecutive_pli_without_idr = 0;
        self.last_pli_at = None;
    }

    pub fn on_pli_sent(&mut self) {
        self.consecutive_pli_without_idr = self.consecutive_pli_without_idr.saturating_add(1);
        self.last_pli_at = Some(Instant::now());
    }

    pub fn should_request_keyframe(&self) -> bool {
        self.last_pli_at.is_none()
            || self
                .last_pli_at
                .is_some_and(|t| t.elapsed() >= self.pli_retry_interval)
    }

    pub fn request_if_due<C: RtcTransportCapability + ?Sized>(
        &mut self,
        capability: &C,
        force: bool,
    ) -> Option<KeyframeSendOutcome> {
        if !force && !self.should_request_keyframe() {
            return None;
        }
        let kind = if self.consecutive_pli_without_idr >= self.fir_after_pli_count {
            KeyframeRequestKind::Fir
        } else {
            KeyframeRequestKind::Pli
        };
        let outcome = capability.send_keyframe(kind);
        if matches!(outcome, KeyframeSendOutcome::Sent) {
            self.on_pli_sent();
        }
        Some(outcome)
    }
}
