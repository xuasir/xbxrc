use crate::transport::rtc::facts::ConnectionLifecycleStateFact;
use crate::transport::rtc::policy::display_supply::{DisplaySupplyState, SchedulingDemandSignal};
use crate::transport::rtc::recovery::policy::DisplaySupplyThresholds;
use crate::{XbxEngineAnchorCandidateLedger, XbxEngineAnchorCandidateState};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VideoSchedulingOwnerState {
    SeekingAnchor,
    Priming,
    RebuildingSupply,
    StableServing,
    DegradedServing,
    SupplyStarved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VideoHealthContract {
    Startup,
    Recovering,
    Stable,
    Starved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecoveryIntentSource {
    Anchor,
    Supply,
}

impl RecoveryIntentSource {
    pub(crate) fn as_contract_source(self) -> VideoSchedulingOwnerContractSource {
        match self {
            RecoveryIntentSource::Anchor => VideoSchedulingOwnerContractSource::Anchor,
            RecoveryIntentSource::Supply => VideoSchedulingOwnerContractSource::Supply,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VideoSchedulingOwnerContractSource {
    Anchor,
    Supply,
    Steady,
}

impl VideoSchedulingOwnerContractSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Anchor => "anchor",
            Self::Supply => "supply",
            Self::Steady => "steady",
        }
    }
}

impl VideoSchedulingOwnerState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SeekingAnchor => "seeking-anchor",
            Self::Priming => "priming",
            Self::RebuildingSupply => "rebuilding-supply",
            Self::StableServing => "stable-serving",
            Self::DegradedServing => "degraded-serving",
            Self::SupplyStarved => "supply-starved",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct VideoSchedulingOwnerInput {
    pub(crate) connection_state: ConnectionLifecycleStateFact,
    pub(crate) recovery_epoch: u64,
    pub(crate) anchor_reason_label: Option<String>,
    pub(crate) demand: SchedulingDemandSignal,
    pub(crate) clean_anchor_epoch: Option<u64>,
    pub(crate) clean_anchor_observed_at_ms: Option<f64>,
    pub(crate) clean_anchor_source_event: Option<String>,
    pub(crate) latest_anchor_candidate_ledger: Option<XbxEngineAnchorCandidateLedger>,
    pub(crate) latest_timeline_chain_state: Option<String>,
    pub(crate) latest_timeline_source_event: Option<String>,
    pub(crate) latest_track_state: Option<String>,
    pub(crate) latest_track_video_bytes_total: Option<u64>,
    pub(crate) display_supply_thresholds: DisplaySupplyThresholds,
    pub(crate) observed_at_ms: f64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoveryIntentContract {
    pub(crate) emit: bool,
    pub(crate) source: RecoveryIntentSource,
    pub(crate) reason_label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VideoSchedulingOwnerOutput {
    pub(crate) state: VideoSchedulingOwnerState,
    pub(crate) health: VideoHealthContract,
    pub(crate) reason_label: String,
    pub(crate) reason_source: VideoSchedulingOwnerContractSource,
    pub(crate) observed_at_ms: f64,
    pub(crate) recovery_intent: Option<RecoveryIntentContract>,
    pub(crate) temporary_diagnostic_summary: Option<String>,
}

#[derive(Clone, Debug)]
struct RecoveryIntentCursor {
    source: RecoveryIntentSource,
    label: String,
    emitted_at_ms: f64,
}

pub(crate) struct VideoSchedulingOwner {
    state: VideoSchedulingOwnerState,
    last_intent: Option<RecoveryIntentCursor>,
    last_recovery_epoch: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecoveryCompletionEvidence {
    NotReady,
    Ready,
}

const CLEAN_ANCHOR_EPOCH_GRACE_MAX_DELTA: u64 = 1;
const CLEAN_ANCHOR_EPOCH_GRACE_WINDOW_MS: f64 = 1_500.0;

impl VideoSchedulingOwner {
    pub(crate) fn new() -> Self {
        Self {
            state: VideoSchedulingOwnerState::SeekingAnchor,
            last_intent: None,
            last_recovery_epoch: None,
        }
    }

    pub(crate) fn evaluate(
        &mut self,
        input: &VideoSchedulingOwnerInput,
    ) -> VideoSchedulingOwnerOutput {
        let current_state = self.state;
        if self
            .last_recovery_epoch
            .is_some_and(|last_epoch| last_epoch != input.recovery_epoch)
        {
            // recovery epoch 进入新轮次时，owner 必须解除上轮 intent 的本地抑制。
            self.last_intent = None;
        }
        self.last_recovery_epoch = Some(input.recovery_epoch);
        let supply_state = input
            .demand
            .classify_display_supply_state(&input.display_supply_thresholds);
        let has_clean_anchor_evidence = Self::has_current_clean_anchor_evidence(input);
        let chain_healthy = matches!(
            input.latest_timeline_chain_state.as_deref(),
            Some("healthy")
        );
        let clean_anchor_hysteresis = Self::clean_anchor_hysteresis_allows_reentry(input);
        let wait_keyframe_rebuild_priority = Self::should_prioritize_wait_keyframe_rebuild(input);
        let has_anchor_issue = if clean_anchor_hysteresis {
            false
        } else if wait_keyframe_rebuild_priority {
            // waitKeyframe + critical noPending 压力下，即使 clean anchor 仍在短窗有效，
            // 也优先回到 anchor 恢复链，避免被 supply-starved 分支压住恢复动作。
            true
        } else if has_clean_anchor_evidence && chain_healthy {
            // clean anchor 已经成立且链路 healthy 时，旧的 anchor reason 不应继续把 owner
            // 锁在 rebuilding-supply，否则 Home 场景会永远收不到 stable-serving。
            false
        } else {
            Self::timeline_indicates_anchor_issue(input) || input.anchor_reason_label.is_some()
        };
        let completion_evidence = Self::resolve_recovery_completion_evidence(
            self.state,
            input,
            supply_state,
            has_anchor_issue,
        );
        let supply_absent = Self::supply_is_absent(input);
        let next = self.transition(
            self.state,
            input,
            input.connection_state,
            has_anchor_issue,
            supply_state,
            completion_evidence,
            supply_absent,
            has_clean_anchor_evidence,
        );
        self.state = next;

        let health = match next {
            VideoSchedulingOwnerState::SeekingAnchor | VideoSchedulingOwnerState::Priming => {
                VideoHealthContract::Startup
            }
            VideoSchedulingOwnerState::RebuildingSupply => VideoHealthContract::Recovering,
            VideoSchedulingOwnerState::StableServing
            | VideoSchedulingOwnerState::DegradedServing => VideoHealthContract::Stable,
            VideoSchedulingOwnerState::SupplyStarved => VideoHealthContract::Starved,
        };

        let recovery_intent = self.build_recovery_intent(next, input, supply_state);
        let (reason_label, reason_source) = if let Some(intent) = recovery_intent.as_ref() {
            (
                intent.reason_label.clone(),
                intent.source.as_contract_source(),
            )
        } else {
            let label = match next {
                VideoSchedulingOwnerState::SeekingAnchor => "seekingAnchor",
                VideoSchedulingOwnerState::Priming => "priming",
                VideoSchedulingOwnerState::StableServing => "steady",
                VideoSchedulingOwnerState::DegradedServing => "degradedSteady",
                VideoSchedulingOwnerState::RebuildingSupply => "rebuildingSupply",
                VideoSchedulingOwnerState::SupplyStarved => "supplyStarved",
            };
            (
                label.to_string(),
                VideoSchedulingOwnerContractSource::Steady,
            )
        };
        let temporary_diagnostic_summary = Self::build_temporary_diagnostic_summary(
            current_state,
            next,
            input,
            supply_state,
            has_clean_anchor_evidence,
            has_anchor_issue,
            completion_evidence,
            &reason_label,
            reason_source,
        );
        VideoSchedulingOwnerOutput {
            state: next,
            health,
            reason_label,
            reason_source,
            observed_at_ms: input.observed_at_ms,
            recovery_intent,
            temporary_diagnostic_summary,
        }
    }

    fn transition(
        &self,
        current: VideoSchedulingOwnerState,
        input: &VideoSchedulingOwnerInput,
        connection_state: ConnectionLifecycleStateFact,
        has_anchor_issue: bool,
        supply_state: DisplaySupplyState,
        completion_evidence: RecoveryCompletionEvidence,
        supply_absent: bool,
        has_clean_anchor_evidence: bool,
    ) -> VideoSchedulingOwnerState {
        if !matches!(
            connection_state,
            ConnectionLifecycleStateFact::Connected | ConnectionLifecycleStateFact::Recovering
        ) {
            return VideoSchedulingOwnerState::SeekingAnchor;
        }
        match current {
            VideoSchedulingOwnerState::SeekingAnchor => {
                if has_anchor_issue {
                    VideoSchedulingOwnerState::RebuildingSupply
                } else if completion_evidence == RecoveryCompletionEvidence::Ready {
                    VideoSchedulingOwnerState::Priming
                } else if matches!(supply_state, DisplaySupplyState::Healthy) && !supply_absent {
                    VideoSchedulingOwnerState::Priming
                } else {
                    VideoSchedulingOwnerState::SupplyStarved
                }
            }
            VideoSchedulingOwnerState::Priming => {
                if has_anchor_issue {
                    VideoSchedulingOwnerState::RebuildingSupply
                } else if completion_evidence == RecoveryCompletionEvidence::Ready {
                    VideoSchedulingOwnerState::StableServing
                } else {
                    VideoSchedulingOwnerState::SupplyStarved
                }
            }
            VideoSchedulingOwnerState::RebuildingSupply => {
                if has_anchor_issue {
                    VideoSchedulingOwnerState::RebuildingSupply
                } else if completion_evidence == RecoveryCompletionEvidence::Ready {
                    VideoSchedulingOwnerState::StableServing
                } else {
                    VideoSchedulingOwnerState::RebuildingSupply
                }
            }
            VideoSchedulingOwnerState::StableServing => {
                if has_anchor_issue {
                    VideoSchedulingOwnerState::RebuildingSupply
                } else if completion_evidence == RecoveryCompletionEvidence::Ready {
                    VideoSchedulingOwnerState::StableServing
                } else if Self::should_absorb_steady_jitter(
                    input,
                    supply_state,
                    has_clean_anchor_evidence,
                ) {
                    VideoSchedulingOwnerState::DegradedServing
                } else {
                    VideoSchedulingOwnerState::SupplyStarved
                }
            }
            VideoSchedulingOwnerState::DegradedServing => {
                if has_anchor_issue {
                    VideoSchedulingOwnerState::RebuildingSupply
                } else if completion_evidence == RecoveryCompletionEvidence::Ready {
                    VideoSchedulingOwnerState::StableServing
                } else if Self::should_absorb_steady_jitter(
                    input,
                    supply_state,
                    has_clean_anchor_evidence,
                ) {
                    VideoSchedulingOwnerState::DegradedServing
                } else {
                    VideoSchedulingOwnerState::SupplyStarved
                }
            }
            VideoSchedulingOwnerState::SupplyStarved => {
                if has_anchor_issue {
                    VideoSchedulingOwnerState::RebuildingSupply
                } else if completion_evidence == RecoveryCompletionEvidence::Ready {
                    VideoSchedulingOwnerState::StableServing
                } else {
                    VideoSchedulingOwnerState::SupplyStarved
                }
            }
        }
    }

    fn resolve_recovery_completion_evidence(
        current: VideoSchedulingOwnerState,
        input: &VideoSchedulingOwnerInput,
        supply_state: DisplaySupplyState,
        has_anchor_issue: bool,
    ) -> RecoveryCompletionEvidence {
        let has_clean_anchor_evidence = Self::has_current_clean_anchor_evidence(input);
        let has_stale_clean_anchor_fact = input.clean_anchor_epoch.is_some_and(|epoch| {
            epoch != input.recovery_epoch
                && input.clean_anchor_source_event.as_deref()
                    == Some("chain-clean-keyframe-submitted")
        }) || input
            .latest_anchor_candidate_ledger
            .as_ref()
            .is_some_and(|candidate| {
                candidate.state == XbxEngineAnchorCandidateState::SubmittedCleanAnchor
                    && candidate.source_event == "chain-clean-keyframe-submitted"
                    && candidate.recovery_epoch != input.recovery_epoch
            });
        let chain_healthy = matches!(
            input.latest_timeline_chain_state.as_deref(),
            Some("healthy")
        );
        if current == VideoSchedulingOwnerState::RebuildingSupply {
            if has_stale_clean_anchor_fact && !has_clean_anchor_evidence {
                return RecoveryCompletionEvidence::NotReady;
            }
            if !has_clean_anchor_evidence
                && !Self::supply_recovery_can_settle_without_explicit_clean_anchor(
                    input,
                    supply_state,
                    chain_healthy,
                )
            {
                return RecoveryCompletionEvidence::NotReady;
            }
        }
        if current == VideoSchedulingOwnerState::SupplyStarved
            && Self::supply_recovery_can_settle_without_explicit_clean_anchor(
                input,
                supply_state,
                chain_healthy,
            )
        {
            return RecoveryCompletionEvidence::Ready;
        }
        if has_anchor_issue || !matches!(supply_state, DisplaySupplyState::Healthy) {
            return RecoveryCompletionEvidence::NotReady;
        }
        let present_fresh = input
            .demand
            .present_age_ms
            .is_some_and(|age| age <= input.display_supply_thresholds.degraded_present_age_ms);
        let decode_fresh = input
            .demand
            .decode_age_ms
            .is_some_and(|age| age <= input.display_supply_thresholds.degraded_decode_age_ms);
        // 恢复到 stable-serving 需要比“可播放”更稳：单点 freshness 不足以说明供给已真正降压。
        if !present_fresh || !decode_fresh {
            return RecoveryCompletionEvidence::NotReady;
        }
        if !Self::supply_pressure_is_settled(input, has_clean_anchor_evidence, chain_healthy) {
            return RecoveryCompletionEvidence::NotReady;
        }
        // audioOnly / 无视频字节属于“供给未恢复”，不能提前回到 stable-serving。
        let track_audio_only = matches!(input.latest_track_state.as_deref(), Some("audioOnly"));
        let track_has_video_bytes = input
            .latest_track_video_bytes_total
            .is_some_and(|bytes| bytes > 0);
        if track_audio_only && !track_has_video_bytes {
            return RecoveryCompletionEvidence::NotReady;
        }
        if !chain_healthy {
            return RecoveryCompletionEvidence::NotReady;
        }
        let recovery_noise_source = matches!(
            input.latest_timeline_source_event.as_deref(),
            Some("frame-await-recovery-keyframe")
                | Some("frame-inspection-rejected-await-keyframe")
                | Some("gap-repair-in-flight")
        );
        if recovery_noise_source && !(has_clean_anchor_evidence && chain_healthy) {
            return RecoveryCompletionEvidence::NotReady;
        }
        RecoveryCompletionEvidence::Ready
    }

    fn supply_recovery_can_settle_without_explicit_clean_anchor(
        input: &VideoSchedulingOwnerInput,
        supply_state: DisplaySupplyState,
        chain_healthy: bool,
    ) -> bool {
        if !matches!(supply_state, DisplaySupplyState::Healthy) || !chain_healthy {
            return false;
        }
        if input.connection_state != ConnectionLifecycleStateFact::Connected {
            return false;
        }
        if input.demand.video_renderer_stalled {
            return false;
        }
        let stable_timeline_source = matches!(
            input.latest_timeline_source_event.as_deref(),
            Some("frame-complete-candidate")
        );
        let present_fresh = input
            .demand
            .present_age_ms
            .is_some_and(|age| age <= input.display_supply_thresholds.degraded_present_age_ms);
        let decode_fresh = input
            .demand
            .decode_age_ms
            .is_some_and(|age| age <= input.display_supply_thresholds.degraded_decode_age_ms);
        let track_attached = matches!(
            input.latest_track_state.as_deref(),
            Some("remoteTrackAttached")
        );
        let track_has_video_bytes = input
            .latest_track_video_bytes_total
            .is_some_and(|bytes| bytes > 0);
        stable_timeline_source
            && present_fresh
            && decode_fresh
            && track_attached
            && track_has_video_bytes
    }

    fn has_current_clean_anchor_evidence(input: &VideoSchedulingOwnerInput) -> bool {
        let explicit_clean_anchor = input.clean_anchor_epoch.is_some_and(|epoch| {
            Self::clean_anchor_epoch_is_usable(
                epoch,
                input.clean_anchor_observed_at_ms,
                input.recovery_epoch,
                input.observed_at_ms,
            )
        }) && input.clean_anchor_source_event.as_deref() == Some("chain-clean-keyframe-submitted");
        if explicit_clean_anchor {
            return true;
        }
        input
            .latest_anchor_candidate_ledger
            .as_ref()
            .is_some_and(|candidate| {
                candidate.state == XbxEngineAnchorCandidateState::SubmittedCleanAnchor
                    && candidate.source_event == "chain-clean-keyframe-submitted"
                    && Self::clean_anchor_epoch_is_usable(
                        candidate.recovery_epoch,
                        Some(candidate.observed_at_ms),
                        input.recovery_epoch,
                        input.observed_at_ms,
                    )
            })
    }

    fn build_temporary_diagnostic_summary(
        current_state: VideoSchedulingOwnerState,
        next_state: VideoSchedulingOwnerState,
        input: &VideoSchedulingOwnerInput,
        supply_state: DisplaySupplyState,
        has_clean_anchor_evidence: bool,
        has_anchor_issue: bool,
        completion_evidence: RecoveryCompletionEvidence,
        reason_label: &str,
        reason_source: VideoSchedulingOwnerContractSource,
    ) -> Option<String> {
        // 临时诊断：只在 clean anchor 可见但 owner 仍未回稳时打点，方便直接回看同拍事实。
        let has_visible_clean_anchor_fact = has_clean_anchor_evidence
            || input.clean_anchor_epoch.is_some()
            || input.clean_anchor_source_event.as_deref() == Some("chain-clean-keyframe-submitted")
            || input
                .latest_anchor_candidate_ledger
                .as_ref()
                .is_some_and(|candidate| {
                    candidate.state == XbxEngineAnchorCandidateState::SubmittedCleanAnchor
                        && candidate.source_event == "chain-clean-keyframe-submitted"
                });
        if current_state != VideoSchedulingOwnerState::RebuildingSupply
            || next_state == VideoSchedulingOwnerState::StableServing
            || !has_visible_clean_anchor_fact
        {
            return None;
        }
        Some(format!(
            "temp owner diag currentState={} nextState={} recoveryEpoch={} cleanAnchorEpoch={:?} cleanAnchorObservedAtMs={:?} cleanAnchorSourceEvent={:?} latestAnchorCandidateState={:?} latestAnchorCandidateEpoch={:?} latestTimelineChainState={:?} latestTimelineSourceEvent={:?} latestTrackState={:?} latestTrackVideoBytesTotal={:?} presentAgeMs={:?} decodeAgeMs={:?} noPendingPressureLevel={:?} noPendingStreak={:?} supplyState={:?} hasCleanAnchorEvidence={} hasAnchorIssue={} completionEvidence={:?} reasonLabel={} reasonSource={}",
            current_state.as_str(),
            next_state.as_str(),
            input.recovery_epoch,
            input.clean_anchor_epoch,
            input.clean_anchor_observed_at_ms,
            input.clean_anchor_source_event.as_deref(),
            input
                .latest_anchor_candidate_ledger
                .as_ref()
                .map(|candidate| candidate.state.as_str()),
            input
                .latest_anchor_candidate_ledger
                .as_ref()
                .map(|candidate| candidate.recovery_epoch),
            input.latest_timeline_chain_state.as_deref(),
            input.latest_timeline_source_event.as_deref(),
            input.latest_track_state.as_deref(),
            input.latest_track_video_bytes_total,
            input.demand.present_age_ms,
            input.demand.decode_age_ms,
            input.demand.no_pending_pressure_level.as_deref(),
            input.demand.no_pending_streak,
            supply_state,
            has_clean_anchor_evidence,
            has_anchor_issue,
            completion_evidence,
            reason_label,
            reason_source.as_str(),
        ))
    }

    fn clean_anchor_epoch_is_usable(
        anchor_epoch: u64,
        anchor_observed_at_ms: Option<f64>,
        current_recovery_epoch: u64,
        now_ms: f64,
    ) -> bool {
        if anchor_epoch == current_recovery_epoch {
            return true;
        }
        if current_recovery_epoch < anchor_epoch {
            return false;
        }
        let epoch_delta = current_recovery_epoch - anchor_epoch;
        if epoch_delta > CLEAN_ANCHOR_EPOCH_GRACE_MAX_DELTA {
            return false;
        }
        anchor_observed_at_ms.is_some_and(|anchor_ms| {
            (now_ms - anchor_ms).max(0.0) <= CLEAN_ANCHOR_EPOCH_GRACE_WINDOW_MS
        })
    }

    fn clean_anchor_hysteresis_allows_reentry(input: &VideoSchedulingOwnerInput) -> bool {
        Self::has_current_clean_anchor_evidence(input)
            && matches!(
                input.latest_timeline_source_event.as_deref(),
                Some(
                    "gap-reorder-pending"
                        | "gap-resolved"
                        | "frame-complete-candidate"
                        | "frame-observed"
                )
            )
    }

    fn should_prioritize_wait_keyframe_rebuild(input: &VideoSchedulingOwnerInput) -> bool {
        if !matches!(
            input.demand.no_pending_pressure_level.as_deref(),
            Some("critical")
        ) {
            return false;
        }
        let no_pending_streak = input.demand.no_pending_streak.unwrap_or_default();
        if no_pending_streak < input.display_supply_thresholds.critical_no_pending_streak {
            return false;
        }
        matches!(
            input.latest_timeline_source_event.as_deref(),
            Some("frame-await-recovery-keyframe" | "frame-inspection-rejected-await-keyframe")
        )
    }

    fn supply_pressure_is_settled(
        input: &VideoSchedulingOwnerInput,
        has_clean_anchor_evidence: bool,
        chain_healthy: bool,
    ) -> bool {
        let pressure_level = input.demand.no_pending_pressure_level.as_deref();
        let no_pending_streak = input.demand.no_pending_streak.unwrap_or_default();
        // Home trace 里 `noPendingFrame` 会在短窗内抖高，但如果已经有 clean anchor
        // 且 present/decode 都是 fresh，就不该因为短暂高压把 owner 卡死在恢复态。
        // 只有持续到更高阈值的高压，才视为真正没有降压。
        if matches!(pressure_level, Some("high" | "critical"))
            && no_pending_streak >= input.display_supply_thresholds.critical_no_pending_streak
        {
            return Self::allows_lingering_no_pending_under_connected_recovery(
                input,
                has_clean_anchor_evidence,
                chain_healthy,
            );
        }
        true
    }

    fn allows_lingering_no_pending_under_connected_recovery(
        input: &VideoSchedulingOwnerInput,
        has_clean_anchor_evidence: bool,
        chain_healthy: bool,
    ) -> bool {
        if input.connection_state != ConnectionLifecycleStateFact::Connected {
            return false;
        }
        if !has_clean_anchor_evidence || !chain_healthy {
            return false;
        }
        if input.demand.video_renderer_stalled {
            return false;
        }
        let present_fresh = input
            .demand
            .present_age_ms
            .is_some_and(|age| age <= input.display_supply_thresholds.degraded_present_age_ms);
        let decode_fresh = input
            .demand
            .decode_age_ms
            .is_some_and(|age| age <= input.display_supply_thresholds.degraded_decode_age_ms);
        let track_attached = matches!(
            input.latest_track_state.as_deref(),
            Some("remoteTrackAttached")
        );
        let track_has_video_bytes = input
            .latest_track_video_bytes_total
            .is_some_and(|bytes| bytes > 0);
        present_fresh && decode_fresh && track_attached && track_has_video_bytes
    }

    fn timeline_indicates_anchor_issue(input: &VideoSchedulingOwnerInput) -> bool {
        matches!(
            input.latest_timeline_chain_state.as_deref(),
            Some("broken" | "recovering")
        ) || matches!(
            input.latest_timeline_source_event.as_deref(),
            Some("frame-await-recovery-keyframe" | "frame-inspection-rejected-await-keyframe")
        )
    }

    fn supply_is_absent(input: &VideoSchedulingOwnerInput) -> bool {
        let no_real_present = input.demand.present_age_ms.is_none();
        let no_real_decode = input.demand.decode_age_ms.is_none();
        let track_audio_only = matches!(input.latest_track_state.as_deref(), Some("audioOnly"));
        let track_has_video_bytes = input
            .latest_track_video_bytes_total
            .is_some_and(|bytes| bytes > 0);
        no_real_present && no_real_decode && track_audio_only && !track_has_video_bytes
    }

    fn should_absorb_steady_jitter(
        input: &VideoSchedulingOwnerInput,
        supply_state: DisplaySupplyState,
        has_clean_anchor_evidence: bool,
    ) -> bool {
        if input.connection_state != ConnectionLifecycleStateFact::Connected {
            return false;
        }
        if !has_clean_anchor_evidence {
            return false;
        }
        if input.demand.video_renderer_stalled {
            return false;
        }
        if matches!(supply_state, DisplaySupplyState::Critical) {
            return false;
        }
        let no_pending_streak = input.demand.no_pending_streak.unwrap_or_default();
        if no_pending_streak >= input.display_supply_thresholds.critical_no_pending_streak {
            return false;
        }
        let present_age_ms = input.demand.present_age_ms.unwrap_or(f64::INFINITY);
        let decode_age_ms = input.demand.decode_age_ms.unwrap_or(f64::INFINITY);
        let present_soft_limit = input
            .display_supply_thresholds
            .degraded_present_age_ms
            .max(1.0)
            * 2.0;
        let decode_soft_limit = input
            .display_supply_thresholds
            .degraded_decode_age_ms
            .max(1.0)
            * 2.0;
        if present_age_ms > present_soft_limit || decode_age_ms > decode_soft_limit {
            return false;
        }
        let track_attached = matches!(
            input.latest_track_state.as_deref(),
            Some("remoteTrackAttached")
        );
        let track_has_video_bytes = input
            .latest_track_video_bytes_total
            .is_some_and(|bytes| bytes > 0);
        track_attached && track_has_video_bytes
    }

    fn build_recovery_intent(
        &mut self,
        state: VideoSchedulingOwnerState,
        input: &VideoSchedulingOwnerInput,
        supply_state: DisplaySupplyState,
    ) -> Option<RecoveryIntentContract> {
        let contract = match state {
            VideoSchedulingOwnerState::RebuildingSupply => {
                let label = input
                    .anchor_reason_label
                    .clone()
                    .unwrap_or_else(|| "transportAwaitRecoveryKeyframe".to_string());
                Some(RecoveryIntentContract {
                    emit: self.should_emit_intent(
                        RecoveryIntentSource::Anchor,
                        &label,
                        input.observed_at_ms,
                    ),
                    source: RecoveryIntentSource::Anchor,
                    reason_label: label,
                })
            }
            VideoSchedulingOwnerState::SupplyStarved => {
                let label = match supply_state {
                    DisplaySupplyState::Critical => "displaySupplyCritical",
                    DisplaySupplyState::Degraded => "displaySupplyDegraded",
                    DisplaySupplyState::Healthy => return None,
                }
                .to_string();
                Some(RecoveryIntentContract {
                    emit: self.should_emit_intent(
                        RecoveryIntentSource::Supply,
                        &label,
                        input.observed_at_ms,
                    ),
                    source: RecoveryIntentSource::Supply,
                    reason_label: label,
                })
            }
            VideoSchedulingOwnerState::SeekingAnchor
            | VideoSchedulingOwnerState::Priming
            | VideoSchedulingOwnerState::StableServing
            | VideoSchedulingOwnerState::DegradedServing => {
                self.last_intent = None;
                None
            }
        };
        contract
    }

    fn should_emit_intent(
        &mut self,
        source: RecoveryIntentSource,
        label: &str,
        observed_at_ms: f64,
    ) -> bool {
        const OWNER_REPEAT_SUPPRESS_MS: f64 = 160.0;
        let should_emit = if let Some(last) = self.last_intent.as_ref() {
            last.source != source
                || last.label != label
                || (observed_at_ms - last.emitted_at_ms).max(0.0) >= OWNER_REPEAT_SUPPRESS_MS
        } else {
            true
        };
        if should_emit {
            self.last_intent = Some(RecoveryIntentCursor {
                source,
                label: label.to_string(),
                emitted_at_ms: observed_at_ms,
            });
        }
        should_emit
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RecoveryIntentSource, VideoHealthContract, VideoSchedulingOwner, VideoSchedulingOwnerInput,
        VideoSchedulingOwnerState,
    };
    use crate::transport::rtc::facts::ConnectionLifecycleStateFact;
    use crate::transport::rtc::policy::display_supply::SchedulingDemandSignal;
    use crate::transport::rtc::recovery::policy::DisplaySupplyThresholds;

    fn thresholds() -> DisplaySupplyThresholds {
        DisplaySupplyThresholds {
            degraded_no_pending_streak: 48,
            critical_no_pending_streak: 96,
            degraded_present_age_ms: 180.0,
            degraded_decode_age_ms: 140.0,
            critical_present_age_ms: 600.0,
            critical_decode_age_ms: 320.0,
            degraded_present_drop_ratio: 0.03,
            critical_present_drop_ratio: 0.08,
            degraded_present_overwrite_ratio: 0.05,
            critical_present_overwrite_ratio: 0.12,
            degraded_pacer_drop_ratio: 0.02,
            critical_pacer_drop_ratio: 0.06,
            degraded_renderer_drop_ratio: 0.015,
            critical_renderer_drop_ratio: 0.05,
        }
    }

    fn home_thresholds() -> DisplaySupplyThresholds {
        DisplaySupplyThresholds {
            degraded_no_pending_streak: 80,
            critical_no_pending_streak: 150,
            degraded_present_age_ms: 240.0,
            degraded_decode_age_ms: 180.0,
            critical_present_age_ms: 720.0,
            critical_decode_age_ms: 420.0,
            degraded_present_drop_ratio: 0.04,
            critical_present_drop_ratio: 0.10,
            degraded_present_overwrite_ratio: 0.06,
            critical_present_overwrite_ratio: 0.14,
            degraded_pacer_drop_ratio: 0.03,
            critical_pacer_drop_ratio: 0.08,
            degraded_renderer_drop_ratio: 0.02,
            critical_renderer_drop_ratio: 0.06,
        }
    }

    fn input(
        connection_state: ConnectionLifecycleStateFact,
        anchor_reason_label: Option<&str>,
        demand: SchedulingDemandSignal,
        timeline_chain_state: Option<&str>,
        timeline_source_event: Option<&str>,
        track_state: Option<&str>,
        track_video_bytes_total: Option<u64>,
        observed_at_ms: f64,
        recovery_epoch: u64,
    ) -> VideoSchedulingOwnerInput {
        VideoSchedulingOwnerInput {
            connection_state,
            recovery_epoch,
            anchor_reason_label: anchor_reason_label.map(str::to_string),
            demand,
            clean_anchor_epoch: None,
            clean_anchor_observed_at_ms: None,
            clean_anchor_source_event: None,
            latest_timeline_chain_state: timeline_chain_state.map(str::to_string),
            latest_timeline_source_event: timeline_source_event.map(str::to_string),
            latest_track_state: track_state.map(str::to_string),
            latest_track_video_bytes_total: track_video_bytes_total,
            display_supply_thresholds: thresholds(),
            observed_at_ms,
            latest_anchor_candidate_ledger: None,
        }
    }

    #[test]
    fn anchor_broken_enters_rebuilding_supply() {
        let mut owner = VideoSchedulingOwner::new();
        let output = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            Some("transportAwaitRecoveryKeyframe"),
            SchedulingDemandSignal::default(),
            None,
            None,
            None,
            None,
            100.0,
            1,
        ));
        assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
        assert_eq!(output.health, VideoHealthContract::Recovering);
        let intent = output.recovery_intent.expect("recovery intent");
        assert_eq!(intent.source, RecoveryIntentSource::Anchor);
        assert_eq!(intent.reason_label, "transportAwaitRecoveryKeyframe");
        assert!(intent.emit);
    }

    #[test]
    fn supply_starving_without_anchor_break_enters_supply_starved() {
        let mut owner = VideoSchedulingOwner::new();
        let output = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("critical".to_string()),
                no_pending_streak: Some(160),
                present_age_ms: Some(1100.0),
                decode_age_ms: Some(600.0),
                video_renderer_stalled: true,
                ..SchedulingDemandSignal::default()
            },
            None,
            None,
            None,
            None,
            200.0,
            1,
        ));
        assert_eq!(output.state, VideoSchedulingOwnerState::SupplyStarved);
        assert_eq!(output.health, VideoHealthContract::Starved);
        let intent = output.recovery_intent.expect("recovery intent");
        assert_eq!(intent.source, RecoveryIntentSource::Supply);
        assert_eq!(intent.reason_label, "displaySupplyCritical");
        assert!(intent.emit);
    }

    #[test]
    fn anchor_cleared_and_supply_healthy_returns_to_stable_serving() {
        let mut owner = VideoSchedulingOwner::new();
        let _ = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            Some("transportAwaitRecoveryKeyframe"),
            SchedulingDemandSignal::default(),
            None,
            Some("frame-await-recovery-keyframe"),
            Some("remoteTrackAttached"),
            Some(10_000),
            300.0,
            1,
        ));
        let mut ready = input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("normal".to_string()),
                no_pending_streak: Some(0),
                present_age_ms: Some(12.0),
                decode_age_ms: Some(9.0),
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            Some("healthy"),
            Some("frame-complete-candidate"),
            Some("remoteTrackAttached"),
            Some(20_000),
            520.0,
            1,
        );
        ready.clean_anchor_epoch = Some(1);
        ready.clean_anchor_observed_at_ms = Some(518.0);
        ready.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
        let output = owner.evaluate(&ready);
        assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
        assert_eq!(output.health, VideoHealthContract::Stable);
        assert!(output.recovery_intent.is_none());
    }

    #[test]
    fn sustained_critical_pressure_without_clean_anchor_keeps_owner_in_rebuilding_supply() {
        let mut owner = VideoSchedulingOwner::new();
        let _ = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            Some("transportAwaitRecoveryKeyframe"),
            SchedulingDemandSignal::default(),
            Some("recovering"),
            Some("frame-await-recovery-keyframe"),
            Some("remoteTrackAttached"),
            Some(30_000),
            520.0,
            1,
        ));

        let ready = input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("critical".to_string()),
                no_pending_streak: Some(180),
                present_age_ms: Some(12.0),
                decode_age_ms: Some(9.0),
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            Some("healthy"),
            Some("frame-complete-candidate"),
            Some("remoteTrackAttached"),
            Some(36_000),
            540.0,
            1,
        );
        let output = owner.evaluate(&ready);
        assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
        assert_eq!(output.health, VideoHealthContract::Recovering);
    }

    #[test]
    fn connected_lingering_no_pending_with_clean_anchor_can_return_to_stable_serving() {
        let mut owner = VideoSchedulingOwner::new();
        let _ = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            Some("transportAwaitRecoveryKeyframe"),
            SchedulingDemandSignal::default(),
            Some("recovering"),
            Some("frame-await-recovery-keyframe"),
            Some("remoteTrackAttached"),
            Some(30_000),
            560.0,
            1,
        ));

        let mut ready = input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("critical".to_string()),
                no_pending_streak: Some(220),
                present_age_ms: Some(14.0),
                decode_age_ms: Some(10.0),
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            Some("healthy"),
            Some("frame-observed"),
            Some("remoteTrackAttached"),
            Some(42_000),
            580.0,
            1,
        );
        ready.clean_anchor_epoch = Some(1);
        ready.clean_anchor_observed_at_ms = Some(579.0);
        ready.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());

        let output = owner.evaluate(&ready);
        assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
        assert_eq!(output.health, VideoHealthContract::Stable);
        assert!(output.recovery_intent.is_none());
    }

    #[test]
    fn clean_anchor_and_fresh_supply_can_exit_rebuilding_even_with_recovery_noise() {
        let mut owner = VideoSchedulingOwner::new();
        let _ = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            Some("transportAwaitRecoveryKeyframe"),
            SchedulingDemandSignal::default(),
            Some("recovering"),
            Some("frame-await-recovery-keyframe"),
            Some("remoteTrackAttached"),
            Some(30_000),
            520.0,
            1,
        ));

        let mut ready = input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("high".to_string()),
                no_pending_streak: Some(120),
                present_age_ms: Some(16.0),
                decode_age_ms: Some(12.0),
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            Some("healthy"),
            Some("frame-await-recovery-keyframe"),
            Some("remoteTrackAttached"),
            Some(36_000),
            540.0,
            1,
        );
        ready.clean_anchor_epoch = Some(1);
        ready.clean_anchor_observed_at_ms = Some(538.0);
        ready.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
        ready.display_supply_thresholds = home_thresholds();
        let output = owner.evaluate(&ready);
        assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
        assert_eq!(output.health, VideoHealthContract::Stable);
    }

    #[test]
    fn disconnected_or_recovering_lifecycle_constraints_owner_state() {
        let mut owner = VideoSchedulingOwner::new();
        let connected = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("normal".to_string()),
                no_pending_streak: Some(0),
                present_age_ms: Some(16.0),
                decode_age_ms: Some(8.0),
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            Some("healthy"),
            Some("frame-observed"),
            Some("remoteTrackAttached"),
            Some(10_000),
            620.0,
            1,
        ));
        assert_eq!(connected.state, VideoSchedulingOwnerState::Priming);
        let disconnected = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Closed,
            Some("transportAwaitRecoveryKeyframe"),
            SchedulingDemandSignal::default(),
            None,
            None,
            None,
            None,
            700.0,
            1,
        ));
        assert_eq!(disconnected.state, VideoSchedulingOwnerState::SeekingAnchor);
        assert_eq!(disconnected.health, VideoHealthContract::Startup);
        assert!(disconnected.recovery_intent.is_none());
    }

    #[test]
    fn epoch_change_reopens_same_intent_even_within_suppression_window() {
        let mut owner = VideoSchedulingOwner::new();
        let first = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("critical".to_string()),
                no_pending_streak: Some(220),
                present_age_ms: Some(1200.0),
                decode_age_ms: Some(640.0),
                video_renderer_stalled: true,
                ..SchedulingDemandSignal::default()
            },
            None,
            None,
            Some("audioOnly"),
            Some(0),
            800.0,
            7,
        ));
        assert!(first
            .recovery_intent
            .as_ref()
            .is_some_and(|intent| intent.emit));

        let suppressed = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("critical".to_string()),
                no_pending_streak: Some(220),
                present_age_ms: Some(1200.0),
                decode_age_ms: Some(640.0),
                video_renderer_stalled: true,
                ..SchedulingDemandSignal::default()
            },
            None,
            None,
            Some("audioOnly"),
            Some(0),
            801.0,
            7,
        ));
        assert!(suppressed
            .recovery_intent
            .as_ref()
            .is_some_and(|intent| !intent.emit));

        let reopened = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("critical".to_string()),
                no_pending_streak: Some(220),
                present_age_ms: Some(1200.0),
                decode_age_ms: Some(640.0),
                video_renderer_stalled: true,
                ..SchedulingDemandSignal::default()
            },
            None,
            None,
            Some("audioOnly"),
            Some(0),
            802.0,
            8,
        ));
        assert!(reopened
            .recovery_intent
            .as_ref()
            .is_some_and(|intent| intent.emit));
    }

    #[test]
    fn critical_no_pending_without_real_present_cannot_enter_stable_serving() {
        let mut owner = VideoSchedulingOwner::new();
        let first = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("critical".to_string()),
                no_pending_streak: Some(980),
                present_age_ms: None,
                decode_age_ms: None,
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            None,
            None,
            Some("audioOnly"),
            Some(0),
            900.0,
            1,
        ));
        assert_ne!(first.state, VideoSchedulingOwnerState::StableServing);
        assert_eq!(first.state, VideoSchedulingOwnerState::SupplyStarved);
    }

    #[test]
    fn healthy_candidate_without_supply_recovery_keeps_owner_rebuilding_supply() {
        let mut owner = VideoSchedulingOwner::new();
        let _ = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            Some("transportAwaitRecoveryKeyframe"),
            SchedulingDemandSignal::default(),
            Some("recovering"),
            Some("frame-await-recovery-keyframe"),
            Some("remoteTrackAttached"),
            Some(30_000),
            1000.0,
            1,
        ));
        let output = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("normal".to_string()),
                no_pending_streak: Some(12),
                present_age_ms: Some(220.0),
                decode_age_ms: Some(180.0),
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            Some("healthy"),
            Some("frame-complete-candidate"),
            Some("remoteTrackAttached"),
            Some(80_000),
            1015.0,
            1,
        ));
        assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
        assert_eq!(output.health, VideoHealthContract::Recovering);
    }

    #[test]
    fn rebuilding_supply_can_exit_without_clean_anchor_when_supply_is_fresh_and_connected() {
        let mut owner = VideoSchedulingOwner::new();
        let _ = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            Some("transportAwaitRecoveryKeyframe"),
            SchedulingDemandSignal::default(),
            Some("recovering"),
            Some("frame-await-recovery-keyframe"),
            Some("remoteTrackAttached"),
            Some(30_000),
            520.0,
            1,
        ));

        let mut ready = input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("normal".to_string()),
                no_pending_streak: Some(0),
                present_age_ms: Some(15.0),
                decode_age_ms: Some(17.0),
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            Some("healthy"),
            Some("frame-complete-candidate"),
            Some("remoteTrackAttached"),
            Some(42_000),
            540.0,
            1,
        );
        ready.display_supply_thresholds = home_thresholds();

        let output = owner.evaluate(&ready);
        assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
        assert_eq!(output.health, VideoHealthContract::Stable);
        assert!(output.recovery_intent.is_none());
    }

    #[test]
    fn supply_starved_can_exit_with_fresh_connected_supply_even_without_clean_anchor() {
        let mut owner = VideoSchedulingOwner::new();
        let first = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("critical".to_string()),
                no_pending_streak: Some(140),
                present_age_ms: Some(1_100.0),
                decode_age_ms: Some(640.0),
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            Some("healthy"),
            Some("frame-complete-candidate"),
            Some("remoteTrackAttached"),
            Some(96_000),
            1_000.0,
            4,
        ));
        assert_eq!(first.state, VideoSchedulingOwnerState::SupplyStarved);

        let ready = input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("normal".to_string()),
                no_pending_streak: Some(2),
                present_age_ms: Some(16.0),
                decode_age_ms: Some(12.0),
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            Some("healthy"),
            Some("frame-observed"),
            Some("remoteTrackAttached"),
            Some(120_000),
            1_020.0,
            4,
        );
        let output = owner.evaluate(&ready);
        assert_eq!(output.state, VideoSchedulingOwnerState::StableServing);
        assert_eq!(output.health, VideoHealthContract::Stable);
        assert!(output.recovery_intent.is_none());
    }

    #[test]
    fn owner_exits_recovering_only_after_strong_recovery_evidence() {
        let mut owner = VideoSchedulingOwner::new();
        let _ = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            Some("transportAwaitRecoveryKeyframe"),
            SchedulingDemandSignal::default(),
            Some("recovering"),
            Some("frame-await-recovery-keyframe"),
            Some("remoteTrackAttached"),
            Some(60_000),
            1200.0,
            1,
        ));
        let still_recovering = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("normal".to_string()),
                no_pending_streak: Some(0),
                present_age_ms: Some(260.0),
                decode_age_ms: Some(210.0),
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            Some("healthy"),
            Some("frame-complete-candidate"),
            Some("remoteTrackAttached"),
            Some(90_000),
            1210.0,
            1,
        ));
        assert_eq!(
            still_recovering.state,
            VideoSchedulingOwnerState::RebuildingSupply
        );

        let mut ready = input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("normal".to_string()),
                no_pending_streak: Some(0),
                present_age_ms: Some(20.0),
                decode_age_ms: Some(16.0),
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            Some("healthy"),
            Some("frame-observed"),
            Some("remoteTrackAttached"),
            Some(120_000),
            1230.0,
            1,
        );
        ready.clean_anchor_epoch = Some(1);
        ready.clean_anchor_observed_at_ms = Some(1225.0);
        ready.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
        let stable = owner.evaluate(&ready);
        assert_eq!(stable.state, VideoSchedulingOwnerState::StableServing);
        assert_eq!(stable.health, VideoHealthContract::Stable);
    }

    #[test]
    fn rebuilding_supply_cannot_close_to_stable_without_explicit_healthy_chain() {
        let mut owner = VideoSchedulingOwner::new();
        let _ = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            Some("transportAwaitRecoveryKeyframe"),
            SchedulingDemandSignal::default(),
            Some("recovering"),
            Some("frame-await-recovery-keyframe"),
            Some("remoteTrackAttached"),
            Some(60_000),
            1_200.0,
            1,
        ));
        let mut ready = input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("normal".to_string()),
                no_pending_streak: Some(0),
                present_age_ms: Some(20.0),
                decode_age_ms: Some(16.0),
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            None,
            Some("frame-observed"),
            Some("remoteTrackAttached"),
            Some(120_000),
            1_230.0,
            1,
        );
        ready.clean_anchor_epoch = Some(1);
        ready.clean_anchor_observed_at_ms = Some(1_225.0);
        ready.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
        let not_ready = owner.evaluate(&ready);
        assert_eq!(not_ready.state, VideoSchedulingOwnerState::RebuildingSupply);
        assert_eq!(not_ready.health, VideoHealthContract::Recovering);
        assert!(not_ready.temporary_diagnostic_summary.is_some());
    }

    #[test]
    fn rebuilding_supply_cannot_exit_with_stale_clean_anchor_fact_outside_grace_window() {
        let mut owner = VideoSchedulingOwner::new();
        let _ = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            Some("transportAwaitRecoveryKeyframe"),
            SchedulingDemandSignal::default(),
            Some("recovering"),
            Some("frame-await-recovery-keyframe"),
            Some("remoteTrackAttached"),
            Some(10_000),
            1300.0,
            3,
        ));

        let mut stale_anchor = input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("normal".to_string()),
                no_pending_streak: Some(0),
                present_age_ms: Some(16.0),
                decode_age_ms: Some(12.0),
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            Some("healthy"),
            Some("frame-observed"),
            Some("remoteTrackAttached"),
            Some(20_000),
            1310.0,
            3,
        );
        stale_anchor.clean_anchor_epoch = Some(2);
        stale_anchor.clean_anchor_observed_at_ms = Some(300.0);
        stale_anchor.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
        stale_anchor.observed_at_ms = 2_300.0;
        let not_ready = owner.evaluate(&stale_anchor);
        assert_eq!(not_ready.state, VideoSchedulingOwnerState::RebuildingSupply);
    }

    #[test]
    fn critical_wait_keyframe_noise_prefers_rebuilding_over_supply_starved_even_with_clean_anchor()
    {
        let mut owner = VideoSchedulingOwner::new();
        let mut stable = input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("normal".to_string()),
                no_pending_streak: Some(0),
                present_age_ms: Some(18.0),
                decode_age_ms: Some(12.0),
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            Some("healthy"),
            Some("frame-observed"),
            Some("remoteTrackAttached"),
            Some(90_000),
            3_000.0,
            5,
        );
        stable.clean_anchor_epoch = Some(5);
        stable.clean_anchor_observed_at_ms = Some(2_999.0);
        stable.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());
        let stable_output = owner.evaluate(&stable);
        assert_eq!(stable_output.state, VideoSchedulingOwnerState::Priming);

        let mut noisy = input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("critical".to_string()),
                no_pending_streak: Some(220),
                present_age_ms: Some(22.0),
                decode_age_ms: Some(16.0),
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            Some("healthy"),
            Some("frame-await-recovery-keyframe"),
            Some("remoteTrackAttached"),
            Some(120_000),
            3_020.0,
            5,
        );
        noisy.clean_anchor_epoch = Some(5);
        noisy.clean_anchor_observed_at_ms = Some(3_018.0);
        noisy.clean_anchor_source_event = Some("chain-clean-keyframe-submitted".to_string());

        let output = owner.evaluate(&noisy);
        assert_eq!(output.state, VideoSchedulingOwnerState::RebuildingSupply);
        assert_eq!(output.health, VideoHealthContract::Recovering);
        let intent = output.recovery_intent.expect("anchor intent");
        assert_eq!(intent.source, RecoveryIntentSource::Anchor);
        assert_eq!(intent.reason_label, "transportAwaitRecoveryKeyframe");
    }

    #[test]
    fn rebuilding_supply_cannot_close_by_clean_anchor_candidate_without_explicit_healthy_chain() {
        let mut owner = VideoSchedulingOwner::new();
        let _ = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            Some("transportAwaitRecoveryKeyframe"),
            SchedulingDemandSignal::default(),
            Some("recovering"),
            Some("frame-await-recovery-keyframe"),
            Some("remoteTrackAttached"),
            Some(10_000),
            1300.0,
            3,
        ));

        let mut ready = input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("normal".to_string()),
                no_pending_streak: Some(0),
                present_age_ms: Some(16.0),
                decode_age_ms: Some(12.0),
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            Some("broken"),
            Some("gap-reorder-pending"),
            Some("remoteTrackAttached"),
            Some(120_000),
            1310.0,
            3,
        );
        ready.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
            recovery_epoch: 3,
            frame_rtp_timestamp: Some(120_000),
            state: crate::XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
            source_event: "chain-clean-keyframe-submitted".to_string(),
            failure_reason: None,
            observed_at_ms: 1309.0,
        });

        let not_ready = owner.evaluate(&ready);
        assert_eq!(not_ready.state, VideoSchedulingOwnerState::RebuildingSupply);
        assert_eq!(not_ready.health, VideoHealthContract::Recovering);
    }

    #[test]
    fn rebuilding_supply_allows_current_clean_anchor_candidate() {
        let mut owner = VideoSchedulingOwner::new();
        let _ = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            Some("transportAwaitRecoveryKeyframe"),
            SchedulingDemandSignal::default(),
            Some("recovering"),
            Some("frame-await-recovery-keyframe"),
            Some("remoteTrackAttached"),
            Some(40_000),
            2_000.0,
            10,
        ));

        let mut ready = input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("normal".to_string()),
                no_pending_streak: Some(0),
                present_age_ms: Some(14.0),
                decode_age_ms: Some(11.0),
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            Some("healthy"),
            Some("frame-observed"),
            Some("remoteTrackAttached"),
            Some(150_000),
            2_700.0,
            10,
        );
        ready.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
            recovery_epoch: 10,
            frame_rtp_timestamp: Some(149_900),
            state: crate::XbxEngineAnchorCandidateState::SubmittedCleanAnchor,
            source_event: "chain-clean-keyframe-submitted".to_string(),
            failure_reason: None,
            observed_at_ms: 2_000.0,
        });

        let stable = owner.evaluate(&ready);
        assert_eq!(stable.state, VideoSchedulingOwnerState::StableServing);
        assert_eq!(stable.health, VideoHealthContract::Stable);
        assert!(stable.temporary_diagnostic_summary.is_none());
    }

    #[test]
    fn clean_anchor_recovery_requires_present_freshness_before_stable_serving() {
        let mut owner = VideoSchedulingOwner::new();
        let _ = owner.evaluate(&input(
            ConnectionLifecycleStateFact::Connected,
            Some("transportAwaitRecoveryKeyframe"),
            SchedulingDemandSignal::default(),
            Some("recovering"),
            Some("frame-await-recovery-keyframe"),
            Some("remoteTrackAttached"),
            Some(20_000),
            1_500.0,
            7,
        ));

        let mut missing_present = input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("normal".to_string()),
                no_pending_streak: Some(0),
                present_age_ms: None,
                decode_age_ms: Some(9.0),
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            Some("healthy"),
            Some("frame-observed"),
            Some("remoteTrackAttached"),
            Some(88_000),
            1_520.0,
            7,
        );
        missing_present.clean_anchor_epoch = Some(7);
        missing_present.clean_anchor_observed_at_ms = Some(1_518.0);
        missing_present.clean_anchor_source_event =
            Some("chain-clean-keyframe-submitted".to_string());
        let blocked = owner.evaluate(&missing_present);
        assert_eq!(blocked.state, VideoSchedulingOwnerState::RebuildingSupply);
        assert_eq!(blocked.health, VideoHealthContract::Recovering);

        let mut fresh_present = input(
            ConnectionLifecycleStateFact::Connected,
            None,
            SchedulingDemandSignal {
                no_pending_pressure_level: Some("normal".to_string()),
                no_pending_streak: Some(0),
                present_age_ms: Some(14.0),
                decode_age_ms: Some(9.0),
                video_renderer_stalled: false,
                ..SchedulingDemandSignal::default()
            },
            Some("healthy"),
            Some("frame-observed"),
            Some("remoteTrackAttached"),
            Some(90_000),
            1_540.0,
            7,
        );
        fresh_present.clean_anchor_epoch = Some(7);
        fresh_present.clean_anchor_observed_at_ms = Some(1_538.0);
        fresh_present.clean_anchor_source_event =
            Some("chain-clean-keyframe-submitted".to_string());
        let healed = owner.evaluate(&fresh_present);
        assert_eq!(healed.state, VideoSchedulingOwnerState::StableServing);
        assert_eq!(healed.health, VideoHealthContract::Stable);
    }
}
