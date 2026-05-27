use crate::transport::rtc::facts::ConnectionLifecycleStateFact;
use crate::transport::rtc::policy::display_supply::{
    DisplaySupplyCriticalSignal, DisplaySupplyState, SchedulingDemandSignal,
};
use crate::transport::rtc::recovery::contract::{
    current_clean_anchor_bridge_observed_at_ms, current_clean_anchor_observed_at_ms,
    derive_gap_severity_from_timeline_observation, frame_value_from_gap_severity,
    has_current_transport_await_issue_from_observation, is_ingress_waiting_keyframe,
    is_invalid_recovery_bootstrap_reject_reason, is_media_healthy_baseline,
    is_receiver_state_waiting_keyframe, is_transport_await_probe_source_event, FrameValue,
    GapSeverity, MediaSupplyPhase, RecoveryExitPath, DISPLAYED_IDR_SERVING_STALE_SUBMIT_BREAK_MS,
};
use crate::transport::rtc::recovery::policy::DisplaySupplyThresholds;
use crate::transport::rtc::session::control_model::SessionFaultDomain;
use crate::{
    XbxEngineAnchorCandidateLedger, XbxEngineAnchorCandidateState,
    XbxEngineVideoTimelineObservation,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VideoSchedulingOwnerState {
    SeekingAnchor,
    Priming,
    RebuildingSupply,
    StableServing,
    DegradedServing,
    SupplyStarved,
}

/// 对外四态合同（RFC / trace / diagnostics）；内部仍用 `VideoSchedulingOwnerState` 细粒度机。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VideoSchedulingOwnerContractState {
    Starting,
    Playing,
    WaitingKeyframe,
    DisplayStalled,
}

impl VideoSchedulingOwnerContractState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Playing => "playing",
            Self::WaitingKeyframe => "waitingKeyframe",
            Self::DisplayStalled => "displayStalled",
        }
    }
}

impl VideoSchedulingOwnerState {
    pub(crate) fn contract_state(
        self,
        receiver_waiting_keyframe: bool,
        host_present_stall_active: bool,
    ) -> VideoSchedulingOwnerContractState {
        if host_present_stall_active || matches!(self, Self::SupplyStarved) {
            return VideoSchedulingOwnerContractState::DisplayStalled;
        }
        if receiver_waiting_keyframe || matches!(self, Self::RebuildingSupply) {
            return VideoSchedulingOwnerContractState::WaitingKeyframe;
        }
        match self {
            Self::StableServing | Self::DegradedServing => {
                VideoSchedulingOwnerContractState::Playing
            }
            Self::SeekingAnchor | Self::Priming => VideoSchedulingOwnerContractState::Starting,
            Self::RebuildingSupply => VideoSchedulingOwnerContractState::WaitingKeyframe,
            Self::SupplyStarved => VideoSchedulingOwnerContractState::DisplayStalled,
        }
    }
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
    pub(crate) first_frame_acquisition_priority_allowed: bool,
    pub(crate) anchor_reason_label: Option<String>,
    pub(crate) demand: SchedulingDemandSignal,
    pub(crate) clean_anchor_epoch: Option<u64>,
    pub(crate) clean_anchor_observed_at_ms: Option<f64>,
    pub(crate) clean_anchor_source_event: Option<String>,
    pub(crate) clean_anchor_bridge_epoch: Option<u64>,
    pub(crate) clean_anchor_bridge_observed_at_ms: Option<f64>,
    pub(crate) clean_anchor_bridge_source_event: Option<String>,
    pub(crate) latest_anchor_candidate_ledger: Option<XbxEngineAnchorCandidateLedger>,
    /// RFC 四态；优先于 timeline.chain.state 驱动 owner / contract。
    pub(crate) receiver_state: Option<String>,
    /// gap/frame 快照；chain.state 应与 `receiver_state` 一致。
    pub(crate) latest_video_timeline_observation: Option<XbxEngineVideoTimelineObservation>,
    pub(crate) latest_timeline_chain_state: Option<String>,
    pub(crate) latest_timeline_source_event: Option<String>,
    pub(crate) latest_track_state: Option<String>,
    pub(crate) latest_track_video_bytes_total: Option<u64>,
    pub(crate) latest_h264_bootstrap_ready: Option<bool>,
    pub(crate) latest_h264_bootstrap_reject_reason: Option<String>,
    pub(crate) latest_h264_committed_sps_present: Option<bool>,
    pub(crate) latest_h264_committed_pps_present: Option<bool>,
    pub(crate) latest_h264_delta_continuation_ready: Option<bool>,
    pub(crate) latest_h264_observed_at_ms: Option<f64>,
    pub(crate) recovery_displayed_idr_at_ms: Option<f64>,
    pub(crate) recovery_fresh_anchor_recovered_at_ms: Option<f64>,
    pub(crate) recovery_exit_path: RecoveryExitPath,
    pub(crate) recovery_surface_phase:
        crate::transport::rtc::recovery::contract::RecoverySurfacePhase,
    pub(crate) derived_decoder_health:
        crate::transport::rtc::recovery::contract::DerivedDecoderHealth,
    pub(crate) displayed_idr_serving_wide: bool,
    pub(crate) contract_snapshot:
        crate::transport::rtc::recovery::contract::RecoveryContractSnapshot,
    pub(crate) display_supply_thresholds: DisplaySupplyThresholds,
    pub(crate) observed_at_ms: f64,
}

impl VideoSchedulingOwnerInput {
    /// L3 控制面：宽 serving（Insert/decode）或 relaxed（短脉冲抑制 transport-await）。
    fn displayed_idr_control_plane_active(&self) -> bool {
        self.displayed_idr_serving_wide || self.contract_snapshot.serving_relaxed
    }

    fn has_established_displayed_idr_fact(&self) -> bool {
        self.recovery_displayed_idr_at_ms.is_some()
            || self.recovery_fresh_anchor_recovered_at_ms.is_some()
    }

    fn effective_receiver_state(&self) -> Option<&str> {
        self.receiver_state
            .as_deref()
            .or_else(|| {
                self.latest_video_timeline_observation
                    .as_ref()
                    .map(|o| o.chain.state.as_str())
            })
            .or(self.latest_timeline_chain_state.as_deref())
    }

    fn effective_chain_state(&self) -> Option<&str> {
        self.effective_receiver_state()
    }

    fn effective_chain_reason(&self) -> Option<&str> {
        self.latest_video_timeline_observation
            .as_ref()
            .and_then(|o| o.chain.reason.as_deref())
    }

    fn effective_source_event(&self) -> Option<&str> {
        self.latest_video_timeline_observation
            .as_ref()
            .map(|o| o.source_event.as_str())
            .or(self.latest_timeline_source_event.as_deref())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoveryIntentContract {
    pub(crate) emit: bool,
    pub(crate) source: RecoveryIntentSource,
    pub(crate) reason: OwnerRecoveryReason,
    pub(crate) reason_label: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnerRecoveryReason {
    TransportAwaitRecoveryKeyframe,
    /// 供给重建或入口噪声：先停在怀疑层，由 session policy 门控升级。
    LocalSupplySuspect,
    DisplaySupplyCritical,
    DisplaySupplyDegraded,
    /// Host tick 持续前进但 present epoch 卡住：优先走显示域本地恢复梯子。
    HostPresentStalled,
}

impl RecoveryIntentContract {
    /// RFC：FaultDomain 只从 `session::control_model` 导出，避免与 `session::policy` 并行推导。
    #[allow(dead_code)]
    pub(crate) fn session_fault_domain(&self) -> SessionFaultDomain {
        crate::transport::rtc::session::control_model::resolve_session_fault_domain_from_owner_recovery_reason(
            self.reason,
        )
    }
}

impl OwnerRecoveryReason {
    pub(crate) fn source(self) -> RecoveryIntentSource {
        match self {
            Self::TransportAwaitRecoveryKeyframe => RecoveryIntentSource::Anchor,
            Self::LocalSupplySuspect => RecoveryIntentSource::Supply,
            Self::DisplaySupplyCritical
            | Self::DisplaySupplyDegraded
            | Self::HostPresentStalled => RecoveryIntentSource::Supply,
        }
    }

    pub(crate) fn as_reason_label(self) -> &'static str {
        match self {
            Self::TransportAwaitRecoveryKeyframe => "receiverWaitingKeyframe",
            Self::LocalSupplySuspect => "rebuildingSupplySuspect",
            Self::DisplaySupplyCritical => "displaySupplyCritical",
            Self::DisplaySupplyDegraded => "displaySupplyDegraded",
            Self::HostPresentStalled => "hostPresentStalled",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VideoSchedulingOwnerDiagnostics {
    pub(crate) reason_label: String,
    pub(crate) reason_source: VideoSchedulingOwnerContractSource,
    pub(crate) temporary_diagnostic_summary: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VideoSchedulingOwnerOutput {
    pub(crate) state: VideoSchedulingOwnerState,
    pub(crate) contract_state: VideoSchedulingOwnerContractState,
    pub(crate) health: VideoHealthContract,
    pub(crate) observed_at_ms: f64,
    pub(crate) recovery_intent: Option<RecoveryIntentContract>,
    pub(crate) diagnostics: VideoSchedulingOwnerDiagnostics,
}

#[derive(Clone, Debug)]
struct RecoveryIntentCursor {
    source: RecoveryIntentSource,
    label: String,
    emitted_at_ms: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct DisplaySupplyRecoveryGate {
    soft_critical_since_ms: Option<f64>,
    pending_supply_starved_since_ms: Option<f64>,
    pending_supply_starved_label: Option<&'static str>,
}

pub(crate) struct VideoSchedulingOwner {
    state: VideoSchedulingOwnerState,
    last_intent: Option<RecoveryIntentCursor>,
    last_recovery_epoch: Option<u64>,
    display_supply_recovery_gate: DisplaySupplyRecoveryGate,
    host_present_stall_tracker: HostPresentStallTracker,
}

/// 跟踪 display tick 前进但 present epoch 不前进的持续拍数。
#[derive(Clone, Debug, Default)]
struct HostPresentStallTracker {
    last_tick: Option<u64>,
    last_present: Option<u64>,
    tick_without_present_streak: u32,
}

impl HostPresentStallTracker {
    fn observe(&mut self, tick: u64, present: u64) {
        if let (Some(lt), Some(lp)) = (self.last_tick, self.last_present) {
            if tick > lt {
                if present > lp {
                    self.tick_without_present_streak = 0;
                } else if present == lp {
                    self.tick_without_present_streak =
                        self.tick_without_present_streak.saturating_add(1);
                } else {
                    self.tick_without_present_streak = 0;
                }
            }
        }
        self.last_tick = Some(tick);
        self.last_present = Some(present);
    }

    fn observe_display_hold(&mut self, tick: u64, present: u64) {
        self.last_tick = Some(tick);
        self.last_present = Some(present);
        self.tick_without_present_streak = 0;
    }

    fn streak(&self) -> u32 {
        self.tick_without_present_streak
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecoveryCompletionEvidence {
    NotReady,
    ServingReady,
    Ready,
}

const POST_CLEAN_ANCHOR_CONTINUATION_GRACE_MS: f64 = 300.0;
const DISPLAY_SUPPLY_SOFT_CRITICAL_CONFIRM_MS: f64 = 260.0;
/// Stable→supply-starved 确认窗：略加长以减少 healthy↔starved 阈值抖动（薄码流/调度微抖）。
const DISPLAY_SUPPLY_STARVED_CONFIRM_MS: f64 = 280.0;
const RECENT_H264_RECOVERY_BLOCKER_MAX_AGE_MS: f64 = 220.0;
/// host tick 已走而 present epoch 不长的最小连续拍数（策略拍，非固定 wall clock）。
const HOST_PRESENT_STALL_TICK_STREAK_MIN: u32 = 6;

impl VideoSchedulingOwner {
    pub(crate) fn new() -> Self {
        Self {
            state: VideoSchedulingOwnerState::SeekingAnchor,
            last_intent: None,
            last_recovery_epoch: None,
            display_supply_recovery_gate: DisplaySupplyRecoveryGate::default(),
            host_present_stall_tracker: HostPresentStallTracker::default(),
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
            self.display_supply_recovery_gate = DisplaySupplyRecoveryGate::default();
            self.host_present_stall_tracker.reset();
        }
        self.last_recovery_epoch = Some(input.recovery_epoch);
        let tick_epoch = input.demand.host_display_tick_epoch.unwrap_or(0);
        let present_epoch = input.demand.host_frame_present_epoch.unwrap_or(0);
        let host_display_hold = present_epoch > 0
            && matches!(input.demand.host_cadence_phase.as_deref(), Some("steady"))
            && input.demand.no_pending_streak.unwrap_or(0) == 0
            && input
                .demand
                .present_age_ms
                .is_some_and(|age| age <= input.display_supply_thresholds.degraded_present_age_ms);
        if host_display_hold {
            self.host_present_stall_tracker
                .observe_display_hold(tick_epoch, present_epoch);
        } else {
            self.host_present_stall_tracker
                .observe(tick_epoch, present_epoch);
        }
        let host_present_stall_streak = self.host_present_stall_tracker.streak();
        let host_present_stall_active =
            Self::host_present_stall_signal_active(input, &self.host_present_stall_tracker);
        let classified_supply_state = input
            .demand
            .classify_display_supply_state(&input.display_supply_thresholds);
        let critical_signal = input
            .demand
            .critical_signal(&input.display_supply_thresholds);
        let has_clean_anchor_evidence = Self::has_current_clean_anchor_release_evidence(input);
        let displayed_idr_serving_release_active =
            Self::displayed_idr_serving_release_active(input, host_present_stall_streak);
        let chain_healthy = matches!(input.effective_chain_state(), Some("receiving"))
            || (has_clean_anchor_evidence
                && matches!(input.effective_chain_state(), Some("repairing")))
            || Self::pipeline_stressed_repairing_chain_serviceable(
                input,
                has_clean_anchor_evidence,
            )
            || displayed_idr_serving_release_active;
        let ingress_waiting_keyframe = is_ingress_waiting_keyframe(
            input.effective_receiver_state(),
            input.effective_chain_state(),
            input.effective_chain_reason(),
            input.effective_source_event(),
        );
        let effective_supply_state = self.resolve_effective_supply_state(
            input,
            classified_supply_state,
            critical_signal,
            has_clean_anchor_evidence,
            chain_healthy,
        );
        let absorb_soft_display_supply_critical = matches!(
            critical_signal,
            DisplaySupplyCriticalSignal::SoftNoPendingAge
        ) && effective_supply_state
            != classified_supply_state;
        let clean_anchor_hysteresis = Self::clean_anchor_hysteresis_allows_reentry(input);
        let wait_keyframe_rebuild_priority = Self::should_prioritize_wait_keyframe_rebuild(input);
        let first_frame_acquisition_priority_active =
            Self::first_frame_acquisition_priority_active(current_state, input);
        let transport_await_local_probe_probation_active =
            Self::transport_await_local_probe_probation_active(
                current_state,
                input,
                has_clean_anchor_evidence,
                chain_healthy,
            );
        let has_anchor_issue = if first_frame_acquisition_priority_active {
            false
        } else if displayed_idr_serving_release_active {
            false
        } else if transport_await_local_probe_probation_active {
            false
        } else if wait_keyframe_rebuild_priority {
            true
        } else if Self::should_reenter_anchor_recovery_after_clean_anchor(current_state, input) {
            true
        } else if Self::transient_anchor_noise_can_settle(input, has_clean_anchor_evidence) {
            false
        } else if clean_anchor_hysteresis {
            false
        } else if Self::can_release_rebuild_after_terminal_invalid_bootstrap(
            input,
            effective_supply_state,
            has_clean_anchor_evidence,
        ) {
            false
        } else if has_clean_anchor_evidence && chain_healthy {
            // clean anchor 已经成立且链路 healthy 时，旧的 anchor reason 不应继续把 owner
            // 锁在 rebuilding-supply，否则 Home 场景会永远收不到 stable-serving。
            false
        } else if Self::display_pipeline_stressed(input) {
            // 显示管道瓶颈时仍保留 repairing / transport-await 的真实锚点诉求；
            // 仅抑制 receiving 态下挂着旧 anchor 标签的误重建。
            if ingress_waiting_keyframe
                || matches!(
                    input.effective_chain_state(),
                    Some("repairing" | "waiting-keyframe")
                )
                || matches!(
                    input.effective_chain_reason(),
                    Some("gapRepairInFlight" | "receiverWaitingKeyframe")
                )
            {
                Self::timeline_indicates_anchor_issue(input) || input.anchor_reason_label.is_some()
            } else {
                false
            }
        } else {
            Self::timeline_indicates_anchor_issue(input) || input.anchor_reason_label.is_some()
        };
        let has_anchor_issue = if matches!(
            input.recovery_surface_phase,
            crate::transport::rtc::recovery::contract::RecoverySurfacePhase::SupplyBreak
        ) {
            // L0 supply-break：禁止再叠 transport-await / rebuilding-supply 平行叙事。
            false
        } else {
            has_anchor_issue
        };
        let anchor_rebuild =
            Self::effective_anchor_rebuild(has_anchor_issue, input, ingress_waiting_keyframe);
        let completion_evidence = Self::resolve_recovery_completion_evidence(
            self.state,
            input,
            effective_supply_state,
            classified_supply_state,
            has_anchor_issue,
            ingress_waiting_keyframe,
            host_present_stall_streak,
        );
        let supply_absent = Self::supply_is_absent(input);
        let hold_supply_starved_transition = self.should_hold_supply_starved_transition(
            input,
            effective_supply_state,
            has_anchor_issue,
            completion_evidence,
        );
        let next = self.transition(
            self.state,
            input,
            input.connection_state,
            anchor_rebuild,
            effective_supply_state,
            completion_evidence,
            supply_absent,
            has_clean_anchor_evidence,
            absorb_soft_display_supply_critical,
            hold_supply_starved_transition,
            host_present_stall_active,
        );
        let next = if matches!(
            input.recovery_surface_phase,
            crate::transport::rtc::recovery::contract::RecoverySurfacePhase::SupplyBreak
        ) && !matches!(
            next,
            VideoSchedulingOwnerState::SeekingAnchor | VideoSchedulingOwnerState::Priming
        ) {
            VideoSchedulingOwnerState::SupplyStarved
        } else {
            next
        };
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

        let recovery_intent = self.build_recovery_intent(
            next,
            input,
            effective_supply_state,
            completion_evidence,
            ingress_waiting_keyframe,
            host_present_stall_active,
            host_present_stall_streak,
        );
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
                VideoSchedulingOwnerState::SupplyStarved => {
                    if host_present_stall_active {
                        "hostPresentStalled"
                    } else {
                        "supplyStarved"
                    }
                }
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
            effective_supply_state,
            has_clean_anchor_evidence,
            has_anchor_issue,
            completion_evidence,
            &reason_label,
            reason_source,
        );
        VideoSchedulingOwnerOutput {
            state: next,
            contract_state: next
                .contract_state(ingress_waiting_keyframe, host_present_stall_active),
            health,
            observed_at_ms: input.observed_at_ms,
            recovery_intent,
            diagnostics: VideoSchedulingOwnerDiagnostics {
                reason_label,
                reason_source,
                temporary_diagnostic_summary,
            },
        }
    }

    fn resolve_effective_supply_state(
        &mut self,
        input: &VideoSchedulingOwnerInput,
        supply_state: DisplaySupplyState,
        critical_signal: DisplaySupplyCriticalSignal,
        has_clean_anchor_evidence: bool,
        chain_healthy: bool,
    ) -> DisplaySupplyState {
        if critical_signal != DisplaySupplyCriticalSignal::SoftNoPendingAge {
            self.display_supply_recovery_gate.soft_critical_since_ms = None;
            return supply_state;
        }
        if !self.should_gate_soft_display_supply_critical(
            input,
            has_clean_anchor_evidence,
            chain_healthy,
        ) {
            self.display_supply_recovery_gate.soft_critical_since_ms = None;
            return supply_state;
        }
        let critical_since = self
            .display_supply_recovery_gate
            .soft_critical_since_ms
            .get_or_insert(input.observed_at_ms);
        let no_pending_streak = input.demand.no_pending_streak.unwrap_or_default();
        let streak_confirms = no_pending_streak
            >= input
                .display_supply_thresholds
                .critical_no_pending_streak
                .saturating_mul(2);
        let time_confirms = (input.observed_at_ms - *critical_since).max(0.0)
            >= DISPLAY_SUPPLY_SOFT_CRITICAL_CONFIRM_MS;
        if streak_confirms || time_confirms {
            DisplaySupplyState::Critical
        } else {
            DisplaySupplyState::Degraded
        }
    }

    fn display_pipeline_stressed(input: &VideoSchedulingOwnerInput) -> bool {
        input
            .demand
            .present_pipeline_stressed(&input.display_supply_thresholds)
    }

    fn effective_anchor_rebuild(
        has_anchor_issue: bool,
        input: &VideoSchedulingOwnerInput,
        ingress_waiting_keyframe: bool,
    ) -> bool {
        if !has_anchor_issue {
            return false;
        }
        if ingress_waiting_keyframe {
            return true;
        }
        !Self::display_pipeline_stressed(input)
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
        absorb_soft_display_supply_critical: bool,
        hold_supply_starved_transition: bool,
        host_present_stall_active: bool,
    ) -> VideoSchedulingOwnerState {
        if !matches!(
            connection_state,
            ConnectionLifecycleStateFact::Connected | ConnectionLifecycleStateFact::Recovering
        ) {
            return VideoSchedulingOwnerState::SeekingAnchor;
        }
        let first_present_grace_active = Self::first_present_grace_active(input);
        if let Some(phase_state) = Self::owner_state_for_media_supply_phase(
            input.contract_snapshot.media_supply_phase,
            current,
            has_anchor_issue,
        ) {
            return phase_state;
        }
        match current {
            VideoSchedulingOwnerState::SeekingAnchor => {
                if has_anchor_issue {
                    VideoSchedulingOwnerState::RebuildingSupply
                } else if completion_evidence == RecoveryCompletionEvidence::Ready {
                    VideoSchedulingOwnerState::Priming
                } else if first_present_grace_active {
                    VideoSchedulingOwnerState::Priming
                } else if matches!(supply_state, DisplaySupplyState::Healthy) && !supply_absent {
                    VideoSchedulingOwnerState::Priming
                } else if Self::media_supply_phase_requires_supply_starved(input)
                    || Self::segmented_supply_starved_evidence(input)
                    || matches!(supply_state, DisplaySupplyState::Critical)
                    || matches!(
                        input.demand.no_pending_pressure_level.as_deref(),
                        Some("critical")
                    )
                {
                    VideoSchedulingOwnerState::SupplyStarved
                } else {
                    VideoSchedulingOwnerState::Priming
                }
            }
            VideoSchedulingOwnerState::Priming => {
                if has_anchor_issue {
                    VideoSchedulingOwnerState::RebuildingSupply
                } else if completion_evidence == RecoveryCompletionEvidence::Ready {
                    VideoSchedulingOwnerState::StableServing
                } else if first_present_grace_active {
                    VideoSchedulingOwnerState::Priming
                } else if Self::media_supply_phase_requires_supply_starved(input)
                    || Self::segmented_supply_starved_evidence(input)
                {
                    VideoSchedulingOwnerState::SupplyStarved
                } else {
                    VideoSchedulingOwnerState::DegradedServing
                }
            }
            VideoSchedulingOwnerState::RebuildingSupply => {
                if completion_evidence == RecoveryCompletionEvidence::Ready {
                    VideoSchedulingOwnerState::StableServing
                } else if completion_evidence == RecoveryCompletionEvidence::ServingReady {
                    VideoSchedulingOwnerState::DegradedServing
                } else if host_present_stall_active {
                    VideoSchedulingOwnerState::SupplyStarved
                } else if has_anchor_issue {
                    VideoSchedulingOwnerState::RebuildingSupply
                } else if Self::display_pipeline_stressed(input) {
                    VideoSchedulingOwnerState::DegradedServing
                } else {
                    VideoSchedulingOwnerState::RebuildingSupply
                }
            }
            VideoSchedulingOwnerState::StableServing => {
                if has_anchor_issue {
                    VideoSchedulingOwnerState::RebuildingSupply
                } else if host_present_stall_active {
                    VideoSchedulingOwnerState::SupplyStarved
                } else if completion_evidence == RecoveryCompletionEvidence::Ready {
                    VideoSchedulingOwnerState::StableServing
                } else if absorb_soft_display_supply_critical {
                    VideoSchedulingOwnerState::DegradedServing
                } else if Self::should_absorb_steady_jitter(
                    input,
                    supply_state,
                    has_clean_anchor_evidence,
                ) {
                    VideoSchedulingOwnerState::DegradedServing
                } else if hold_supply_starved_transition {
                    VideoSchedulingOwnerState::DegradedServing
                } else if Self::transient_serving_pipeline_healthy(input) {
                    VideoSchedulingOwnerState::DegradedServing
                } else {
                    VideoSchedulingOwnerState::SupplyStarved
                }
            }
            VideoSchedulingOwnerState::DegradedServing => {
                if has_anchor_issue {
                    VideoSchedulingOwnerState::RebuildingSupply
                } else if host_present_stall_active {
                    VideoSchedulingOwnerState::SupplyStarved
                } else if completion_evidence == RecoveryCompletionEvidence::Ready {
                    VideoSchedulingOwnerState::StableServing
                } else if completion_evidence == RecoveryCompletionEvidence::ServingReady {
                    VideoSchedulingOwnerState::DegradedServing
                } else if absorb_soft_display_supply_critical {
                    VideoSchedulingOwnerState::DegradedServing
                } else if Self::should_absorb_steady_jitter(
                    input,
                    supply_state,
                    has_clean_anchor_evidence,
                ) {
                    VideoSchedulingOwnerState::DegradedServing
                } else if hold_supply_starved_transition {
                    VideoSchedulingOwnerState::DegradedServing
                } else if Self::transient_serving_pipeline_healthy(input)
                    || Self::display_pipeline_stressed_serving_absorb(input)
                {
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
                } else if completion_evidence == RecoveryCompletionEvidence::ServingReady
                    || Self::display_pipeline_stressed_serving_absorb(input)
                {
                    VideoSchedulingOwnerState::DegradedServing
                } else {
                    VideoSchedulingOwnerState::SupplyStarved
                }
            }
        }
    }

    /// decode 新鲜、present 尚可但 FPS 落差大：显示域瓶颈，应吸收为 degraded 而非 starved。
    fn display_pipeline_stressed_serving_absorb(input: &VideoSchedulingOwnerInput) -> bool {
        if !Self::display_pipeline_stressed(input) {
            return false;
        }
        if input.connection_state != ConnectionLifecycleStateFact::Connected {
            return false;
        }
        let decode_fresh = input
            .demand
            .decode_age_ms
            .is_some_and(|age| age <= input.display_supply_thresholds.degraded_decode_age_ms);
        let present_acceptable = input
            .demand
            .present_age_ms
            .is_some_and(|age| age <= input.display_supply_thresholds.critical_present_age_ms);
        let track_attached = matches!(
            input.latest_track_state.as_deref(),
            Some("remoteTrackAttached")
        );
        let track_has_video_bytes = input
            .latest_track_video_bytes_total
            .is_some_and(|bytes| bytes > 0);
        decode_fresh && present_acceptable && track_attached && track_has_video_bytes
    }

    fn pipeline_stressed_repairing_chain_serviceable(
        input: &VideoSchedulingOwnerInput,
        has_clean_anchor_evidence: bool,
    ) -> bool {
        if !has_clean_anchor_evidence || !Self::display_pipeline_stressed(input) {
            return false;
        }
        if !matches!(
            input.effective_chain_state(),
            Some("repairing" | "receiving")
        ) {
            return false;
        }
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
        decode_fresh && track_attached && track_has_video_bytes
    }

    /// decode 新鲜但 present 吞吐偏低时，host tick 会快于 present epoch，不应判 host present stall。
    fn chronic_low_present_throughput(input: &VideoSchedulingOwnerInput) -> bool {
        input
            .demand
            .present_pipeline_stressed(&input.display_supply_thresholds)
    }

    fn host_present_stall_signal_active(
        input: &VideoSchedulingOwnerInput,
        stall_tracker: &HostPresentStallTracker,
    ) -> bool {
        if input.connection_state != ConnectionLifecycleStateFact::Connected {
            return false;
        }
        if input.demand.host_is_priming_without_present() {
            return false;
        }
        if Self::chronic_low_present_throughput(input) {
            return false;
        }
        let tick = input.demand.host_display_tick_epoch.unwrap_or(0);
        if tick == 0 {
            return false;
        }
        stall_tracker.streak() >= HOST_PRESENT_STALL_TICK_STREAK_MIN
            && Self::host_present_stall_pipeline_supply_active(input, stall_tracker)
    }

    fn host_present_stall_pipeline_supply_active(
        input: &VideoSchedulingOwnerInput,
        stall_tracker: &HostPresentStallTracker,
    ) -> bool {
        let track_ok = matches!(
            input.latest_track_state.as_deref(),
            Some("remoteTrackAttached")
        ) && input
            .latest_track_video_bytes_total
            .is_some_and(|bytes| bytes > 0);
        if !track_ok {
            return false;
        }
        // display tick 前进而 present epoch 停滞：不等 decode 仍新鲜（冻屏时 decode 往往已停）。
        if stall_tracker.streak() >= HOST_PRESENT_STALL_TICK_STREAK_MIN {
            return true;
        }
        input
            .demand
            .decode_age_ms
            .is_some_and(|age| age <= input.display_supply_thresholds.degraded_decode_age_ms * 1.5)
    }

    fn resolve_recovery_completion_evidence(
        current: VideoSchedulingOwnerState,
        input: &VideoSchedulingOwnerInput,
        effective_supply_state: DisplaySupplyState,
        classified_supply_state: DisplaySupplyState,
        has_anchor_issue: bool,
        ingress_waiting_keyframe: bool,
        host_present_stall_streak: u32,
    ) -> RecoveryCompletionEvidence {
        let has_clean_anchor_evidence = Self::has_current_clean_anchor_release_evidence(input);
        let transient_anchor_noise_settled =
            Self::transient_anchor_noise_can_settle(input, has_clean_anchor_evidence);
        let has_stale_clean_anchor_fact = input.clean_anchor_epoch.is_some_and(|epoch| {
            epoch != input.recovery_epoch
                && input.clean_anchor_source_event.as_deref()
                    == Some("chain-clean-anchor-submitted")
        }) || input
            .latest_anchor_candidate_ledger
            .as_ref()
            .is_some_and(|candidate| {
                candidate.state == XbxEngineAnchorCandidateState::SubmittedCleanAnchor
                    && candidate.source_event == "chain-clean-anchor-submitted"
                    && candidate.recovery_epoch != input.recovery_epoch
            });
        let chain_healthy = matches!(input.effective_chain_state(), Some("receiving"))
            || transient_anchor_noise_settled
            || Self::pipeline_stressed_repairing_chain_serviceable(
                input,
                has_clean_anchor_evidence,
            );
        let media_healthy_baseline = is_media_healthy_baseline(
            input.connection_state == ConnectionLifecycleStateFact::Connected,
            chain_healthy,
            input.latest_track_state.as_deref(),
            input.latest_track_video_bytes_total,
            input.demand.decode_age_ms,
            input.demand.present_age_ms,
            input.display_supply_thresholds.degraded_decode_age_ms,
            input.display_supply_thresholds.degraded_present_age_ms,
            false,
            Self::renderer_shadow_blocks_recovery_release(input),
        );
        let transport_await_waiting_released = ingress_waiting_keyframe
            && Self::transport_await_waiting_released_by_facts(input, effective_supply_state);
        let terminal_invalid_bootstrap_serving_ready = current
            == VideoSchedulingOwnerState::RebuildingSupply
            && Self::can_release_rebuild_after_terminal_invalid_bootstrap(
                input,
                effective_supply_state,
                has_clean_anchor_evidence,
            );
        if ingress_waiting_keyframe
            && !transport_await_waiting_released
            && !terminal_invalid_bootstrap_serving_ready
            && !Self::displayed_idr_serving_release_active(input, host_present_stall_streak)
        {
            match input.recovery_exit_path {
                RecoveryExitPath::DecodeOutput | RecoveryExitPath::TimedFallback => {
                    if matches!(
                        effective_supply_state,
                        DisplaySupplyState::Healthy | DisplaySupplyState::Degraded
                    ) {
                        return RecoveryCompletionEvidence::ServingReady;
                    }
                    if input.recovery_exit_path == RecoveryExitPath::TimedFallback {
                        return RecoveryCompletionEvidence::ServingReady;
                    }
                }
                _ => {}
            }
            return RecoveryCompletionEvidence::NotReady;
        }
        if Self::displayed_idr_serving_release_active(input, host_present_stall_streak) {
            if matches!(classified_supply_state, DisplaySupplyState::Healthy) {
                return RecoveryCompletionEvidence::Ready;
            }
            if matches!(
                effective_supply_state,
                DisplaySupplyState::Healthy | DisplaySupplyState::Degraded
            ) {
                return RecoveryCompletionEvidence::ServingReady;
            }
        }
        if current == VideoSchedulingOwnerState::RebuildingSupply {
            if has_stale_clean_anchor_fact
                && !has_clean_anchor_evidence
                && !terminal_invalid_bootstrap_serving_ready
            {
                return RecoveryCompletionEvidence::NotReady;
            }
            if !has_clean_anchor_evidence
                && !terminal_invalid_bootstrap_serving_ready
                && !Self::supply_recovery_can_settle_without_explicit_clean_anchor(
                    input,
                    classified_supply_state,
                    chain_healthy,
                )
            {
                return RecoveryCompletionEvidence::NotReady;
            }
        }
        if current == VideoSchedulingOwnerState::SupplyStarved
            && Self::supply_recovery_can_settle_without_explicit_clean_anchor(
                input,
                classified_supply_state,
                chain_healthy,
            )
        {
            return RecoveryCompletionEvidence::Ready;
        }
        if terminal_invalid_bootstrap_serving_ready {
            // 仅 broken 链走 terminal-invalid 的 ServingReady；repairing/healthy 等交给后续 Ready
            // 判定（见 transient_anchor_noise / non_idr 与 degraded_supply_still_releases）。
            if matches!(input.effective_chain_state(), Some("waiting-keyframe")) {
                return RecoveryCompletionEvidence::ServingReady;
            }
        }
        if matches!(
            current,
            VideoSchedulingOwnerState::RebuildingSupply | VideoSchedulingOwnerState::SupplyStarved
        ) && !has_anchor_issue
            && Self::displayed_idr_serving_release_active(input, host_present_stall_streak)
        {
            return RecoveryCompletionEvidence::ServingReady;
        }
        if has_anchor_issue || !matches!(classified_supply_state, DisplaySupplyState::Healthy) {
            if matches!(
                current,
                VideoSchedulingOwnerState::RebuildingSupply
                    | VideoSchedulingOwnerState::SupplyStarved
            ) && Self::can_restore_serving_after_clean_anchor(
                input,
                has_clean_anchor_evidence,
                chain_healthy,
                classified_supply_state,
            ) {
                return RecoveryCompletionEvidence::ServingReady;
            }
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
        let can_bridge_present_feedback_gap = current
            == VideoSchedulingOwnerState::RebuildingSupply
            && Self::can_close_recovery_with_transient_present_feedback_gap(
                input,
                classified_supply_state,
                has_clean_anchor_evidence,
                chain_healthy,
            );
        // 恢复到 stable-serving 需要比“可播放”更稳：单点 freshness 不足以说明供给已真正降压。
        if (!present_fresh || !decode_fresh) && !can_bridge_present_feedback_gap {
            if matches!(
                current,
                VideoSchedulingOwnerState::RebuildingSupply
                    | VideoSchedulingOwnerState::SupplyStarved
            ) && Self::can_restore_serving_after_clean_anchor(
                input,
                has_clean_anchor_evidence,
                chain_healthy,
                classified_supply_state,
            ) {
                return RecoveryCompletionEvidence::ServingReady;
            }
            return RecoveryCompletionEvidence::NotReady;
        }
        if !Self::supply_pressure_is_settled(input, has_clean_anchor_evidence, chain_healthy) {
            if matches!(
                current,
                VideoSchedulingOwnerState::RebuildingSupply
                    | VideoSchedulingOwnerState::SupplyStarved
            ) && Self::can_restore_serving_after_clean_anchor(
                input,
                has_clean_anchor_evidence,
                chain_healthy,
                classified_supply_state,
            ) {
                return RecoveryCompletionEvidence::ServingReady;
            }
            return RecoveryCompletionEvidence::NotReady;
        }
        if !media_healthy_baseline && !transport_await_waiting_released {
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
        if !chain_healthy && !transport_await_waiting_released {
            return RecoveryCompletionEvidence::NotReady;
        }
        let recovery_noise_source = matches!(
            input.effective_source_event(),
            Some("frame-await-recovery-anchor")
                | Some("frame-inspection-rejected-await-anchor")
                | Some("frame-inspection-rejected-trigger-recovery-anchor")
                | Some("gap-repair-in-flight")
        );
        if recovery_noise_source
            && !(has_clean_anchor_evidence && (chain_healthy || transport_await_waiting_released))
        {
            return RecoveryCompletionEvidence::NotReady;
        }
        RecoveryCompletionEvidence::Ready
    }

    fn transport_await_waiting_released_by_facts(
        input: &VideoSchedulingOwnerInput,
        supply_state: DisplaySupplyState,
    ) -> bool {
        let terminal_invalid_bootstrap_release_ready =
            Self::has_fresh_terminal_invalid_bootstrap_release_evidence(input);
        if input.connection_state != ConnectionLifecycleStateFact::Connected {
            return false;
        }
        if !Self::has_current_clean_anchor_release_evidence(input)
            && !terminal_invalid_bootstrap_release_ready
        {
            return false;
        }
        if Self::renderer_shadow_blocks_recovery_release(input) {
            return false;
        }
        let Some(timeline) = input.latest_video_timeline_observation.as_ref() else {
            return false;
        };
        if terminal_invalid_bootstrap_release_ready {
            if matches!(supply_state, DisplaySupplyState::Critical) {
                return false;
            }
        } else if !matches!(supply_state, DisplaySupplyState::Healthy) {
            return false;
        }
        if !terminal_invalid_bootstrap_release_ready
            && Self::has_transport_await_hard_rebuild_evidence(input)
        {
            return false;
        }
        let gap_severity = derive_gap_severity_from_timeline_observation(timeline);
        let frame_value = frame_value_from_gap_severity(gap_severity);
        if !terminal_invalid_bootstrap_release_ready
            && !matches!(frame_value, Some(FrameValue::RecoveryAnchor))
        {
            return false;
        }
        if !terminal_invalid_bootstrap_release_ready
            && !is_transport_await_probe_source_event(Some(timeline.source_event.as_str()))
            && !has_current_transport_await_issue_from_observation(
                timeline,
                Self::current_release_anchor_observed_at_ms(input),
            )
        {
            return false;
        }
        let decode_fresh = input
            .demand
            .decode_age_ms
            .is_some_and(|age| age <= input.display_supply_thresholds.degraded_decode_age_ms);
        let present_fresh = input
            .demand
            .present_age_ms
            .is_some_and(|age| age <= input.display_supply_thresholds.degraded_present_age_ms)
            || Self::first_present_feedback_gap_active(input);
        let track_attached = matches!(
            input.latest_track_state.as_deref(),
            Some("remoteTrackAttached")
        );
        let track_has_video_bytes = input
            .latest_track_video_bytes_total
            .is_some_and(|bytes| bytes > 0);
        decode_fresh && present_fresh && track_attached && track_has_video_bytes
    }

    fn terminal_invalid_bootstrap_has_serviceable_output(
        input: &VideoSchedulingOwnerInput,
        supply_state: DisplaySupplyState,
    ) -> bool {
        if matches!(supply_state, DisplaySupplyState::Critical) {
            return false;
        }
        let decode_fresh = input
            .demand
            .decode_age_ms
            .is_some_and(|age| age <= input.display_supply_thresholds.degraded_decode_age_ms);
        let present_fresh = input
            .demand
            .present_age_ms
            .is_some_and(|age| age <= input.display_supply_thresholds.degraded_present_age_ms)
            || Self::first_present_feedback_gap_active(input);
        let track_attached = matches!(
            input.latest_track_state.as_deref(),
            Some("remoteTrackAttached")
        );
        let track_has_video_bytes = input
            .latest_track_video_bytes_total
            .is_some_and(|bytes| bytes > 0);
        decode_fresh && present_fresh && track_attached && track_has_video_bytes
    }

    fn has_fresh_terminal_invalid_bootstrap_release_evidence(
        input: &VideoSchedulingOwnerInput,
    ) -> bool {
        if input.latest_h264_bootstrap_ready != Some(false)
            || !is_invalid_recovery_bootstrap_reject_reason(
                input.latest_h264_bootstrap_reject_reason.as_deref(),
            )
        {
            return false;
        }
        let metadata_ready = input.latest_h264_committed_sps_present == Some(true)
            && input.latest_h264_committed_pps_present == Some(true)
            && input.latest_h264_delta_continuation_ready == Some(true);
        if !metadata_ready {
            return false;
        }
        input
            .latest_h264_observed_at_ms
            .is_some_and(|observed_at_ms| {
                (input.observed_at_ms - observed_at_ms).max(0.0)
                    <= RECENT_H264_RECOVERY_BLOCKER_MAX_AGE_MS
            })
    }

    fn first_present_grace_active(input: &VideoSchedulingOwnerInput) -> bool {
        if !input.demand.host_is_priming_without_present() {
            return false;
        }
        input.demand.host_display_tick_epoch.unwrap_or_default() > 0
    }

    /// L3 表驱动：Owner 状态为 L0 `media_supply_phase` 的投影，禁止平行 completion 叙事。
    fn owner_state_for_media_supply_phase(
        phase: MediaSupplyPhase,
        current: VideoSchedulingOwnerState,
        has_anchor_issue: bool,
    ) -> Option<VideoSchedulingOwnerState> {
        match phase {
            MediaSupplyPhase::Priming => Some(if has_anchor_issue {
                VideoSchedulingOwnerState::RebuildingSupply
            } else {
                VideoSchedulingOwnerState::Priming
            }),
            MediaSupplyPhase::SupplyBreak => Some(VideoSchedulingOwnerState::SupplyStarved),
            _ => None,
        }
    }

    fn media_supply_phase_requires_supply_starved(input: &VideoSchedulingOwnerInput) -> bool {
        matches!(
            input.contract_snapshot.media_supply_phase,
            MediaSupplyPhase::SupplyBreak | MediaSupplyPhase::MustIdr
        )
    }

    fn segmented_supply_starved_evidence(input: &VideoSchedulingOwnerInput) -> bool {
        let thresholds = &input.display_supply_thresholds;
        let decode_bad = input
            .demand
            .decode_age_ms
            .is_some_and(|age| age >= thresholds.critical_decode_age_ms);
        let submit_bad = input
            .demand
            .submit_age_ms
            .is_some_and(|age| age >= thresholds.critical_decode_age_ms);
        let present_bad = input
            .demand
            .present_age_ms
            .is_some_and(|age| age >= thresholds.critical_present_age_ms);
        (decode_bad || submit_bad) && present_bad
    }

    fn first_present_feedback_gap_active(input: &VideoSchedulingOwnerInput) -> bool {
        input.demand.host_display_tick_epoch.unwrap_or_default() > 0
            && input.demand.host_frame_present_epoch.unwrap_or_default() == 0
            && input
                .demand
                .host_mailbox_enqueue_count_total
                .unwrap_or_default()
                == 0
    }

    fn host_presentation_serviceable(input: &VideoSchedulingOwnerInput) -> bool {
        let pressure_high = matches!(
            input.demand.no_pending_pressure_level.as_deref(),
            Some("high" | "critical")
        ) && input.demand.no_pending_streak.unwrap_or_default()
            >= input.display_supply_thresholds.degraded_no_pending_streak;
        if pressure_high {
            return false;
        }
        input
            .demand
            .present_age_ms
            .is_some_and(|age| age <= input.display_supply_thresholds.degraded_present_age_ms)
            || Self::first_present_feedback_gap_active(input)
    }

    fn renderer_shadow_blocks_recovery_release(input: &VideoSchedulingOwnerInput) -> bool {
        input.demand.video_renderer_stalled && !Self::host_presentation_serviceable(input)
    }

    fn can_release_rebuild_after_terminal_invalid_bootstrap(
        input: &VideoSchedulingOwnerInput,
        supply_state: DisplaySupplyState,
        has_clean_anchor_evidence: bool,
    ) -> bool {
        if input.connection_state != ConnectionLifecycleStateFact::Connected {
            return false;
        }
        if matches!(supply_state, DisplaySupplyState::Critical) {
            return false;
        }
        if Self::renderer_shadow_blocks_recovery_release(input) {
            return false;
        }
        if Self::has_rejected_transport_await_anchor_candidate(input) {
            return false;
        }
        if !Self::has_fresh_terminal_invalid_bootstrap_release_evidence(input) {
            return false;
        }
        if has_clean_anchor_evidence {
            // broken 链 + 终端 deferred invalid 时，display 可能暂时不满足 degraded fresh，
            // 但仍需退出 rebuilding-supply 进入 degraded-serving 吸收尾风险（见单测
            // degraded_supply_still_releases_terminal_invalid_bootstrap_waiting）。
            if matches!(input.effective_chain_state(), Some("waiting-keyframe")) {
                return true;
            }
            return Self::terminal_invalid_bootstrap_has_serviceable_output(input, supply_state);
        }
        // 没有 clean anchor 时，只有在当前输出仍可服务且本次 bootstrap 明确不可用时，
        // 才允许退出 rebuilding-supply，避免把“等不到 clean anchor”的坏链锁死成永久恢复态。
        Self::terminal_invalid_bootstrap_has_serviceable_output(input, supply_state)
    }

    fn first_frame_acquisition_priority_active(
        current: VideoSchedulingOwnerState,
        input: &VideoSchedulingOwnerInput,
    ) -> bool {
        if !matches!(
            current,
            VideoSchedulingOwnerState::SeekingAnchor | VideoSchedulingOwnerState::Priming
        ) {
            return false;
        }
        if !input.first_frame_acquisition_priority_allowed {
            return false;
        }
        if input.connection_state != ConnectionLifecycleStateFact::Connected {
            return false;
        }
        if !Self::first_present_grace_active(input) {
            return false;
        }
        let bootstrap_reject_in_startup = input.latest_h264_bootstrap_ready == Some(false)
            && input.latest_h264_bootstrap_reject_reason.is_some();
        let bootstrap_reject_source_pending = matches!(
            input.effective_source_event(),
            Some(
                "frame-inspection-rejected-await-anchor"
                    | "frame-inspection-rejected-trigger-recovery-anchor"
                    | "frame-await-recovery-anchor"
            )
        );
        if !bootstrap_reject_in_startup && !bootstrap_reject_source_pending {
            return false;
        }
        let track_attached = matches!(
            input.latest_track_state.as_deref(),
            Some("remoteTrackAttached")
        );
        let track_has_video_bytes = input
            .latest_track_video_bytes_total
            .is_some_and(|bytes| bytes > 0);
        if !track_attached || !track_has_video_bytes {
            return false;
        }
        // 除显式 wait-keyframe 外，ingress 等关键帧 / gap 修复噪声也应保持首帧前保护，避免时间线先滑到 gap-* 即失效。
        is_ingress_waiting_keyframe(
            input.effective_receiver_state(),
            input.effective_chain_state(),
            input.effective_chain_reason(),
            input.effective_source_event(),
        ) || matches!(
            input.effective_source_event(),
            Some(
                "frame-inspection-rejected-await-anchor"
                    | "frame-inspection-rejected-trigger-recovery-anchor"
                    | "frame-await-recovery-anchor"
                    | "gap-repair-in-flight"
            )
        )
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
        if Self::renderer_shadow_blocks_recovery_release(input) {
            return false;
        }
        let stable_timeline_source = matches!(
            input.effective_source_event(),
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

    fn can_close_recovery_with_transient_present_feedback_gap(
        input: &VideoSchedulingOwnerInput,
        supply_state: DisplaySupplyState,
        has_clean_anchor_evidence: bool,
        chain_healthy: bool,
    ) -> bool {
        if !matches!(supply_state, DisplaySupplyState::Healthy) {
            return false;
        }
        if input.connection_state != ConnectionLifecycleStateFact::Connected {
            return false;
        }
        if !has_clean_anchor_evidence || !chain_healthy {
            return false;
        }
        if Self::renderer_shadow_blocks_recovery_release(input)
            || input.demand.present_age_ms.is_some()
        {
            return false;
        }
        let first_present_feedback_gap_active = Self::first_present_feedback_gap_active(input);
        if !first_present_feedback_gap_active {
            if matches!(
                input.demand.no_pending_pressure_level.as_deref(),
                Some("high" | "critical")
            ) {
                return false;
            }
            if input.demand.no_pending_streak.unwrap_or(u32::MAX) > 2 {
                return false;
            }
        }
        let stable_timeline_source = matches!(
            input.effective_source_event(),
            Some("frame-complete-candidate" | "frame-observed")
        );
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
        stable_timeline_source && decode_fresh && track_attached && track_has_video_bytes
    }

    fn can_restore_serving_after_clean_anchor(
        input: &VideoSchedulingOwnerInput,
        has_clean_anchor_evidence: bool,
        chain_healthy: bool,
        supply_state: DisplaySupplyState,
    ) -> bool {
        if input.connection_state != ConnectionLifecycleStateFact::Connected {
            return false;
        }
        if !has_clean_anchor_evidence || !chain_healthy {
            return false;
        }
        // RFC: 解码后队列溢出/本地丢帧属于显示域调度问题，不应阻断 recovery-to-serving；
        // 允许尽快回到 serving，让 release-clock + local drop 吸收局部积压。
        if Self::renderer_shadow_blocks_recovery_release(input) {
            return false;
        }
        if Self::has_unresolved_invalid_bootstrap_blocker(input) {
            return false;
        }
        let track_audio_only = matches!(input.latest_track_state.as_deref(), Some("audioOnly"));
        let track_attached = matches!(
            input.latest_track_state.as_deref(),
            Some("remoteTrackAttached")
        );
        let track_has_video_bytes = input
            .latest_track_video_bytes_total
            .is_some_and(|bytes| bytes > 0);
        if track_audio_only || !track_attached || !track_has_video_bytes {
            return false;
        }
        let media_continuity_ready = Self::has_recent_serviceable_media_continuity(
            input,
            has_clean_anchor_evidence,
            chain_healthy,
        );
        // 统一仲裁下，clean anchor 一旦成立，owner 应尽快退出 recovering，
        // 让 steady/degraded 分支继续吸收供给抖动，而不是继续卡在 rebuilding-supply。
        let decode_serviceable = input
            .demand
            .decode_age_ms
            .is_some_and(|age| age <= input.display_supply_thresholds.critical_decode_age_ms)
            || media_continuity_ready;
        if !decode_serviceable {
            return false;
        }
        input
            .demand
            .present_age_ms
            .is_some_and(|age| age <= input.display_supply_thresholds.critical_present_age_ms)
            || Self::can_close_recovery_with_transient_present_feedback_gap(
                input,
                supply_state,
                has_clean_anchor_evidence,
                chain_healthy,
            )
            || media_continuity_ready
    }

    fn has_recent_serviceable_media_continuity(
        input: &VideoSchedulingOwnerInput,
        has_clean_anchor_evidence: bool,
        chain_healthy: bool,
    ) -> bool {
        if input.connection_state != ConnectionLifecycleStateFact::Connected {
            return false;
        }
        if !has_clean_anchor_evidence
            || !chain_healthy
            || Self::renderer_shadow_blocks_recovery_release(input)
        {
            return false;
        }
        let track_attached = matches!(
            input.latest_track_state.as_deref(),
            Some("remoteTrackAttached")
        );
        let track_has_video_bytes = input
            .latest_track_video_bytes_total
            .is_some_and(|bytes| bytes > 0);
        if !track_attached || !track_has_video_bytes {
            return false;
        }
        let stable_timeline_source = matches!(
            input.effective_source_event(),
            Some("frame-complete-candidate" | "frame-observed")
        );
        if !stable_timeline_source {
            return false;
        }
        let continuity_metadata_ready = input.latest_h264_committed_sps_present == Some(true)
            && input.latest_h264_committed_pps_present == Some(true)
            && input.latest_h264_delta_continuation_ready == Some(true);
        if !continuity_metadata_ready {
            return false;
        }
        if !Self::post_clean_anchor_continuation_grace_active(input) {
            return false;
        }
        input
            .latest_h264_observed_at_ms
            .is_some_and(|observed_at_ms| {
                (input.observed_at_ms - observed_at_ms).max(0.0)
                    <= RECENT_H264_RECOVERY_BLOCKER_MAX_AGE_MS
            })
    }

    fn has_current_clean_anchor_release_evidence(input: &VideoSchedulingOwnerInput) -> bool {
        input.has_established_displayed_idr_fact()
    }

    fn current_release_anchor_observed_at_ms(input: &VideoSchedulingOwnerInput) -> Option<f64> {
        current_clean_anchor_observed_at_ms(
            input.clean_anchor_epoch,
            input.clean_anchor_observed_at_ms,
            input.clean_anchor_source_event.as_deref(),
            input.recovery_epoch,
        )
        .or_else(|| {
            current_clean_anchor_bridge_observed_at_ms(
                input.clean_anchor_bridge_epoch,
                input.clean_anchor_bridge_observed_at_ms,
                input.clean_anchor_bridge_source_event.as_deref(),
                input.recovery_epoch,
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
        // 临时诊断：只在 displayed-idr / fresh-anchor 已建立但 owner 仍未回稳时打点。
        let has_visible_clean_anchor_fact = has_clean_anchor_evidence
            || input.has_established_displayed_idr_fact()
            || input.recovery_fresh_anchor_recovered_at_ms.is_some()
            || matches!(
                input.clean_anchor_source_event.as_deref(),
                Some("displayed-idr")
            );
        if current_state != VideoSchedulingOwnerState::RebuildingSupply
            || next_state != VideoSchedulingOwnerState::RebuildingSupply
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
            input.effective_chain_state(),
            input.effective_source_event(),
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

    fn clean_anchor_hysteresis_allows_reentry(input: &VideoSchedulingOwnerInput) -> bool {
        Self::has_current_clean_anchor_release_evidence(input)
            && !Self::has_post_clean_anchor_transport_await_issue(input)
            && !Self::has_unresolved_invalid_bootstrap_blocker(input)
            && matches!(
                input.effective_source_event(),
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
        Self::is_transport_await_probe_signal(input)
            && Self::has_transport_await_hard_rebuild_evidence(input)
    }

    fn transient_anchor_noise_can_settle(
        input: &VideoSchedulingOwnerInput,
        has_clean_anchor_evidence: bool,
    ) -> bool {
        if !has_clean_anchor_evidence || Self::renderer_shadow_blocks_recovery_release(input) {
            return false;
        }
        if !matches!(
            input.latest_track_state.as_deref(),
            Some("remoteTrackAttached")
        ) || !input
            .latest_track_video_bytes_total
            .is_some_and(|bytes| bytes > 0)
        {
            return false;
        }
        if input.latest_h264_committed_sps_present != Some(true)
            || input.latest_h264_committed_pps_present != Some(true)
            || input.latest_h264_delta_continuation_ready != Some(true)
        {
            return false;
        }
        if !Self::post_clean_anchor_continuation_grace_active(input) {
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
        if !present_fresh || !decode_fresh {
            return false;
        }
        let anchor_candidate_stable =
            input
                .latest_anchor_candidate_ledger
                .as_ref()
                .is_some_and(|candidate| {
                    matches!(
                        candidate.state,
                        XbxEngineAnchorCandidateState::Observed
                            | XbxEngineAnchorCandidateState::Repaired
                            | XbxEngineAnchorCandidateState::SubmittedCleanAnchor
                    )
                });
        if !anchor_candidate_stable {
            return false;
        }
        matches!(
            input.effective_source_event(),
            Some("nack-observation" | "gap-repair-in-flight" | "gap-resolved")
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
        if Self::renderer_shadow_blocks_recovery_release(input) {
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

    /// ingress 已发布 `receiver_state` 时，timeline 仅作 trace，不参与 anchor 门控。
    fn receiver_observation_authoritative(input: &VideoSchedulingOwnerInput) -> bool {
        input.receiver_state.is_some()
    }

    fn timeline_indicates_anchor_issue(input: &VideoSchedulingOwnerInput) -> bool {
        if Self::steady_displayed_idr_bootstrap_continuation_active(input)
            && matches!(input.demand.host_cadence_phase.as_deref(), Some("steady"))
        {
            return false;
        }
        if input.has_established_displayed_idr_fact()
            && Self::has_current_clean_anchor_release_evidence(input)
            && matches!(
                input.effective_source_event(),
                Some("frame-await-recovery-anchor")
            )
            && !is_receiver_state_waiting_keyframe(input.effective_receiver_state())
        {
            return false;
        }
        if Self::receiver_observation_authoritative(input) {
            return is_receiver_state_waiting_keyframe(input.receiver_state.as_deref());
        }
        if let Some(observation) = input.latest_video_timeline_observation.as_ref() {
            if has_current_transport_await_issue_from_observation(
                observation,
                Self::current_release_anchor_observed_at_ms(input),
            ) {
                return true;
            }
        }
        is_ingress_waiting_keyframe(
            input.effective_receiver_state(),
            input.effective_chain_state(),
            input.effective_chain_reason(),
            input.effective_source_event(),
        )
    }

    fn has_post_clean_anchor_transport_await_issue(input: &VideoSchedulingOwnerInput) -> bool {
        if Self::receiver_observation_authoritative(input) {
            return false;
        }
        input
            .latest_video_timeline_observation
            .as_ref()
            .is_some_and(|observation| {
                has_current_transport_await_issue_from_observation(
                    observation,
                    Self::current_release_anchor_observed_at_ms(input),
                )
            })
    }

    fn transport_await_local_probe_probation_active(
        current: VideoSchedulingOwnerState,
        input: &VideoSchedulingOwnerInput,
        has_clean_anchor_evidence: bool,
        chain_healthy: bool,
    ) -> bool {
        if !matches!(
            current,
            VideoSchedulingOwnerState::StableServing
                | VideoSchedulingOwnerState::DegradedServing
                | VideoSchedulingOwnerState::SupplyStarved
        ) {
            return false;
        }
        if !has_clean_anchor_evidence || !chain_healthy {
            return false;
        }
        if !Self::is_transport_await_probe_signal(input) {
            return false;
        }
        !Self::has_transport_await_hard_rebuild_evidence(input)
    }

    fn is_transport_await_probe_signal(input: &VideoSchedulingOwnerInput) -> bool {
        is_transport_await_probe_source_event(input.effective_source_event())
    }

    fn has_transport_await_hard_rebuild_evidence(input: &VideoSchedulingOwnerInput) -> bool {
        Self::has_rejected_transport_await_anchor_candidate(input)
            || Self::has_post_startup_transport_await_bootstrap_failure(input)
            || Self::transport_await_chain_is_hard_broken(input)
    }

    fn has_rejected_transport_await_anchor_candidate(input: &VideoSchedulingOwnerInput) -> bool {
        input
            .latest_anchor_candidate_ledger
            .as_ref()
            .is_some_and(|candidate| {
                candidate.recovery_epoch == input.recovery_epoch
                    && candidate.state == XbxEngineAnchorCandidateState::Rejected
                    && matches!(
                        candidate.failure_reason,
                        Some(
                            crate::XbxEngineAnchorCandidateFailureReason::AwaitingRecoveryKeyframe
                                | crate::XbxEngineAnchorCandidateFailureReason::InspectionRejectedMissingSps
                                | crate::XbxEngineAnchorCandidateFailureReason::InspectionRejectedMissingPps
                                | crate::XbxEngineAnchorCandidateFailureReason::InspectionRejectedInvalidSliceHeader
                                | crate::XbxEngineAnchorCandidateFailureReason::ChainBrokenReferenceUnrecoverable
                                | crate::XbxEngineAnchorCandidateFailureReason::GapExpiredDeadline
                        )
                    )
            })
    }

    fn has_post_startup_transport_await_bootstrap_failure(
        input: &VideoSchedulingOwnerInput,
    ) -> bool {
        if Self::first_present_feedback_gap_active(input) {
            return false;
        }
        if input.latest_h264_bootstrap_ready != Some(false) {
            return false;
        }
        let Some(observed_at_ms) = input.latest_h264_observed_at_ms else {
            return false;
        };
        if (input.observed_at_ms - observed_at_ms).max(0.0)
            > RECENT_H264_RECOVERY_BLOCKER_MAX_AGE_MS
        {
            return false;
        }
        if Self::current_release_anchor_observed_at_ms(input)
            .is_some_and(|clean_anchor_at_ms| clean_anchor_at_ms >= observed_at_ms)
        {
            return false;
        }
        matches!(
            input.latest_h264_bootstrap_reject_reason.as_deref(),
            Some(
                "bootstrapMissingSps"
                    | "bootstrapMissingPps"
                    | "inspectionRejectInvalidSliceHeader"
            )
        )
    }

    fn transport_await_chain_is_hard_broken(input: &VideoSchedulingOwnerInput) -> bool {
        input.effective_chain_state() == Some("waiting-keyframe")
            && Self::is_transport_await_probe_signal(input)
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

    /// 无 clean anchor 但 decode/present 仍新鲜、链路 receiving：视为短暂供给抖动，避免误落 supply-starved。
    fn transient_serving_pipeline_healthy(input: &VideoSchedulingOwnerInput) -> bool {
        if input.connection_state != ConnectionLifecycleStateFact::Connected {
            return false;
        }
        if !matches!(input.effective_chain_state(), Some("receiving")) {
            return false;
        }
        let decode_fresh = input
            .demand
            .decode_age_ms
            .is_some_and(|age| age <= input.display_supply_thresholds.degraded_decode_age_ms);
        let present_fresh = input
            .demand
            .present_age_ms
            .is_some_and(|age| age <= input.display_supply_thresholds.degraded_present_age_ms);
        let track_attached = matches!(
            input.latest_track_state.as_deref(),
            Some("remoteTrackAttached")
        );
        let track_has_video_bytes = input
            .latest_track_video_bytes_total
            .is_some_and(|bytes| bytes > 0);
        decode_fresh && present_fresh && track_attached && track_has_video_bytes
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
        if Self::renderer_shadow_blocks_recovery_release(input) {
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

    fn should_gate_soft_display_supply_critical(
        &self,
        input: &VideoSchedulingOwnerInput,
        has_clean_anchor_evidence: bool,
        chain_healthy: bool,
    ) -> bool {
        if input.connection_state != ConnectionLifecycleStateFact::Connected {
            return false;
        }
        if !matches!(
            self.state,
            VideoSchedulingOwnerState::StableServing
                | VideoSchedulingOwnerState::DegradedServing
                | VideoSchedulingOwnerState::SupplyStarved
        ) {
            return false;
        }
        if Self::renderer_shadow_blocks_recovery_release(input) {
            return false;
        }
        let track_attached = matches!(
            input.latest_track_state.as_deref(),
            Some("remoteTrackAttached")
        );
        let track_has_video_bytes = input
            .latest_track_video_bytes_total
            .is_some_and(|bytes| bytes > 0);
        track_attached && track_has_video_bytes && (has_clean_anchor_evidence || chain_healthy)
    }

    fn should_hold_supply_starved_transition(
        &mut self,
        input: &VideoSchedulingOwnerInput,
        supply_state: DisplaySupplyState,
        has_anchor_issue: bool,
        completion_evidence: RecoveryCompletionEvidence,
    ) -> bool {
        if has_anchor_issue || completion_evidence == RecoveryCompletionEvidence::Ready {
            self.display_supply_recovery_gate
                .pending_supply_starved_since_ms = None;
            self.display_supply_recovery_gate
                .pending_supply_starved_label = None;
            return false;
        }
        if !matches!(
            self.state,
            VideoSchedulingOwnerState::StableServing | VideoSchedulingOwnerState::DegradedServing
        ) {
            self.display_supply_recovery_gate
                .pending_supply_starved_since_ms = None;
            self.display_supply_recovery_gate
                .pending_supply_starved_label = None;
            return false;
        }
        let label = match supply_state {
            DisplaySupplyState::Critical => "displaySupplyCritical",
            DisplaySupplyState::Degraded => "displaySupplyDegraded",
            DisplaySupplyState::Healthy => {
                self.display_supply_recovery_gate
                    .pending_supply_starved_since_ms = None;
                self.display_supply_recovery_gate
                    .pending_supply_starved_label = None;
                return false;
            }
        };
        if input.connection_state != ConnectionLifecycleStateFact::Connected
            || Self::renderer_shadow_blocks_recovery_release(input)
        {
            self.display_supply_recovery_gate
                .pending_supply_starved_since_ms = None;
            self.display_supply_recovery_gate
                .pending_supply_starved_label = None;
            return false;
        }
        let chain_healthy = matches!(input.effective_chain_state(), Some("receiving"));
        let track_attached = matches!(
            input.latest_track_state.as_deref(),
            Some("remoteTrackAttached")
        );
        let track_has_video_bytes = input
            .latest_track_video_bytes_total
            .is_some_and(|bytes| bytes > 0);
        let decode_fresh = input
            .demand
            .decode_age_ms
            .is_some_and(|age| age <= input.display_supply_thresholds.degraded_decode_age_ms * 1.5);
        if !(chain_healthy && track_attached && track_has_video_bytes && decode_fresh) {
            self.display_supply_recovery_gate
                .pending_supply_starved_since_ms = None;
            self.display_supply_recovery_gate
                .pending_supply_starved_label = None;
            return false;
        }
        if self
            .display_supply_recovery_gate
            .pending_supply_starved_label
            != Some(label)
        {
            self.display_supply_recovery_gate
                .pending_supply_starved_label = Some(label);
            self.display_supply_recovery_gate
                .pending_supply_starved_since_ms = Some(input.observed_at_ms);
            return true;
        }
        let Some(since_ms) = self
            .display_supply_recovery_gate
            .pending_supply_starved_since_ms
        else {
            self.display_supply_recovery_gate
                .pending_supply_starved_since_ms = Some(input.observed_at_ms);
            return true;
        };
        (input.observed_at_ms - since_ms).max(0.0) < DISPLAY_SUPPLY_STARVED_CONFIRM_MS
    }

    fn build_supply_break_recovery_intent(
        owner: &mut Self,
        input: &VideoSchedulingOwnerInput,
        supply_state: DisplaySupplyState,
        host_present_stall_active: bool,
        host_present_stall_streak: u32,
    ) -> Option<RecoveryIntentContract> {
        use crate::transport::rtc::recovery::contract::DerivedDecoderHealth;
        if Self::displayed_idr_serving_release_active(input, host_present_stall_streak) {
            owner.last_intent = None;
            return None;
        }
        if host_present_stall_active {
            let reason = OwnerRecoveryReason::HostPresentStalled;
            let label = reason.as_reason_label();
            return Some(RecoveryIntentContract {
                emit: owner.should_emit_intent(reason.source(), label, input.observed_at_ms),
                source: reason.source(),
                reason,
                reason_label: label.to_string(),
            });
        }
        if matches!(
            input.derived_decoder_health,
            DerivedDecoderHealth::SupplyStalled
        ) {
            let reason = match supply_state {
                DisplaySupplyState::Critical => OwnerRecoveryReason::DisplaySupplyCritical,
                DisplaySupplyState::Degraded => OwnerRecoveryReason::DisplaySupplyDegraded,
                DisplaySupplyState::Healthy => return None,
            };
            return Some(RecoveryIntentContract {
                emit: owner.should_emit_intent(
                    reason.source(),
                    reason.as_reason_label(),
                    input.observed_at_ms,
                ),
                source: reason.source(),
                reason,
                reason_label: reason.as_reason_label().to_string(),
            });
        }
        owner.last_intent = None;
        None
    }

    fn rebuilding_supply_strong_anchor_intent(
        input: &VideoSchedulingOwnerInput,
        _completion_evidence: RecoveryCompletionEvidence,
    ) -> bool {
        input.anchor_reason_label.is_some()
            || Self::timeline_indicates_anchor_issue(input)
            || Self::has_transport_await_hard_rebuild_evidence(input)
    }

    fn build_recovery_intent(
        &mut self,
        state: VideoSchedulingOwnerState,
        input: &VideoSchedulingOwnerInput,
        supply_state: DisplaySupplyState,
        completion_evidence: RecoveryCompletionEvidence,
        ingress_waiting_keyframe: bool,
        host_present_stall_active: bool,
        host_present_stall_streak: u32,
    ) -> Option<RecoveryIntentContract> {
        let routed_state = match intent_table::route(
            input.recovery_surface_phase,
            state,
            ingress_waiting_keyframe,
        ) {
            intent_table::IntentRoute::SupplyBreakSupplyStarved => {
                return Self::build_supply_break_recovery_intent(
                    self,
                    input,
                    supply_state,
                    host_present_stall_active,
                    host_present_stall_streak,
                );
            }
            intent_table::IntentRoute::AwaitIdrIngressWaiting => {
                return Self::recovery_intent_for_ingress_waiting_keyframe(self, input, state);
            }
            intent_table::IntentRoute::ByOwnerState(owner_state) => owner_state,
        };
        let contract = match routed_state {
            VideoSchedulingOwnerState::RebuildingSupply => {
                if Self::displayed_idr_serving_release_active(input, host_present_stall_streak) {
                    self.last_intent = None;
                    return None;
                }
                let strong_anchor =
                    Self::rebuilding_supply_strong_anchor_intent(input, completion_evidence);
                let (reason, label) = if strong_anchor {
                    let reason = OwnerRecoveryReason::TransportAwaitRecoveryKeyframe;
                    (reason, reason.as_reason_label().to_string())
                } else {
                    let reason = OwnerRecoveryReason::LocalSupplySuspect;
                    let label = reason.as_reason_label().to_string();
                    (reason, label)
                };
                Some(RecoveryIntentContract {
                    emit: self.should_emit_intent(reason.source(), &label, input.observed_at_ms),
                    source: reason.source(),
                    reason,
                    reason_label: label,
                })
            }
            VideoSchedulingOwnerState::SupplyStarved => {
                if Self::displayed_idr_serving_release_active(input, host_present_stall_streak) {
                    self.last_intent = None;
                    return None;
                }
                if input.has_established_displayed_idr_fact()
                    && Self::has_current_clean_anchor_release_evidence(input)
                    && matches!(
                        input.effective_source_event(),
                        Some("frame-await-recovery-anchor")
                    )
                    && !host_present_stall_active
                    && !is_receiver_state_waiting_keyframe(input.effective_receiver_state())
                {
                    self.last_intent = None;
                    return None;
                }
                if host_present_stall_active {
                    let reason = OwnerRecoveryReason::HostPresentStalled;
                    let label = reason.as_reason_label();
                    return Some(RecoveryIntentContract {
                        emit: self.should_emit_intent(reason.source(), label, input.observed_at_ms),
                        source: reason.source(),
                        reason,
                        reason_label: label.to_string(),
                    });
                }
                if ingress_waiting_keyframe
                    && !Self::timeline_indicates_anchor_issue(input)
                    && input.anchor_reason_label.is_none()
                {
                    return Self::recovery_intent_for_ingress_waiting_keyframe(self, input, state);
                }
                if ingress_waiting_keyframe
                    || Self::timeline_indicates_anchor_issue(input)
                    || input.anchor_reason_label.is_some()
                {
                    let reason = OwnerRecoveryReason::TransportAwaitRecoveryKeyframe;
                    let label = reason.as_reason_label();
                    return Some(RecoveryIntentContract {
                        emit: self.should_emit_intent(reason.source(), label, input.observed_at_ms),
                        source: reason.source(),
                        reason,
                        reason_label: label.to_string(),
                    });
                }
                let reason = match supply_state {
                    DisplaySupplyState::Critical => OwnerRecoveryReason::DisplaySupplyCritical,
                    DisplaySupplyState::Degraded => OwnerRecoveryReason::DisplaySupplyDegraded,
                    DisplaySupplyState::Healthy => return None,
                };
                Some(RecoveryIntentContract {
                    emit: self.should_emit_intent(
                        reason.source(),
                        reason.as_reason_label(),
                        input.observed_at_ms,
                    ),
                    source: reason.source(),
                    reason,
                    reason_label: reason.as_reason_label().to_string(),
                })
            }
            VideoSchedulingOwnerState::StableServing
            | VideoSchedulingOwnerState::DegradedServing => {
                if host_present_stall_active {
                    let reason = OwnerRecoveryReason::HostPresentStalled;
                    let label = reason.as_reason_label();
                    return Some(RecoveryIntentContract {
                        emit: self.should_emit_intent(reason.source(), label, input.observed_at_ms),
                        source: reason.source(),
                        reason,
                        reason_label: label.to_string(),
                    });
                }
                if ingress_waiting_keyframe
                    && !Self::displayed_idr_serving_release_active(input, host_present_stall_streak)
                {
                    return Self::recovery_intent_for_ingress_waiting_keyframe(self, input, state);
                }
                self.last_intent = None;
                None
            }
            VideoSchedulingOwnerState::SeekingAnchor | VideoSchedulingOwnerState::Priming => {
                self.last_intent = None;
                None
            }
        };
        contract
    }

    fn recovery_intent_for_ingress_waiting_keyframe(
        &mut self,
        input: &VideoSchedulingOwnerInput,
        state: VideoSchedulingOwnerState,
    ) -> Option<RecoveryIntentContract> {
        match input.recovery_exit_path {
            RecoveryExitPath::TimedFallback => {
                let reason = match state {
                    VideoSchedulingOwnerState::DegradedServing
                    | VideoSchedulingOwnerState::StableServing => {
                        OwnerRecoveryReason::DisplaySupplyDegraded
                    }
                    _ => OwnerRecoveryReason::LocalSupplySuspect,
                };
                let label = reason.as_reason_label().to_string();
                Some(RecoveryIntentContract {
                    emit: self.should_emit_intent(reason.source(), &label, input.observed_at_ms),
                    source: reason.source(),
                    reason,
                    reason_label: label,
                })
            }
            RecoveryExitPath::DecodeOutput => {
                self.last_intent = None;
                None
            }
            _ => {
                let reason = OwnerRecoveryReason::TransportAwaitRecoveryKeyframe;
                let label = reason.as_reason_label();
                Some(RecoveryIntentContract {
                    emit: self.should_emit_intent(reason.source(), label, input.observed_at_ms),
                    source: reason.source(),
                    reason,
                    reason_label: label.to_string(),
                })
            }
        }
    }

    /// steady 播放中 inspection 观测到 bootstrapMissingIdr 的非 IDR continuation（SPS/PPS 已 committed）。
    fn steady_displayed_idr_bootstrap_continuation_active(
        input: &VideoSchedulingOwnerInput,
    ) -> bool {
        if !input.displayed_idr_serving_wide || !input.has_established_displayed_idr_fact() {
            return false;
        }
        if input.latest_h264_committed_sps_present != Some(true)
            || input.latest_h264_committed_pps_present != Some(true)
            || input.latest_h264_delta_continuation_ready != Some(true)
        {
            return false;
        }
        input.latest_h264_bootstrap_ready == Some(false)
            && matches!(
                input.latest_h264_bootstrap_reject_reason.as_deref(),
                Some("bootstrapMissingIdr" | "NonIdrVcl")
            )
    }

    /// displayed-idr 已落地且 host/decode 持续新鲜：不应再挂 transport-await / rebuilding-supply。
    fn displayed_idr_serving_release_active(
        input: &VideoSchedulingOwnerInput,
        host_present_stall_streak: u32,
    ) -> bool {
        if !input.displayed_idr_control_plane_active()
            || !input.has_established_displayed_idr_fact()
        {
            return false;
        }
        if input
            .demand
            .submit_age_ms
            .is_some_and(|age| age >= DISPLAYED_IDR_SERVING_STALE_SUBMIT_BREAK_MS)
        {
            return false;
        }
        if matches!(input.recovery_exit_path, RecoveryExitPath::TimedFallback) {
            return false;
        }
        if input.connection_state != ConnectionLifecycleStateFact::Connected {
            return false;
        }
        if is_receiver_state_waiting_keyframe(input.effective_receiver_state()) {
            if input.contract_snapshot.serving_relaxed
                && host_present_stall_streak < HOST_PRESENT_STALL_TICK_STREAK_MIN
            {
                // relaxed 控制面：不因 receiver waiting-keyframe 单独阻断 release。
            } else if host_present_stall_streak >= HOST_PRESENT_STALL_TICK_STREAK_MIN {
                return false;
            } else {
                let decode_stale = input.demand.decode_age_ms.is_some_and(|age| {
                    age > input.display_supply_thresholds.degraded_decode_age_ms * 2.0
                });
                if decode_stale {
                    return false;
                }
            }
        }
        if Self::renderer_shadow_blocks_recovery_release(input) {
            return false;
        }
        if !matches!(
            input.latest_track_state.as_deref(),
            Some("remoteTrackAttached")
        ) || !input
            .latest_track_video_bytes_total
            .is_some_and(|bytes| bytes > 0)
        {
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
        if !present_fresh || !decode_fresh {
            return Self::steady_displayed_idr_bootstrap_continuation_active(input)
                && matches!(input.demand.host_cadence_phase.as_deref(), Some("steady"));
        }
        matches!(input.demand.host_cadence_phase.as_deref(), Some("steady"))
            || input.demand.host_frame_present_epoch.unwrap_or(0) > 0
    }

    fn latest_clean_anchor_submitted_at_ms(input: &VideoSchedulingOwnerInput) -> Option<f64> {
        input
            .recovery_fresh_anchor_recovered_at_ms
            .or(input.recovery_displayed_idr_at_ms)
            .or_else(|| {
                current_clean_anchor_observed_at_ms(
                    input.clean_anchor_epoch,
                    input.clean_anchor_observed_at_ms,
                    input.clean_anchor_source_event.as_deref(),
                    input.recovery_epoch,
                )
            })
    }

    fn has_unresolved_invalid_bootstrap_blocker(input: &VideoSchedulingOwnerInput) -> bool {
        if input.has_established_displayed_idr_fact() {
            return false;
        }
        if input.latest_h264_bootstrap_ready != Some(false)
            || !is_invalid_recovery_bootstrap_reject_reason(
                input.latest_h264_bootstrap_reject_reason.as_deref(),
            )
        {
            return false;
        }
        let Some(observed_at_ms) = input.latest_h264_observed_at_ms else {
            return false;
        };
        if (input.observed_at_ms - observed_at_ms).max(0.0)
            > RECENT_H264_RECOVERY_BLOCKER_MAX_AGE_MS
        {
            return false;
        }
        let metadata_ready = input.latest_h264_committed_sps_present == Some(true)
            && input.latest_h264_committed_pps_present == Some(true)
            && input.latest_h264_delta_continuation_ready == Some(true);
        !metadata_ready
    }

    fn has_recent_continuation_only_recovery_blocker(input: &VideoSchedulingOwnerInput) -> bool {
        if input.latest_h264_bootstrap_ready != Some(false) {
            return false;
        }
        if !matches!(
            input.latest_h264_bootstrap_reject_reason.as_deref(),
            Some("bootstrapMissingIdr" | "NonIdrVcl")
        ) {
            return false;
        }
        if input.latest_h264_committed_sps_present != Some(true)
            || input.latest_h264_committed_pps_present != Some(true)
            || input.latest_h264_delta_continuation_ready != Some(true)
        {
            return false;
        }
        input
            .latest_h264_observed_at_ms
            .is_some_and(|observed_at_ms| {
                (input.observed_at_ms - observed_at_ms).max(0.0)
                    <= RECENT_H264_RECOVERY_BLOCKER_MAX_AGE_MS
            })
    }

    fn latest_clean_anchor_age_ms(input: &VideoSchedulingOwnerInput) -> Option<f64> {
        Self::latest_clean_anchor_submitted_at_ms(input)
            .map(|submitted_at_ms| (input.observed_at_ms - submitted_at_ms).max(0.0))
    }

    fn post_clean_anchor_continuation_grace_active(input: &VideoSchedulingOwnerInput) -> bool {
        Self::latest_clean_anchor_age_ms(input)
            .is_some_and(|age_ms| age_ms <= POST_CLEAN_ANCHOR_CONTINUATION_GRACE_MS)
    }

    fn should_reenter_anchor_recovery_after_clean_anchor(
        current: VideoSchedulingOwnerState,
        input: &VideoSchedulingOwnerInput,
    ) -> bool {
        if input.has_established_displayed_idr_fact() {
            return false;
        }
        if !matches!(
            current,
            VideoSchedulingOwnerState::StableServing
                | VideoSchedulingOwnerState::DegradedServing
                | VideoSchedulingOwnerState::SupplyStarved
        ) {
            return false;
        }
        if input.connection_state != ConnectionLifecycleStateFact::Connected {
            return false;
        }
        if !Self::has_current_clean_anchor_release_evidence(input)
            || Self::post_clean_anchor_continuation_grace_active(input)
        {
            return false;
        }
        if Self::receiver_observation_authoritative(input) {
            if !is_receiver_state_waiting_keyframe(input.receiver_state.as_deref()) {
                return false;
            }
        } else if !Self::has_recent_continuation_only_recovery_blocker(input) {
            return false;
        }
        if !matches!(
            input.latest_track_state.as_deref(),
            Some("remoteTrackAttached")
        ) || !input
            .latest_track_video_bytes_total
            .is_some_and(|bytes| bytes > 0)
        {
            return false;
        }
        let gap_escalates_recovery = match input.latest_video_timeline_observation.as_ref() {
            None => true,
            Some(timeline) => {
                let gs = derive_gap_severity_from_timeline_observation(timeline);
                matches!(
                    gs,
                    GapSeverity::ReferenceGap
                        | GapSeverity::AnchorGap
                        | GapSeverity::ChainBroken
                        | GapSeverity::RecoveryBlocked
                )
            }
        };
        let has_transport_await_hard_evidence =
            Self::has_transport_await_hard_rebuild_evidence(input);
        if !gap_escalates_recovery && !has_transport_await_hard_evidence {
            return false;
        }
        matches!(
            input.effective_source_event(),
            Some(
                "frame-complete-candidate"
                    | "frame-observed"
                    | "gap-repair-in-flight"
                    | "gap-resolved"
                    | "gap-reorder-pending"
            )
        )
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

/// (surface_phase, owner_state) → intent 构建路径；避免与 L0 contract 平行嵌套 if。
mod intent_table {
    use super::VideoSchedulingOwnerState;
    use crate::transport::rtc::recovery::contract::RecoverySurfacePhase;

    pub(super) enum IntentRoute {
        SupplyBreakSupplyStarved,
        AwaitIdrIngressWaiting,
        ByOwnerState(VideoSchedulingOwnerState),
    }

    pub(super) fn route(
        surface: RecoverySurfacePhase,
        state: VideoSchedulingOwnerState,
        ingress_waiting_keyframe: bool,
    ) -> IntentRoute {
        match (surface, state) {
            (RecoverySurfacePhase::SupplyBreak, VideoSchedulingOwnerState::SupplyStarved) => {
                IntentRoute::SupplyBreakSupplyStarved
            }
            (RecoverySurfacePhase::AwaitIdr, VideoSchedulingOwnerState::RebuildingSupply)
                if ingress_waiting_keyframe =>
            {
                IntentRoute::AwaitIdrIngressWaiting
            }
            _ => IntentRoute::ByOwnerState(state),
        }
    }
}

#[cfg(test)]
#[path = "video_scheduling_owner.test.rs"]
mod tests;
