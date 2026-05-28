use std::time::{Duration, Instant};

use crate::transport::rtc::capability::{
    KeyframeRequestKind, KeyframeSendOutcome, RtcTransportCapability,
};
use crate::transport::rtc::recovery::contract::{sparse_idr_pli_interval_ms, SparseIdrRhythm};

use super::timing::ReceiveTimingProfile;

/// PLI/FIR 调度结果：与 transport `KeyframeSendOutcome` 解耦，覆盖节流与同拍合并。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyframeRequestDispatch {
    Sent(KeyframeSendOutcome),
    /// 本路径因节奏未发；窗口内尚无其它成功发送。
    Throttled,
    /// 同间隔内已有其它路径成功发送 PLI/FIR，本路径不再重复发。
    Coalesced,
}

/// receiver-local PLI/FIR 调度：失败只更新本地退避，不进入全局 recovery。
pub struct KeyframeRequester {
    _timing: ReceiveTimingProfile,
    last_pli_at: Option<Instant>,
    consecutive_pli_without_idr: u8,
    pli_retry_interval: Duration,
    sparse_pli_retry_interval: Duration,
    fir_after_pli_count: u8,
}

impl KeyframeRequester {
    pub fn new(timing: ReceiveTimingProfile) -> Self {
        let effective_rtt_ms = timing.keyframe_fallback_ms as f64;
        let sparse_ms = sparse_idr_pli_interval_ms(effective_rtt_ms.max(48.0));
        Self {
            _timing: timing,
            last_pli_at: None,
            consecutive_pli_without_idr: 0,
            pli_retry_interval: Duration::from_millis((timing.keyframe_fallback_ms / 4).max(24)),
            sparse_pli_retry_interval: Duration::from_millis(sparse_ms.round() as u64),
            fir_after_pli_count: 2,
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

    fn pli_interval_for_rhythm(&self, rhythm: SparseIdrRhythm) -> Duration {
        if rhythm.active {
            Duration::from_millis(rhythm.pli_interval_ms.round().max(12.0) as u64)
        } else {
            self.pli_retry_interval
        }
    }

    fn should_request_keyframe_with_interval(&self, interval: Duration) -> bool {
        self.last_pli_at.is_none_or(|t| t.elapsed() >= interval)
    }

    /// 同间隔内是否已发送过 PLI（用于 coalesce，避免 sent/throttled 打架）。
    fn pli_sent_within_interval(&self, interval: Duration) -> bool {
        self.last_pli_at.is_some_and(|t| t.elapsed() < interval)
    }

    pub fn should_request_keyframe(&self) -> bool {
        self.should_request_keyframe_with_interval(self.pli_retry_interval)
    }

    pub fn request_if_due<C: RtcTransportCapability + ?Sized>(
        &mut self,
        capability: &C,
        force: bool,
        rhythm: SparseIdrRhythm,
    ) -> KeyframeRequestDispatch {
        self.request_dispatch(capability, force, rhythm)
    }

    pub fn request_dispatch<C: RtcTransportCapability + ?Sized>(
        &mut self,
        capability: &C,
        force: bool,
        rhythm: SparseIdrRhythm,
    ) -> KeyframeRequestDispatch {
        let interval = self.pli_interval_for_rhythm(rhythm);
        if self.pli_sent_within_interval(interval) {
            return KeyframeRequestDispatch::Coalesced;
        }
        if !force {
            if rhythm.active && !rhythm.pli_due {
                return KeyframeRequestDispatch::Throttled;
            }
            if !self.should_request_keyframe_with_interval(interval) {
                return KeyframeRequestDispatch::Throttled;
            }
        }
        let kind = if self.consecutive_pli_without_idr >= self.fir_after_pli_count {
            KeyframeRequestKind::Fir
        } else {
            KeyframeRequestKind::Pli
        };
        let outcome = capability.send_keyframe(kind);
        if matches!(outcome, KeyframeSendOutcome::Sent) {
            self.on_pli_sent();
            KeyframeRequestDispatch::Sent(outcome)
        } else {
            KeyframeRequestDispatch::Sent(outcome)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::rtc::capability::TestTransportCapability;
    use crate::transport::rtc::receive::timing::ReceiveTimingProfile;

    #[test]
    fn second_request_within_interval_is_coalesced_not_throttled() {
        let mut requester = KeyframeRequester::new(ReceiveTimingProfile::for_target(None));
        let capability = TestTransportCapability;
        let rhythm = SparseIdrRhythm::default();
        let first = requester.request_dispatch(&capability, true, rhythm);
        assert_eq!(
            first,
            KeyframeRequestDispatch::Sent(KeyframeSendOutcome::Sent)
        );
        let second = requester.request_dispatch(&capability, false, rhythm);
        assert_eq!(second, KeyframeRequestDispatch::Coalesced);
    }
}
