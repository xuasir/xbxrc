//! 控制面使用的 escalation 原因标签：仅从结构化来源读取，**不**回退 `recovery_diagnosis`。
//!
//! 顺序：`recovery_active_escalation_reason`（policy 每拍）→ `latest_video_escalation_observation.reason`
//! → `video_owner_reason`。DTO 的 `recovery_diagnosis` 由 diagnostics `stats` 的 `resolve_recovery_diagnosis`（owner 合同 + runtime_state 有效标签）组装，不再回退裸 `stats.recovery_diagnosis`。

use crate::XbxEngineMediaRuntimeStats;

/// Session phase / owner mode / recovery stage 等控制逻辑只应使用本函数；勿把 `recovery_diagnosis` 当控制输入。
pub(crate) fn escalation_structured_label(stats: &XbxEngineMediaRuntimeStats) -> Option<&str> {
    stats
        .recovery_active_escalation_reason
        .as_deref()
        .or_else(|| {
            stats
                .latest_video_escalation_observation
                .as_ref()
                .map(|o| o.reason.as_str())
        })
        .or_else(|| stats.video_owner_reason.as_deref())
}
