use xbxengine_protocol::XbxEngineTargetTypeDto;

/// RFC §5：receiver-local NACK / keyframe 时序（毫秒）。
#[derive(Clone, Copy, Debug)]
pub struct ReceiveTimingProfile {
    pub reorder_wait_ms: u64,
    pub first_nack_ms: u64,
    pub nack_retry_ms: u64,
    pub keyframe_fallback_ms: u64,
}

impl ReceiveTimingProfile {
    pub fn for_target(target: Option<XbxEngineTargetTypeDto>) -> Self {
        match target {
            Some(XbxEngineTargetTypeDto::Cloud) => Self {
                reorder_wait_ms: 12,
                first_nack_ms: 18,
                nack_retry_ms: 60,
                keyframe_fallback_ms: 200,
            },
            _ => Self {
                reorder_wait_ms: 5,
                first_nack_ms: 6,
                nack_retry_ms: 12,
                keyframe_fallback_ms: 48,
            },
        }
    }
}
