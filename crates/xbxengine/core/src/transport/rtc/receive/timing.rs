use std::time::Duration;

use xbxengine_protocol::XbxEngineTargetTypeDto;

/// RTT 感知下的单次 NACK 维护参数（由 [`ReceiveTimingProfile`] + 有效 RTT 导出）。
#[derive(Clone, Copy, Debug)]
pub struct NackSchedulingParams {
    pub reorder_wait: Duration,
    pub first_nack: Duration,
    pub retry_interval: Duration,
    pub max_retries: u8,
    pub nack_timeout: Duration,
    pub keyframe_escalation_dwell: Duration,
}

/// RFC §5：receiver-local NACK / keyframe 时序（毫秒）。
#[derive(Clone, Copy, Debug)]
pub struct ReceiveTimingProfile {
    pub reorder_wait_ms: u64,
    pub first_nack_ms: u64,
    pub nack_retry_ms: u64,
    pub keyframe_fallback_ms: u64,
    pub max_nack_retries: u8,
}

impl ReceiveTimingProfile {
    pub fn for_target(target: Option<XbxEngineTargetTypeDto>) -> Self {
        match target {
            Some(XbxEngineTargetTypeDto::Cloud) => Self {
                reorder_wait_ms: 12,
                first_nack_ms: 18,
                nack_retry_ms: 60,
                keyframe_fallback_ms: 200,
                max_nack_retries: 10,
            },
            _ => Self {
                reorder_wait_ms: 5,
                first_nack_ms: 6,
                nack_retry_ms: 12,
                keyframe_fallback_ms: 48,
                max_nack_retries: 20,
            },
        }
    }

    /// 由 profile 下限 + 有效 RTT 导出 libwebrtc 风格的 NACK 调度参数。
    pub fn nack_scheduling_params(&self, effective_rtt_ms: f64) -> NackSchedulingParams {
        let rtt_ms = effective_rtt_ms.clamp(5.0, 800.0);
        let reorder_wait_ms = self.reorder_wait_ms.max((0.5 * rtt_ms).round() as u64);
        let first_nack_ms = self.first_nack_ms.max(rtt_ms.round() as u64 + 5);
        let retry_interval_ms =
            crate::transport::rtc::recovery::timing::nack_retry_interval_u64_from_rtt_ms(rtt_ms);
        let nack_timeout_ms = (1.6 * rtt_ms + 40.0).clamp(45.0, 420.0).round() as u64;
        let dwell_ms = (80.0_f64).max(1.5 * rtt_ms).min(200.0).round() as u64;
        NackSchedulingParams {
            reorder_wait: Duration::from_millis(reorder_wait_ms),
            first_nack: Duration::from_millis(first_nack_ms),
            retry_interval: Duration::from_millis(retry_interval_ms),
            max_retries: self.max_nack_retries,
            nack_timeout: Duration::from_millis(nack_timeout_ms),
            keyframe_escalation_dwell: Duration::from_millis(dwell_ms),
        }
    }
}
