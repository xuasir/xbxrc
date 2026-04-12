//! WebRTC session 顶层编排：BWE、视频 owner、recovery coordinator、昂贵恢复门控。
//! RFC：`FaultDomain`/`CostCeiling` 语义见 `session::control_model` 与 `recovery/coordinator` 注释对照。

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::api::backend::{
    XbxEngineKeyframeRequestEpisodeObservation, XbxEngineMediaRuntimeStats,
    XbxEngineRecoveryBudgetSnapshot, XbxEngineRecoveryDecisionLedgerObservation,
};
use crate::api::runtime::XbxEngineRuntimeConfig;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::bwe::evaluator::RtcBweEvaluation;
use crate::transport::rtc::bwe::policy::resolve_target_remb_kbps;
use crate::transport::rtc::facts::{ConnectionLifecycleStateFact, SessionCommand};
use crate::transport::rtc::policy::bwe::BwePolicyProposal;
use crate::transport::rtc::policy::planner::PlannedTransportCommand;
use crate::transport::rtc::policy::recovery::RecoveryPolicyProposal;
use crate::transport::rtc::policy::scheduling::{
    map_planned_command_to_session_commands, SchedulingPolicyEngine, SchedulingPolicyInput,
    TwccWarmupState,
};
use crate::transport::rtc::policy::video_scheduling_owner::{
    RecoveryIntentContract, RecoveryIntentSource, VideoSchedulingOwner, VideoSchedulingOwnerState,
};
use crate::transport::rtc::projection::TransportSnapshot;
use crate::transport::rtc::recovery::contract::{
    current_clean_anchor_observed_at_ms, has_current_transport_await_issue_from_observation,
    is_ingress_waiting_keyframe, is_media_healthy_baseline,
    is_terminal_transport_await_deferred_episode,
};
use crate::transport::rtc::recovery::coordinator::{
    RecoveryCoordinator, RecoveryCoordinatorProposal, RecoveryOwnerSignal,
};
use crate::transport::rtc::recovery::escalation::{
    RecoveryAction, VideoEscalationController, VideoEscalationReason,
};
use crate::transport::rtc::recovery::policy::ScenarioPolicyProfileKind;
use crate::transport::rtc::recovery::runtime_state::{
    has_fresh_media_output, resolve_recovery_profile,
};
use crate::transport::rtc::recovery::startup::SessionPhase;
use crate::transport::rtc::session::actor::SessionPolicyHook;
use crate::transport::rtc::session::connectivity_reason::{
    map_label_to_escalation_reason, parse_session_phase, resolve_connectivity_fallback_reason,
    resolve_lifecycle_reconnect_reason_label,
};
use crate::transport::rtc::session::control_model::{
    owner_recovery_reason_to_escalation_reason, resolve_session_fault_domain,
    resolve_session_recovery_stage, session_cost_ceiling_for_recovery_action, SessionFaultDomain,
};
use crate::transport::rtc::session::expensive_recovery_gate::ExpensiveRecoveryGate;
use crate::transport::rtc::session::facts::{
    build_rtc_session_policy_orchestration_input, RtcSessionPolicyOrchestrationInput,
};
use crate::transport::rtc::session::recovery_ramp_guard::{
    ramp_up_active, resolve_stable_recovery_settle,
    should_absorb_light_recovery_signal_during_ramp_up, RecoveryRampResolution,
};
use crate::transport::rtc::session::startup_compat::{
    first_frame_acquisition_priority_active, should_hold_pre_first_frame_connected_idle_timeout,
    should_hold_pre_first_frame_display_supply_degraded,
};

const DEFAULT_BWE_TARGET_KBPS: u32 = 16_000;
const BWE_UNSTABLE_HOLD_CONFIRMATION_TICKS: u8 = 2;
const RECOVERY_STARTUP_GRACE_MS: u64 = 800;
const RECOVERING_RECONNECT_PROPOSAL_INTERVAL_MS: f64 = 1_500.0;
const CONNECTING_PRE_FIRST_FRAME_RECONNECT_PROPOSAL_INTERVAL_MS: f64 = 4_500.0;
const CLOUD_RECOVERING_RECONNECT_PROPOSAL_INTERVAL_MS: f64 = 2_500.0;
const CLOUD_BUILDER_CONFIGURED_RECONNECT_PROPOSAL_INTERVAL_MS: f64 = 4_500.0;
const CLOUD_MISSING_LOCAL_FEEDBACK_RECONNECT_PROPOSAL_INTERVAL_MS: f64 = 3_500.0;
const RECOVERY_NO_PROGRESS_RECONNECT_FALLBACK_MS: f64 = 4_000.0;
const CONNECTING_PRE_FIRST_FRAME_FAILED_TERMINAL_MIN_MS: f64 = 90_000.0;
const CONNECTED_PRESENT_STALL_RECONNECT_FALLBACK_MS: f64 = 10_000.0;
const CONNECTED_PRESENT_STALL_MIN_AGE_MS: f64 = 1_500.0;
const CONNECTED_PRESENT_STALL_HARD_AGE_MS: f64 = 4_000.0;
const CONNECTED_INGRESS_WITHOUT_SUCCESS_OUTPUT_FAILED_TERMINAL_MS: f64 = 12_000.0;
const CONNECTED_CONNECTIVITY_EVIDENCE_STALE_MS: f64 = 3_000.0;
const LIVENESS_RECONNECT_ATTEMPT_LIMIT: u8 = 3;
const CLOUD_LIVENESS_RECONNECT_ATTEMPT_LIMIT: u8 = 6;
const ADAPTER_IDLE_RENDER_SLACK_MIN_MS: f64 = 220.0;
const ADAPTER_IDLE_RENDER_SLACK_MAX_MS: f64 = 450.0;
const RECENT_RECOVERY_DECISION_LEDGER_CAPACITY: usize = 64;
const RECOVERY_OBSERVATION_WINDOW_MS: f64 = 3_000.0;
const RECOVERY_OBSERVATION_NO_PROGRESS_FALLBACK_MS: f64 = 1_200.0;
const RECOVERY_OBSERVATION_KEYFRAME_WINDOW_MS: f64 = 900.0;
/// `transportAwaitRecoveryKeyframe` 门控用的观测仅统计该诊断链，避免同 epoch 内其它 keyframe/decoder 自愈污染。
const TRANSPORT_AWAIT_RECOVERY_KEYFRAME_DIAGNOSIS: &str = "transportAwaitRecoveryKeyframe";

fn recovery_decision_ledger_recovery_epoch(
    ledger: &XbxEngineRecoveryDecisionLedgerObservation,
) -> Option<u64> {
    ledger
        .budget_after
        .as_ref()
        .map(|budget| budget.recovery_epoch)
        .or_else(|| {
            ledger
                .budget_before
                .as_ref()
                .map(|budget| budget.recovery_epoch)
        })
}

/// 与当前 `transport_recovery_epoch` 对齐的观测下界：排除上一轮恢复在滑窗内遗留的自愈痕迹。
fn recovery_observation_epoch_floor_ms(stats: &XbxEngineMediaRuntimeStats) -> f64 {
    stats.transport_recovery_episode_opened_at_ms.unwrap_or(0.0)
}

fn ledger_input_signals_transport_await_recovery_keyframe(
    ledger: &XbxEngineRecoveryDecisionLedgerObservation,
) -> bool {
    ledger
        .input_signal
        .contains(TRANSPORT_AWAIT_RECOVERY_KEYFRAME_DIAGNOSIS)
}

fn is_transport_await_keyframe_episode(
    episode: &XbxEngineKeyframeRequestEpisodeObservation,
) -> bool {
    episode.request_reason.as_deref() == Some(TRANSPORT_AWAIT_RECOVERY_KEYFRAME_DIAGNOSIS)
}

/// 取当前 stats 下最近一条 transport-await keyframe episode 的请求/解码时间（用于 reconnect fallback 门控）。
fn transport_await_keyframe_episode_latest_times(
    stats: &XbxEngineMediaRuntimeStats,
) -> (Option<f64>, Option<f64>) {
    let mut best_requested_at_ms: Option<f64> = None;
    let mut best_first_keyframe_decoded_at_ms: Option<f64> = None;

    let mut consider = |episode: &XbxEngineKeyframeRequestEpisodeObservation| {
        if !is_transport_await_keyframe_episode(episode) {
            return;
        }
        if best_requested_at_ms.is_none_or(|prev| episode.requested_at_ms >= prev) {
            best_requested_at_ms = Some(episode.requested_at_ms);
            best_first_keyframe_decoded_at_ms = episode.first_keyframe_decoded_at_ms;
        }
    };

    for episode in stats.recent_keyframe_request_episodes.iter() {
        consider(episode);
    }
    if let Some(episode) = stats.latest_keyframe_request_episode.as_ref() {
        consider(episode);
    }
    (best_requested_at_ms, best_first_keyframe_decoded_at_ms)
}

/// 仅用于 recovery decision ledger 的 `state_before` / `state_after` 叙事字符串，**不参与**策略分支编排。
/// 顶层编排语义以 `control_model` 的 Stage / FaultDomain / CostCeiling 为准。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecoveryLedgerNarrativeState {
    Detecting,
    Observing,
    LocalSelfHealing,
    RecoveryEligible,
    ActiveRecovery,
    RecoveryBlocked,
    RampUp,
    Reconnecting,
    Stable,
    FailedTerminal,
}

#[derive(Clone, Copy, Debug, Default)]
struct ConnectedRenderLivenessSignal {
    latest_video_host_present_time_ms: Option<f64>,
    inbound_primary_video_bytes_total: u64,
    no_pending_pressure_is_high: bool,
}

#[derive(Clone, Debug)]
struct RecoveryObservationSnapshot {
    ingress_active: bool,
    reassembly_active: bool,
    decode_active: bool,
    render_active: bool,
    rtc_connectivity_connected: bool,
    reconnect_in_flight: bool,
    stable_serving: bool,
    last_media_progress_at: Option<f64>,
    last_video_decode_ok_at: Option<f64>,
    last_keyframe_requested_at: Option<f64>,
    last_keyframe_decoded_at: Option<f64>,
    local_decoder_reset_count_in_window: u32,
    keyframe_request_count_in_window: u32,
}

impl RecoveryObservationSnapshot {
    /// 仅计入与 `transportAwaitRecoveryKeyframe` 链对齐的 decoder reset / keyframe；不包含 NACK skip：
    /// `latest_video_nack_observation` 无恢复链语义，同 epoch 内任意 skip 会误满足「已尝试局部自愈」。
    fn local_self_healing_attempted(&self) -> bool {
        self.local_decoder_reset_count_in_window > 0 || self.keyframe_request_count_in_window > 0
    }

    fn media_progress_is_stalled_long_enough(&self, observed_at_ms: f64) -> bool {
        let latest_progress = [
            self.last_media_progress_at,
            self.last_video_decode_ok_at,
            self.last_keyframe_decoded_at,
        ]
        .into_iter()
        .flatten()
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
        latest_progress.is_some_and(|last| {
            (observed_at_ms - last).max(0.0) >= RECOVERY_OBSERVATION_NO_PROGRESS_FALLBACK_MS
        })
    }

    fn outside_keyframe_recovery_window(&self, observed_at_ms: f64) -> bool {
        self.last_keyframe_requested_at.is_none_or(|last| {
            (observed_at_ms - last).max(0.0) >= RECOVERY_OBSERVATION_KEYFRAME_WINDOW_MS
        })
    }

    fn allows_transport_await_reconnect_fallback(&self, observed_at_ms: f64) -> bool {
        let has_media_stage_signal = self.ingress_active
            || self.reassembly_active
            || self.decode_active
            || self.render_active;
        self.rtc_connectivity_connected
            && !self.reconnect_in_flight
            && !self.stable_serving
            && has_media_stage_signal
            && self.local_self_healing_attempted()
            && self.media_progress_is_stalled_long_enough(observed_at_ms)
            && self.outside_keyframe_recovery_window(observed_at_ms)
    }
}

impl RecoveryLedgerNarrativeState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Detecting => "detecting",
            Self::Observing => "observing",
            Self::LocalSelfHealing => "local-self-healing",
            Self::RecoveryEligible => "recovery-eligible",
            Self::ActiveRecovery => "active-recovery",
            Self::RecoveryBlocked => "recovery-blocked",
            Self::RampUp => "ramp-up",
            Self::Reconnecting => "reconnecting",
            Self::Stable => "stable",
            Self::FailedTerminal => "failed-terminal",
        }
    }
}

fn is_first_frame_acquisition_reason_label(value: &str) -> bool {
    matches!(
        value,
        "bootstrapMissingSps"
            | "bootstrapMissingPps"
            | "recoverySustaining"
            | "inspectionRejectInvalidSliceHeader"
            | "NonIdrVcl"
            | "transportAwaitRecoveryKeyframe"
            | "ingressWaitKeyframe"
    )
}

/// rtc session 主线策略：
/// - 统一把 reconnect/recovery/BWE proposal 收口到 session policy
/// - 复用 planner 的优先级（reconnect > recovery > bwe）
/// - stack 只做命令执行与 CommandResultFact 回写
///
/// RFC 名词与 `session::control_model` 对齐：`Stage` / `FaultDomain` / `CostCeiling` 仅作编排语义；
/// 禁止新增包级 NACK 依赖；昂贵门控见 `expensive_recovery_gate`。
pub struct RtcSessionPolicy {
    runtime_config: Arc<Mutex<XbxEngineRuntimeConfig>>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    scheduling_engine: SchedulingPolicyEngine,
    scheduling_owner: VideoSchedulingOwner,
    recovery_coordinator: RecoveryCoordinator,
    stream_started_at: Instant,
    escalation_profile_kind: ScenarioPolicyProfileKind,
    last_recovery_epoch: u64,
    last_bwe_sample_tick_ms: Option<f64>,
    last_sent_remb_kbps: u32,
    hybrid_ramp_cooldown_ticks: u8,
    next_bwe_observation_id: u64,
    last_bwe_reason: Option<String>,
    unstable_hold_streak: u8,
    last_lifecycle_reconnect_proposal_at_ms: Option<f64>,
    recovery_no_progress_since_ms: Option<f64>,
    recovery_no_progress_last_frame_count: Option<u64>,
    recovery_no_progress_last_transport_progress_token: Option<u64>,
    failed_terminal_since_ms: Option<f64>,
    failed_terminal_reason: Option<String>,
    failed_terminal_last_frame_count: Option<u64>,
    connected_render_stall_since_ms: Option<f64>,
    connected_render_last_present_time_ms: Option<f64>,
    connected_render_last_inbound_video_bytes_total: Option<u64>,
    connected_render_stall_has_ingress_progress: bool,
    liveness_reconnect_attempts_without_progress: u8,
    last_successful_media_edge_at_ms: Option<f64>,
    reconnect_success_edge_at_last_grant: Option<f64>,
    reconnect_grants_without_success_edge: u8,
    last_recovery_state: Option<RecoveryLedgerNarrativeState>,
    next_recovery_decision_ledger_id: u64,
    last_seen_decoder_reset_family_coalesce_deferred_count: u64,
}

impl RtcSessionPolicy {
    /// 仅用于 `latest_recovery_decision_ledger` headline 保留判定：与「会下发 TransportCommand」的
    /// `recovery_decision_ledger_has_pending_transport_command` 不同，localProbe 等叙事占位也要挡住覆盖。
    fn ledger_has_pending_command(ledger: &XbxEngineRecoveryDecisionLedgerObservation) -> bool {
        ledger.action_selected != "none" && ledger.command_result.is_none()
    }

    fn owner_state_is_steady_serving(owner_state: VideoSchedulingOwnerState) -> bool {
        matches!(
            owner_state,
            VideoSchedulingOwnerState::StableServing | VideoSchedulingOwnerState::DegradedServing
        )
    }

    pub fn new(
        runtime_config: Arc<Mutex<XbxEngineRuntimeConfig>>,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Self {
        let recovery_profile = resolve_recovery_profile(runtime_stats.as_ref());
        let stream_started_at = Instant::now();
        let last_recovery_epoch = RuntimeStatsSink::read_shared(runtime_stats.as_ref(), |stats| {
            stats.transport_recovery_epoch
        })
        .unwrap_or(0);
        Self {
            runtime_config,
            runtime_stats,
            scheduling_engine: SchedulingPolicyEngine::new(),
            scheduling_owner: VideoSchedulingOwner::new(),
            recovery_coordinator: RecoveryCoordinator::new(
                VideoEscalationController::new(recovery_profile.escalation_config()),
                stream_started_at,
                Duration::from_millis(RECOVERY_STARTUP_GRACE_MS),
            ),
            stream_started_at,
            escalation_profile_kind: recovery_profile.kind,
            last_recovery_epoch,
            last_bwe_sample_tick_ms: None,
            last_sent_remb_kbps: DEFAULT_BWE_TARGET_KBPS,
            hybrid_ramp_cooldown_ticks: 0,
            next_bwe_observation_id: 0,
            last_bwe_reason: None,
            unstable_hold_streak: 0,
            last_lifecycle_reconnect_proposal_at_ms: None,
            recovery_no_progress_since_ms: None,
            recovery_no_progress_last_frame_count: None,
            recovery_no_progress_last_transport_progress_token: None,
            failed_terminal_since_ms: None,
            failed_terminal_reason: None,
            failed_terminal_last_frame_count: None,
            connected_render_stall_since_ms: None,
            connected_render_last_present_time_ms: None,
            connected_render_last_inbound_video_bytes_total: None,
            connected_render_stall_has_ingress_progress: false,
            liveness_reconnect_attempts_without_progress: 0,
            last_successful_media_edge_at_ms: None,
            reconnect_success_edge_at_last_grant: None,
            reconnect_grants_without_success_edge: 0,
            last_recovery_state: None,
            next_recovery_decision_ledger_id: 0,
            last_seen_decoder_reset_family_coalesce_deferred_count: 0,
        }
    }
}

impl Default for RtcSessionPolicy {
    fn default() -> Self {
        Self::new(
            Arc::new(Mutex::new(XbxEngineRuntimeConfig::default())),
            Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default())),
        )
    }
}

impl SessionPolicyHook for RtcSessionPolicy {
    fn on_snapshot(&mut self, snapshot: &TransportSnapshot) -> Vec<SessionCommand> {
        self.refresh_escalation_profile();
        self.sync_recovery_epoch();
        self.sync_decoder_reset_family_coalesce_deferred_feedback(snapshot);
        self.refresh_successful_media_edge();
        let observed_at_ms = Self::resolve_policy_observed_at_ms(snapshot);
        let orchestration = build_rtc_session_policy_orchestration_input(
            snapshot,
            &self.runtime_stats,
            observed_at_ms,
            self.pre_first_frame_reconnect_fallback_ms(),
        );
        let owner_output = self.evaluate_scheduling_owner(snapshot, &orchestration);
        let recovery = self.build_recovery_proposal(
            snapshot,
            owner_output.state,
            owner_output.recovery_intent.as_ref(),
        );
        self.record_recovery_decision_ledger(snapshot, owner_output.state, recovery.as_ref());
        let reconnect_selected_by_recovery = recovery.as_ref().is_some_and(|proposal| {
            proposal.decision.action == RecoveryAction::RequestReconnectCandidate
        });
        let twcc_warmup_state = self.resolve_twcc_warmup_state();
        let bwe = if reconnect_selected_by_recovery || twcc_warmup_state.blocks_bwe_updates() {
            None
        } else {
            self.build_bwe_proposal(snapshot)
        };
        let bwe_observation_id = bwe
            .as_ref()
            .map(|proposal| proposal.evaluation.observation_id)
            .unwrap_or(0);

        let commands = self
            .scheduling_engine
            .plan(SchedulingPolicyInput {
                owner_state: owner_output.state,
                owner_health: owner_output.health,
                twcc_warmup_state,
                recovery,
                bwe,
            })
            .into_iter()
            .flat_map(|command| self.map_planned_command(command, bwe_observation_id))
            .collect::<Vec<_>>();
        // 临时诊断写在最后，确保 trace 里能看到 owner 这拍的收口失败摘要。
        if let Some(summary) = owner_output
            .diagnostics
            .temporary_diagnostic_summary
            .as_ref()
        {
            RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
                stats.latest_observation_label = Some("videoOwnerTempDiagnostic".to_string());
                stats.latest_observation_summary = Some(summary.clone());
            });
        }
        commands
    }
}

impl RtcSessionPolicy {
    /// transport bridge 上报 decoder reset 同族合并 defer，回滚 escalation 侧 burst 误计。
    fn sync_decoder_reset_family_coalesce_deferred_feedback(
        &mut self,
        snapshot: &TransportSnapshot,
    ) {
        let count = snapshot
            .recovery
            .decoder_reset_family_coalesce_deferred_count;
        if count <= self.last_seen_decoder_reset_family_coalesce_deferred_count {
            return;
        }
        let delta = count - self.last_seen_decoder_reset_family_coalesce_deferred_count;
        self.last_seen_decoder_reset_family_coalesce_deferred_count = count;
        for _ in 0..delta {
            self.recovery_coordinator
                .rollback_decoder_reset_burst_after_transport_family_defer();
        }
    }

    fn resolve_twcc_warmup_state(&self) -> TwccWarmupState {
        RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            if stats.session_target_type != Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud)
            {
                return TwccWarmupState::Inactive;
            }
            if stats
                .latest_video_twcc_observation
                .as_ref()
                .is_some_and(|observation| {
                    observation.source == "local-feedback" && observation.twcc_sample_valid
                })
            {
                return TwccWarmupState::LocalFeedbackReady;
            }

            let has_video_remote_twcc_binding = stats
                .latest_twcc_remote_stream_observation
                .as_ref()
                .is_some_and(|observation| {
                    observation
                        .mime_type
                        .get(..5)
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("video"))
                });
            let has_video_extension_signal = stats
                .latest_twcc_extension_observation
                .as_ref()
                .is_some_and(|observation| {
                    observation.state == "seen" || observation.state == "missing"
                });
            if has_video_remote_twcc_binding || has_video_extension_signal {
                return TwccWarmupState::MissingLocalFeedback;
            }
            if stats.latest_rtc_builder_observation.is_some() {
                return TwccWarmupState::BuilderConfigured;
            }
            TwccWarmupState::Inactive
        })
        .unwrap_or(TwccWarmupState::Inactive)
    }

    fn is_cloud_gaming_profile(&self) -> bool {
        self.escalation_profile_kind == ScenarioPolicyProfileKind::CloudGaming
    }

    fn lifecycle_reconnect_proposal_interval_ms(
        &self,
        snapshot: &TransportSnapshot,
        twcc_warmup_state: TwccWarmupState,
    ) -> f64 {
        if self.is_cloud_gaming_profile() && Self::is_pre_first_frame_connecting_surface(snapshot) {
            return CONNECTING_PRE_FIRST_FRAME_RECONNECT_PROPOSAL_INTERVAL_MS;
        }
        // 云侧 warmup 分阶段放宽 reconnect proposal 节流，避免 feedback 尚未 ready 时过早重连。
        if !self.is_cloud_gaming_profile() {
            return RECOVERING_RECONNECT_PROPOSAL_INTERVAL_MS;
        }
        match twcc_warmup_state {
            TwccWarmupState::BuilderConfigured => {
                CLOUD_BUILDER_CONFIGURED_RECONNECT_PROPOSAL_INTERVAL_MS
            }
            TwccWarmupState::MissingLocalFeedback => {
                CLOUD_MISSING_LOCAL_FEEDBACK_RECONNECT_PROPOSAL_INTERVAL_MS
            }
            TwccWarmupState::LocalFeedbackReady | TwccWarmupState::Inactive => {
                CLOUD_RECOVERING_RECONNECT_PROPOSAL_INTERVAL_MS
            }
        }
    }

    fn pre_first_frame_reconnect_fallback_ms(&self) -> f64 {
        resolve_recovery_profile(self.runtime_stats.as_ref())
            .pre_first_frame_reconnect_fallback_ms()
    }

    fn liveness_reconnect_attempt_limit(&self) -> u8 {
        // 云侧提高尝试上限，给高抖动链路更多恢复机会。
        if self.is_cloud_gaming_profile() {
            CLOUD_LIVENESS_RECONNECT_ATTEMPT_LIMIT
        } else {
            LIVENESS_RECONNECT_ATTEMPT_LIMIT
        }
    }

    #[allow(dead_code)]
    fn first_frame_grace_active(&self) -> bool {
        let first_frame_grace_ms = self
            .runtime_config
            .lock()
            .ok()
            .map(|config| config.webrtc.recovery.first_frame_grace_ms)
            .unwrap_or(RECOVERY_STARTUP_GRACE_MS);
        self.stream_started_at.elapsed() < Duration::from_millis(first_frame_grace_ms)
    }

    fn refresh_escalation_profile(&mut self) {
        let profile = resolve_recovery_profile(self.runtime_stats.as_ref());
        if profile.kind != self.escalation_profile_kind {
            self.recovery_coordinator = RecoveryCoordinator::new(
                VideoEscalationController::new(profile.escalation_config()),
                self.stream_started_at,
                Duration::from_millis(RECOVERY_STARTUP_GRACE_MS),
            );
            self.scheduling_engine = SchedulingPolicyEngine::new();
            self.scheduling_owner = VideoSchedulingOwner::new();
            self.escalation_profile_kind = profile.kind;
            self.last_lifecycle_reconnect_proposal_at_ms = None;
            self.recovery_no_progress_since_ms = None;
            self.recovery_no_progress_last_frame_count = None;
            self.recovery_no_progress_last_transport_progress_token = None;
            self.failed_terminal_since_ms = None;
            self.failed_terminal_reason = None;
            self.failed_terminal_last_frame_count = None;
            self.reset_connected_render_stall_liveness();
            self.liveness_reconnect_attempts_without_progress = 0;
            self.last_successful_media_edge_at_ms = None;
            self.reconnect_success_edge_at_last_grant = None;
            self.reconnect_grants_without_success_edge = 0;
            self.last_recovery_state = None;
            self.next_recovery_decision_ledger_id = 0;
            self.last_seen_decoder_reset_family_coalesce_deferred_count = 0;
        }
    }

    fn refresh_successful_media_edge(&mut self) {
        let latest_success_edge =
            RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
                [
                    stats.latest_video_decode_ok_time_ms,
                    stats.latest_video_host_present_time_ms,
                    stats.video_anchor_clean_observed_at_ms,
                ]
                .into_iter()
                .flatten()
                .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
            })
            .flatten();
        let Some(latest_success_edge) = latest_success_edge else {
            return;
        };
        let advanced = self
            .last_successful_media_edge_at_ms
            .is_none_or(|last| latest_success_edge > last);
        if advanced {
            self.last_successful_media_edge_at_ms = Some(latest_success_edge);
            self.reconnect_success_edge_at_last_grant = None;
            self.reconnect_grants_without_success_edge = 0;
        }
    }

    fn sync_recovery_epoch(&mut self) {
        let recovery_epoch = RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            stats.transport_recovery_epoch
        })
        .unwrap_or(0);
        if recovery_epoch == self.last_recovery_epoch {
            return;
        }
        // recovery epoch 变化只更新游标，coordinator 内部会在每次 proposal
        // 时通过 begin_recovery_epoch 切换预算；这里不重建对象，避免清空短时连击状态。
        self.last_recovery_epoch = recovery_epoch;
    }

    fn build_recovery_proposal(
        &mut self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        recovery_intent: Option<&RecoveryIntentContract>,
    ) -> Option<RecoveryPolicyProposal> {
        self.maybe_clear_failed_terminal(snapshot, owner_state);
        if self.failed_terminal_since_ms.is_some() {
            return None;
        }
        let observed_at_ms = Self::resolve_policy_observed_at_ms(snapshot);
        let recovery_observation =
            self.capture_recovery_observation_snapshot(snapshot, owner_state, observed_at_ms);
        let twcc_warmup_state = self.resolve_twcc_warmup_state();
        let has_media_recovery_surface = recovery_intent.is_some();
        let active_media_recovery_intent = recovery_intent.filter(|intent| intent.emit);
        let passive_anchor_recovery_intent = recovery_intent.filter(|intent| {
            Self::should_forward_passive_anchor_recovery_surface(owner_state, intent)
        });
        let lifecycle_disconnected =
            snapshot.connection.lifecycle_state == ConnectionLifecycleStateFact::Disconnected;
        let lifecycle_recovering =
            snapshot.connection.lifecycle_state == ConnectionLifecycleStateFact::Recovering;
        let recovering_connectivity_failure = lifecycle_recovering
            && snapshot.recovery.latest_diagnosis_label.as_deref()
                == Some("rtcConnectionRecovering")
            && self.has_connected_connectivity_failure_evidence(snapshot, observed_at_ms);
        let force_lifecycle_reconnect = recovering_connectivity_failure
            || (!has_media_recovery_surface
                && (lifecycle_disconnected
                    || self.should_force_liveness_reconnect(
                        snapshot,
                        owner_state,
                        observed_at_ms,
                    )));
        let block_lifecycle_reconnect_candidate =
            lifecycle_recovering && has_media_recovery_surface && !recovering_connectivity_failure;
        let allow_periodic_lifecycle_reconnect =
            lifecycle_recovering && snapshot.media.frame_count > 0 && !has_media_recovery_surface;
        let fallback_connectivity_reason = snapshot
            .recovery
            .latest_diagnosis_label
            .as_deref()
            .and_then(resolve_connectivity_fallback_reason);
        let owner_signal = if force_lifecycle_reconnect || allow_periodic_lifecycle_reconnect {
            let has_current_clean_anchor =
                RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
                    stats
                        .video_anchor_clean_epoch
                        .is_some_and(|epoch| epoch == stats.transport_recovery_epoch)
                        && stats.video_anchor_clean_source_event.as_deref()
                            == Some("chain-clean-keyframe-submitted")
                })
                .unwrap_or(false);
            RuntimeStatsSink::new(self.runtime_stats.clone())
                .complete_transport_recovery_for_lifecycle_recovering(observed_at_ms);
            if self.liveness_reconnect_attempts_without_progress
                >= self.liveness_reconnect_attempt_limit()
            {
                if self.should_enter_connecting_pre_first_frame_failed_terminal(
                    snapshot,
                    twcc_warmup_state,
                    observed_at_ms,
                ) {
                    self.mark_failed_terminal(snapshot, "livenessReconnectAttemptLimitExceeded");
                    return None;
                }
            }
            if !self.should_emit_lifecycle_reconnect(snapshot, observed_at_ms, twcc_warmup_state) {
                return None;
            }
            if !has_current_clean_anchor {
                // 只有在还没有 current clean anchor 时才推进 episode；
                // 否则会把刚到手的恢复成功证据过早轮转掉，owner 来不及收口到 stable-serving。
                RuntimeStatsSink::new(self.runtime_stats.clone())
                    .advance_transport_recovery_episode(observed_at_ms);
            }
            let reason_label = resolve_lifecycle_reconnect_reason_label(
                lifecycle_disconnected,
                recovering_connectivity_failure,
                force_lifecycle_reconnect,
            )
            .to_string();
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::LifecycleRecovering,
                reason_label,
                observed_at_ms,
            }
        } else {
            if let Some(reason) = fallback_connectivity_reason {
                RecoveryOwnerSignal {
                    reason,
                    reason_label: snapshot
                        .recovery
                        .latest_diagnosis_label
                        .clone()
                        .unwrap_or_else(|| reason.label().to_string()),
                    observed_at_ms,
                }
            } else if let Some(intent) =
                active_media_recovery_intent.or(passive_anchor_recovery_intent)
            {
                if self.should_hold_pre_first_frame_connected_idle_timeout(
                    snapshot,
                    owner_state,
                    intent.reason_label.as_str(),
                    observed_at_ms,
                ) {
                    return None;
                }
                if self.should_hold_pre_first_frame_display_supply_degraded(
                    snapshot,
                    owner_state,
                    intent.source,
                    intent.reason_label.as_str(),
                    observed_at_ms,
                ) {
                    return None;
                }
                RecoveryOwnerSignal {
                    reason: owner_recovery_reason_to_escalation_reason(intent.reason),
                    reason_label: intent.reason_label.clone(),
                    observed_at_ms,
                }
            } else if has_media_recovery_surface {
                return None;
            } else {
                let fallback_label = snapshot.recovery.latest_diagnosis_label.as_deref()?;
                if self.should_hold_pre_first_frame_connected_idle_timeout(
                    snapshot,
                    owner_state,
                    fallback_label,
                    observed_at_ms,
                ) {
                    return None;
                }
                if self.should_absorb_render_aware_realtime_adapter_idle_timeout(
                    snapshot,
                    owner_state,
                    fallback_label,
                    observed_at_ms,
                ) {
                    return None;
                }
                if self.should_absorb_stale_transport_await_replay(
                    snapshot,
                    owner_state,
                    fallback_label,
                    observed_at_ms,
                ) {
                    return None;
                }
                if self.should_suppress_adapter_idle_timeout_with_render_slack(
                    fallback_label,
                    observed_at_ms,
                ) {
                    return None;
                }
                let fallback_reason = map_label_to_escalation_reason(fallback_label)?;
                RecoveryOwnerSignal {
                    reason: fallback_reason,
                    reason_label: fallback_label.to_string(),
                    observed_at_ms,
                }
            }
        };
        // intent 路径可能把 transport-await 表面映射成 `WaitKeyframe` 等枚举，但仍携带同一诊断标签；
        // 吸收 stale replay 必须以 snapshot 诊断为准，而不能只看 `owner_signal.reason`。
        let stale_transport_await_diag = snapshot
            .recovery
            .latest_diagnosis_label
            .as_deref()
            == Some("transportAwaitRecoveryKeyframe");
        let stale_transport_await_absorb = stale_transport_await_diag
            && self.should_absorb_stale_transport_await_replay(
                snapshot,
                owner_state,
                owner_signal.reason_label.as_str(),
                observed_at_ms,
            );
        if stale_transport_await_absorb {
            return None;
        }
        let mut proposal = self
            .recovery_coordinator
            .propose_from_owner_signal(owner_signal, self.runtime_stats.as_ref());
        let expensive_recovery_gate = ExpensiveRecoveryGate::new(
            self.runtime_stats.as_ref(),
            self.is_cloud_gaming_profile(),
            self.reconnect_success_edge_at_last_grant,
            self.last_successful_media_edge_at_ms,
            self.reconnect_grants_without_success_edge,
        );
        if self.should_absorb_stale_transport_await_replay(
            snapshot,
            owner_state,
            proposal.signal.reason_label.as_str(),
            observed_at_ms,
        ) {
            return None;
        }
        if self.should_force_first_frame_acquisition_local_action(
            snapshot,
            &proposal,
            observed_at_ms,
        ) {
            proposal.decision.action =
                self.resolve_first_frame_acquisition_local_action(&proposal, observed_at_ms);
        }
        if proposal.decision.action == RecoveryAction::RequestKeyframe
            && first_frame_acquisition_priority_active(
                snapshot,
                observed_at_ms,
                self.runtime_stats.as_ref(),
                self.pre_first_frame_reconnect_fallback_ms(),
            )
            && self.has_recent_first_frame_keyframe_attempt(observed_at_ms)
        {
            if self.should_absorb_stale_transport_await_replay(
                snapshot,
                owner_state,
                proposal.signal.reason_label.as_str(),
                observed_at_ms,
            ) {
                return None;
            }
            proposal.decision.action = RecoveryAction::CoalescedKeyframeInFlight;
        }
        let reconnect_gate_detail = expensive_recovery_gate
            .apply_to_proposal(
                snapshot,
                owner_state,
                &mut proposal,
                observed_at_ms,
                twcc_warmup_state,
                block_lifecycle_reconnect_candidate,
            )
            .detail;
        expensive_recovery_gate.apply_rfc_decode_display_transport_ceiling(
            snapshot,
            observed_at_ms,
            self.recovery_no_progress_since_ms,
            RECOVERY_OBSERVATION_NO_PROGRESS_FALLBACK_MS,
            recovery_observation.local_self_healing_attempted(),
            recovery_observation.media_progress_is_stalled_long_enough(observed_at_ms),
            self.has_connected_connectivity_failure_evidence(snapshot, observed_at_ms),
            &mut proposal,
        );
        if self.should_absorb_supply_degraded_overlap_with_stale_transport_await(
            snapshot,
            owner_state,
            &proposal,
            observed_at_ms,
        ) {
            // 旧 transport-await 恢复窗尚未退干净时，新的 displaySupplyDegraded 容易只是本地显示断流表象。
            // 这时继续发 PLI 只会放大恢复链，先收敛在本地吸收路径。
            proposal.decision.action = RecoveryAction::CooldownSuppressed;
        }
        let transport_await_diagnosis_is_short = snapshot
            .recovery
            .last_observed_at_ms
            .is_some_and(|last| (observed_at_ms - last).max(0.0) <= 220.0);
        if snapshot.connection.lifecycle_state == ConnectionLifecycleStateFact::Connected
            && should_absorb_light_recovery_signal_during_ramp_up(
                self.runtime_stats.as_ref(),
                owner_state,
                &proposal,
                observed_at_ms,
                self.adapter_idle_render_slack_window_ms(),
                transport_await_diagnosis_is_short,
            )
        {
            // clean anchor 刚恢复后的爬升期内，局部 display/idle/短 transport-await 更多是恢复余波。
            // 这类轻信号只并入当前恢复轮次观察，不应立即重新升级动作。
            proposal.decision.action = RecoveryAction::CooldownSuppressed;
        }
        if proposal.signal.reason == VideoEscalationReason::LifecycleRecovering
            && proposal.decision.action == RecoveryAction::RequestReconnectCandidate
        {
            self.liveness_reconnect_attempts_without_progress = self
                .liveness_reconnect_attempts_without_progress
                .saturating_add(1);
        }
        if proposal.signal.reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && proposal.decision.action == RecoveryAction::RequestReconnectCandidate
            && !recovery_observation.allows_transport_await_reconnect_fallback(observed_at_ms)
        {
            proposal.decision.action = RecoveryAction::CooldownSuppressed;
        }
        if proposal.decision.action == RecoveryAction::RequestReconnectCandidate {
            self.record_reconnect_grant_without_success_edge();
        }
        if let Some(reason) =
            self.should_enter_failed_terminal(snapshot, owner_state, &proposal, observed_at_ms)
        {
            self.mark_failed_terminal(snapshot, reason);
            // failed-terminal 一旦在本拍成立，就不能继续把旧 proposal 下发到 planner/command。
            // 否则 ledger 会记成 terminal，但执行层仍收到 reconnect，造成合同分裂。
            return None;
        }
        Some(
            RecoveryPolicyProposal {
                decision: proposal.decision,
                reason: proposal.signal.reason,
                reason_label: proposal.signal.reason_label,
                reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
                reconnect_gate_detail,
                budget_before: proposal.budget_before,
                budget_after: proposal.budget_after,
            }
            .with_runtime_reason_domain(),
        )
    }

    fn should_absorb_render_aware_realtime_adapter_idle_timeout(
        &self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        diagnosis_label: &str,
        observed_at_ms: f64,
    ) -> bool {
        const REALTIME_DIAGNOSIS_MAX_AGE_MS: f64 = 160.0;
        if diagnosis_label != "adapterIdleTimeout" {
            return false;
        }
        if snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Connected
            || owner_state != VideoSchedulingOwnerState::StableServing
        {
            return false;
        }
        let diagnosis_is_realtime = snapshot
            .recovery
            .last_observed_at_ms
            .is_some_and(|last| (observed_at_ms - last).max(0.0) <= REALTIME_DIAGNOSIS_MAX_AGE_MS);
        if !diagnosis_is_realtime {
            return false;
        }
        RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            let current_clean_anchor = stats
                .video_anchor_clean_epoch
                .is_some_and(|epoch| epoch == stats.transport_recovery_epoch)
                && stats.video_anchor_clean_source_event.as_deref()
                    == Some("chain-clean-keyframe-submitted");
            let pipeline_not_stalled = !stats.video_decoder_stalled.unwrap_or(false)
                && !stats.video_renderer_stalled.unwrap_or(false);
            let render_has_headroom = !matches!(
                stats.host_no_pending_pressure_level.as_deref(),
                Some("high" | "critical")
            );
            pipeline_not_stalled
                && render_has_headroom
                && current_clean_anchor
                && has_fresh_media_output(stats, observed_at_ms)
        })
        .unwrap_or(false)
    }

    /// 仅服务于 `transportAwaitRecoveryKeyframe` → reconnect fallback 门控：keyframe/decoder 证据必须与该诊断链一致。
    fn capture_recovery_observation_snapshot(
        &self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        observed_at_ms: f64,
    ) -> RecoveryObservationSnapshot {
        RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            let (last_keyframe_requested_at, last_keyframe_decoded_at) =
                transport_await_keyframe_episode_latest_times(stats);
            RecoveryObservationSnapshot {
                ingress_active: stats.latest_video_packet_arrival_time_ms.is_some(),
                reassembly_active: stats.latest_video_packet_sequence.is_some(),
                decode_active: stats.latest_video_decode_ok_time_ms.is_some(),
                render_active: stats.latest_video_host_present_time_ms.is_some(),
                rtc_connectivity_connected: snapshot.connection.lifecycle_state
                    == ConnectionLifecycleStateFact::Connected,
                reconnect_in_flight: matches!(
                    snapshot.connection.lifecycle_state,
                    ConnectionLifecycleStateFact::Connecting
                        | ConnectionLifecycleStateFact::Recovering
                ),
                stable_serving: owner_state == VideoSchedulingOwnerState::StableServing,
                last_media_progress_at: [
                    stats.latest_video_packet_arrival_time_ms,
                    stats.latest_video_decode_ok_time_ms,
                    stats.latest_video_host_present_time_ms,
                ]
                .into_iter()
                .flatten()
                .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal)),
                last_video_decode_ok_at: stats.latest_video_decode_ok_time_ms,
                last_keyframe_requested_at,
                last_keyframe_decoded_at,
                local_decoder_reset_count_in_window:
                    Self::count_recent_transport_await_decoder_resets(
                        stats,
                        observed_at_ms,
                        RECOVERY_OBSERVATION_WINDOW_MS,
                    ),
                keyframe_request_count_in_window:
                    Self::count_recent_transport_await_keyframe_requests(
                        stats,
                        observed_at_ms,
                        RECOVERY_OBSERVATION_WINDOW_MS,
                    ),
            }
        })
        .unwrap_or_else(|| RecoveryObservationSnapshot {
            ingress_active: false,
            reassembly_active: false,
            decode_active: false,
            render_active: false,
            rtc_connectivity_connected: false,
            reconnect_in_flight: false,
            stable_serving: false,
            last_media_progress_at: None,
            last_video_decode_ok_at: None,
            last_keyframe_requested_at: None,
            last_keyframe_decoded_at: None,
            local_decoder_reset_count_in_window: 0,
            keyframe_request_count_in_window: 0,
        })
    }

    fn count_recent_transport_await_decoder_resets(
        stats: &crate::XbxEngineMediaRuntimeStats,
        observed_at_ms: f64,
        window_ms: f64,
    ) -> u32 {
        let current_epoch = stats.transport_recovery_epoch;
        stats
            .recent_recovery_decision_ledgers
            .iter()
            .filter(|ledger| {
                ledger.action_selected == "requestDecoderReset"
                    && ledger_input_signals_transport_await_recovery_keyframe(ledger)
                    && (observed_at_ms - ledger.observed_at_ms).max(0.0) <= window_ms
                    && recovery_decision_ledger_recovery_epoch(ledger) == Some(current_epoch)
            })
            .count() as u32
    }

    fn count_recent_transport_await_keyframe_requests(
        stats: &crate::XbxEngineMediaRuntimeStats,
        observed_at_ms: f64,
        window_ms: f64,
    ) -> u32 {
        let epoch_floor_ms = recovery_observation_epoch_floor_ms(stats);
        let mut seen_episode_ids = HashSet::new();
        let mut count = 0u32;
        for episode in stats
            .recent_keyframe_request_episodes
            .iter()
            .chain(stats.latest_keyframe_request_episode.iter())
        {
            if !is_transport_await_keyframe_episode(episode) {
                continue;
            }
            if episode.requested_at_ms < epoch_floor_ms {
                continue;
            }
            if (observed_at_ms - episode.requested_at_ms).max(0.0) > window_ms {
                continue;
            }
            if seen_episode_ids.insert(episode.episode_id) {
                count = count.saturating_add(1);
            }
        }
        count
    }

    fn should_absorb_stale_transport_await_replay(
        &self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        _diagnosis_label: &str,
        observed_at_ms: f64,
    ) -> bool {
        const STALE_TRANSPORT_AWAIT_REPLAY_MAX_AGE_MS: f64 = 220.0;
        if snapshot.recovery.latest_diagnosis_label.as_deref()
            != Some("transportAwaitRecoveryKeyframe")
        {
            return false;
        }
        if snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Connected {
            return false;
        }
        RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            let current_clean_anchor = stats
                .video_anchor_clean_epoch
                .is_some_and(|epoch| epoch == stats.transport_recovery_epoch)
                && stats.video_anchor_clean_source_event.as_deref()
                    == Some("chain-clean-keyframe-submitted");
            let clean_anchor_pipeline_evidence = stats.video_anchor_clean_source_event.as_deref()
                == Some("chain-clean-keyframe-submitted")
                && stats.video_anchor_clean_observed_at_ms.is_some();
            let terminal_deferred_transport_await =
                Self::has_terminal_deferred_transport_await_issue(
                    stats,
                    current_clean_anchor,
                    observed_at_ms,
                );
            let transport_await_unresolved = Self::has_unresolved_transport_await_issue(stats)
                && !terminal_deferred_transport_await;
            let chain_healthy = stats
                .latest_video_timeline_observation
                .as_ref()
                .is_some_and(|timeline| timeline.chain.state == "healthy");
            let decode_age_ms = stats
                .latest_video_decode_ok_time_ms
                .map(|at_ms| (observed_at_ms - at_ms).max(0.0));
            let present_age_ms = stats
                .latest_video_host_present_time_ms
                .map(|at_ms| (observed_at_ms - at_ms).max(0.0));
            let (track_state, track_video_bytes_total) = stats
                .latest_video_track_status
                .as_ref()
                .map(|track| (Some(track.state.as_str()), Some(track.video_bytes_total)))
                .unwrap_or((None, None));
            let healthy_media_baseline = is_media_healthy_baseline(
                true,
                chain_healthy,
                track_state,
                track_video_bytes_total,
                decode_age_ms,
                present_age_ms,
                500.0,
                500.0,
                stats.video_decoder_stalled.unwrap_or(false),
                stats.video_renderer_stalled.unwrap_or(false),
            ) && has_fresh_media_output(stats, observed_at_ms);
            let track_attached_with_video =
                stats
                    .latest_video_track_status
                    .as_ref()
                    .is_some_and(|track| {
                        track.state == "remoteTrackAttached" && track.video_bytes_total > 0
                    });
            let pipeline_not_stalled = !stats.video_decoder_stalled.unwrap_or(false)
                && !stats.video_renderer_stalled.unwrap_or(false);
            let diagnosis_is_stale = snapshot.recovery.last_observed_at_ms.is_some_and(|last| {
                (observed_at_ms - last).max(0.0) > STALE_TRANSPORT_AWAIT_REPLAY_MAX_AGE_MS
            });
            // 本函数仅在 `transportAwaitRecoveryKeyframe` 重放入口调用：诊断时间轴足够陈旧且
            // 已有当前 clean anchor 与可服务轨道时，吸收重放，不依赖可能已被首轮 tick 清掉的
            // H264 检验或 recovering timeline。
            let stale_diagnosis_replay_absorbed = diagnosis_is_stale
                && clean_anchor_pipeline_evidence
                && track_attached_with_video
                && pipeline_not_stalled;
            let stale_steady_session_replay_absorbed = diagnosis_is_stale
                && track_attached_with_video
                && pipeline_not_stalled
                && snapshot.media.frame_count > 0
                && stats.session_phase.as_deref() == Some("steady");
            let recovered_hard_gate =
                (!transport_await_unresolved && current_clean_anchor && healthy_media_baseline)
                    || (terminal_deferred_transport_await
                        && current_clean_anchor
                        && track_attached_with_video
                        && pipeline_not_stalled)
                    || stale_diagnosis_replay_absorbed
                    || stale_steady_session_replay_absorbed;
            if Self::owner_state_is_steady_serving(owner_state) && recovered_hard_gate {
                return true;
            }
            diagnosis_is_stale && recovered_hard_gate
        })
        .unwrap_or(false)
    }

    /// transport-await 未决口径与 `recovery::contract` 一致；gap 严重度推导见
    /// `derive_gap_severity_from_timeline_observation` / `derive_gap_severity_with_episode_stall`。
    fn has_unresolved_transport_await_issue(stats: &crate::XbxEngineMediaRuntimeStats) -> bool {
        let timeline = match stats.latest_video_timeline_observation.as_ref() {
            Some(timeline) => timeline,
            None => return false,
        };
        has_current_transport_await_issue_from_observation(
            timeline,
            current_clean_anchor_observed_at_ms(
                stats.video_anchor_clean_epoch,
                stats.video_anchor_clean_observed_at_ms,
                stats.video_anchor_clean_source_event.as_deref(),
                stats.transport_recovery_epoch,
            ),
        )
    }

    fn has_terminal_deferred_transport_await_issue(
        stats: &crate::XbxEngineMediaRuntimeStats,
        has_clean_anchor_evidence: bool,
        observed_at_ms: f64,
    ) -> bool {
        stats
            .latest_keyframe_request_episode
            .as_ref()
            .is_some_and(|episode| {
                is_terminal_transport_await_deferred_episode(
                    episode,
                    stats.latest_h264_inspection_observation.as_ref(),
                    has_clean_anchor_evidence,
                    observed_at_ms,
                    220.0,
                )
            })
    }

    fn should_hold_pre_first_frame_connected_idle_timeout(
        &self,
        snapshot: &TransportSnapshot,
        _owner_state: VideoSchedulingOwnerState,
        diagnosis_label: &str,
        observed_at_ms: f64,
    ) -> bool {
        should_hold_pre_first_frame_connected_idle_timeout(
            snapshot,
            diagnosis_label,
            observed_at_ms,
            self.runtime_stats.as_ref(),
            self.pre_first_frame_reconnect_fallback_ms(),
        )
    }

    fn should_hold_pre_first_frame_display_supply_degraded(
        &self,
        snapshot: &TransportSnapshot,
        _owner_state: VideoSchedulingOwnerState,
        source: RecoveryIntentSource,
        reason_label: &str,
        observed_at_ms: f64,
    ) -> bool {
        should_hold_pre_first_frame_display_supply_degraded(
            snapshot,
            source,
            reason_label,
            observed_at_ms,
            self.runtime_stats.as_ref(),
            self.pre_first_frame_reconnect_fallback_ms(),
        )
    }

    fn should_absorb_supply_degraded_overlap_with_stale_transport_await(
        &self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        proposal: &RecoveryCoordinatorProposal,
        observed_at_ms: f64,
    ) -> bool {
        const STALE_TRANSPORT_AWAIT_OVERLAP_MAX_AGE_MS: f64 = 1_800.0;
        const FRESH_INGRESS_MAX_AGE_MS: f64 = 220.0;
        if self.is_cloud_gaming_profile()
            || snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Connected
            || !matches!(
                owner_state,
                VideoSchedulingOwnerState::SupplyStarved
                    | VideoSchedulingOwnerState::DegradedServing
                    | VideoSchedulingOwnerState::RebuildingSupply,
            )
            || proposal.signal.reason != VideoEscalationReason::AdapterThinStream
            || proposal.signal.reason_label != "displaySupplyDegraded"
            || proposal.decision.action != RecoveryAction::RequestKeyframe
        {
            return false;
        }
        RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            let current_clean_anchor = stats
                .video_anchor_clean_epoch
                .is_some_and(|epoch| epoch == stats.transport_recovery_epoch)
                && stats.video_anchor_clean_source_event.as_deref()
                    == Some("chain-clean-keyframe-submitted");
            let terminal_deferred_transport_await =
                Self::has_terminal_deferred_transport_await_issue(
                    stats,
                    current_clean_anchor,
                    observed_at_ms,
                );
            let current_transport_await_issue = Self::has_unresolved_transport_await_issue(stats);
            let recent_transport_await_episode_window = stats
                .latest_keyframe_request_episode
                .as_ref()
                .is_some_and(|episode| {
                    episode.request_reason.as_deref() == Some("transportAwaitRecoveryKeyframe")
                        && episode.status != "decoded"
                        && (observed_at_ms - episode.requested_at_ms).max(0.0)
                            <= STALE_TRANSPORT_AWAIT_OVERLAP_MAX_AGE_MS
                });
            let overlapping_transport_await = stats
                .latest_keyframe_request_episode
                .as_ref()
                .is_some_and(|episode| {
                    current_transport_await_issue
                        && episode.request_reason.as_deref()
                            == Some("transportAwaitRecoveryKeyframe")
                        && !terminal_deferred_transport_await
                        && episode.status != "decoded"
                        && (observed_at_ms - episode.requested_at_ms).max(0.0)
                            <= STALE_TRANSPORT_AWAIT_OVERLAP_MAX_AGE_MS
                })
                || current_transport_await_issue
                || terminal_deferred_transport_await
                || recent_transport_await_episode_window;
            if !overlapping_transport_await {
                return false;
            }
            let chain_healthy = stats
                .latest_video_timeline_observation
                .as_ref()
                .is_some_and(|timeline| timeline.chain.state == "healthy");
            let track_attached_with_video =
                stats
                    .latest_video_track_status
                    .as_ref()
                    .is_some_and(|track| {
                        track.state == "remoteTrackAttached" && track.video_bytes_total > 0
                    });
            let ingress_is_fresh = stats
                .latest_video_packet_arrival_time_ms
                .is_some_and(|at_ms| (observed_at_ms - at_ms).max(0.0) <= FRESH_INGRESS_MAX_AGE_MS)
                && stats.latest_video_decode_ok_time_ms.is_some_and(|at_ms| {
                    (observed_at_ms - at_ms).max(0.0) <= FRESH_INGRESS_MAX_AGE_MS
                });
            let pipeline_not_stalled = !stats.video_decoder_stalled.unwrap_or(false)
                && !stats.video_renderer_stalled.unwrap_or(false);
            chain_healthy && track_attached_with_video && ingress_is_fresh && pipeline_not_stalled
        })
        .unwrap_or(false)
    }

    fn should_suppress_adapter_idle_timeout_with_render_slack(
        &self,
        diagnosis_label: &str,
        observed_at_ms: f64,
    ) -> bool {
        if diagnosis_label != "adapterIdleTimeout" {
            return false;
        }
        let slack_window_ms = self.adapter_idle_render_slack_window_ms();
        RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            if stats.transport_state != xbxengine_protocol::XbxEngineTransportStateDto::Connected {
                return false;
            }
            let current_clean_anchor = stats
                .video_anchor_clean_epoch
                .is_some_and(|epoch| epoch == stats.transport_recovery_epoch)
                && stats.video_anchor_clean_source_event.as_deref()
                    == Some("chain-clean-keyframe-submitted");
            let pipeline_not_stalled = !stats.video_decoder_stalled.unwrap_or(false)
                && !stats.video_renderer_stalled.unwrap_or(false);
            let present_fresh = stats
                .latest_video_host_present_time_ms
                .is_some_and(|at_ms| (observed_at_ms - at_ms).max(0.0) <= slack_window_ms);
            let decode_fresh = stats
                .latest_video_decode_ok_time_ms
                .is_some_and(|at_ms| (observed_at_ms - at_ms).max(0.0) <= slack_window_ms);
            pipeline_not_stalled && current_clean_anchor && (present_fresh || decode_fresh)
        })
        .unwrap_or(false)
    }

    fn adapter_idle_render_slack_window_ms(&self) -> f64 {
        let configured_idle_timeout_ms = self
            .runtime_config
            .lock()
            .ok()
            .map(|config| config.webrtc.video_pipeline.idle_timeout_ms.max(120))
            .unwrap_or(150) as f64;
        (configured_idle_timeout_ms * 1.5)
            .max(ADAPTER_IDLE_RENDER_SLACK_MIN_MS)
            .min(ADAPTER_IDLE_RENDER_SLACK_MAX_MS)
    }

    fn record_reconnect_grant_without_success_edge(&mut self) {
        self.reconnect_success_edge_at_last_grant = self.last_successful_media_edge_at_ms;
        self.reconnect_grants_without_success_edge =
            self.reconnect_grants_without_success_edge.saturating_add(1);
    }

    #[allow(dead_code)]
    fn media_reconnect_block_reason(
        &self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        proposal: &RecoveryCoordinatorProposal,
        observed_at_ms: f64,
    ) -> Option<&'static str> {
        ExpensiveRecoveryGate::new(
            self.runtime_stats.as_ref(),
            self.is_cloud_gaming_profile(),
            self.reconnect_success_edge_at_last_grant,
            self.last_successful_media_edge_at_ms,
            self.reconnect_grants_without_success_edge,
        )
        .media_reconnect_block_reason(snapshot, owner_state, proposal, observed_at_ms)
    }

    fn maybe_clear_failed_terminal(
        &mut self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
    ) {
        if self.failed_terminal_since_ms.is_none() {
            return;
        }
        let recovered = snapshot.connection.lifecycle_state
            == ConnectionLifecycleStateFact::Connected
            && Self::owner_state_is_steady_serving(owner_state);
        let progressed = self
            .failed_terminal_last_frame_count
            .is_some_and(|last| snapshot.media.frame_count > last);
        if recovered || progressed {
            self.failed_terminal_since_ms = None;
            self.failed_terminal_reason = None;
            self.failed_terminal_last_frame_count = None;
            self.liveness_reconnect_attempts_without_progress = 0;
            self.reconnect_success_edge_at_last_grant = None;
            self.reconnect_grants_without_success_edge = 0;
        }
    }

    fn should_forward_passive_anchor_recovery_surface(
        owner_state: VideoSchedulingOwnerState,
        intent: &RecoveryIntentContract,
    ) -> bool {
        !intent.emit
            && owner_state == VideoSchedulingOwnerState::RebuildingSupply
            && intent.source == RecoveryIntentSource::Anchor
    }

    fn should_enter_connected_ingress_without_success_output_failed_terminal(
        &self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        observed_at_ms: f64,
    ) -> bool {
        if snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Connected
            || Self::owner_state_is_steady_serving(owner_state)
        {
            return false;
        }
        let reconnect_attempt_limit = self.liveness_reconnect_attempt_limit();
        RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            let ingress_recent = stats.inbound_primary_video_bytes_total > 0
                && stats
                    .latest_video_packet_arrival_time_ms
                    .is_some_and(|at_ms| (observed_at_ms - at_ms).max(0.0) <= 1_500.0);
            if !ingress_recent {
                return false;
            }
            let latest_success_output_at_ms = [
                stats.latest_video_decode_ok_time_ms,
                stats.latest_video_host_present_time_ms,
                stats.video_anchor_clean_observed_at_ms,
            ]
            .into_iter()
            .flatten()
            .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
            let no_success_output_long_enough = latest_success_output_at_ms
                .map(|at_ms| (observed_at_ms - at_ms).max(0.0))
                .unwrap_or(f64::INFINITY)
                >= CONNECTED_INGRESS_WITHOUT_SUCCESS_OUTPUT_FAILED_TERMINAL_MS;
            let hard_recovery_already_exhausted =
                self.reconnect_grants_without_success_edge >= reconnect_attempt_limit;
            no_success_output_long_enough && hard_recovery_already_exhausted
        })
        .unwrap_or(false)
    }

    fn should_enter_failed_terminal(
        &self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        proposal: &crate::transport::rtc::recovery::coordinator::RecoveryCoordinatorProposal,
        observed_at_ms: f64,
    ) -> Option<&'static str> {
        if proposal.signal.reason == VideoEscalationReason::LifecycleRecovering
            && proposal.decision.action == RecoveryAction::CooldownSuppressed
            && proposal.budget_after.reconnect_budget_used
                >= proposal.budget_after.reconnect_budget_limit
        {
            return Some("reconnectBudgetExhausted");
        }
        if self.should_enter_connected_ingress_without_success_output_failed_terminal(
            snapshot,
            owner_state,
            observed_at_ms,
        ) {
            return Some("connectedIngressWithoutSuccessfulOutput");
        }
        None
    }

    fn mark_failed_terminal(&mut self, snapshot: &TransportSnapshot, reason: &str) {
        if self.failed_terminal_since_ms.is_none() {
            self.failed_terminal_since_ms = Some(Self::resolve_policy_observed_at_ms(snapshot));
        }
        self.failed_terminal_reason = Some(reason.to_string());
        self.failed_terminal_last_frame_count = Some(snapshot.media.frame_count);
    }

    fn should_emit_lifecycle_reconnect(
        &mut self,
        snapshot: &TransportSnapshot,
        observed_at_ms: f64,
        twcc_warmup_state: TwccWarmupState,
    ) -> bool {
        let proposal_interval_ms =
            self.lifecycle_reconnect_proposal_interval_ms(snapshot, twcc_warmup_state);
        if self
            .last_lifecycle_reconnect_proposal_at_ms
            .is_some_and(|last| (observed_at_ms - last).max(0.0) < proposal_interval_ms)
        {
            return false;
        }
        self.last_lifecycle_reconnect_proposal_at_ms = Some(observed_at_ms);
        true
    }

    fn should_enter_connecting_pre_first_frame_failed_terminal(
        &self,
        snapshot: &TransportSnapshot,
        twcc_warmup_state: TwccWarmupState,
        observed_at_ms: f64,
    ) -> bool {
        if !self.should_soft_hold_early_connecting_failed_terminal(snapshot, twcc_warmup_state) {
            return true;
        }
        self.recovery_no_progress_since_ms
            .is_some_and(|stalled_since| {
                (observed_at_ms - stalled_since).max(0.0)
                    >= CONNECTING_PRE_FIRST_FRAME_FAILED_TERMINAL_MIN_MS
            })
    }

    fn should_soft_hold_early_connecting_failed_terminal(
        &self,
        snapshot: &TransportSnapshot,
        twcc_warmup_state: TwccWarmupState,
    ) -> bool {
        if snapshot.media.frame_count != 0 {
            return false;
        }
        let in_pre_first_frame_surface = matches!(
            snapshot.connection.lifecycle_state,
            ConnectionLifecycleStateFact::New
                | ConnectionLifecycleStateFact::Connecting
                | ConnectionLifecycleStateFact::Recovering
        );
        if !in_pre_first_frame_surface {
            return false;
        }
        if self.is_cloud_gaming_profile() {
            return true;
        }
        let session_target_type = self.read_session_target_type();
        // 首窗 target_type 尚未判定时，按 cloud 同级长窗口处理，避免在 session
        // 还没进入 Provisioned 前被短预算误判为 failed-terminal。
        if session_target_type.is_none() {
            return true;
        }
        if matches!(
            session_target_type,
            Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home)
        ) {
            // home 也保留同样的首帧前软保持窗口，避免仅凭
            // selected pair / ICE 进展但尚未真正出帧时过早收口。
            return true;
        }
        // warmup 已建立后的首帧前路径继续沿用同一条 soft hold，避免 pass11/pass12/pass13 分叉。
        twcc_warmup_state.blocks_bwe_updates()
            && matches!(
                snapshot.connection.lifecycle_state,
                ConnectionLifecycleStateFact::New
            )
    }

    fn read_session_target_type(&self) -> Option<xbxengine_protocol::XbxEngineTargetTypeDto> {
        RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            stats.session_target_type.clone()
        })
        .flatten()
    }

    fn is_pre_first_frame_connecting_surface(snapshot: &TransportSnapshot) -> bool {
        snapshot.media.frame_count == 0
            && matches!(
                snapshot.connection.lifecycle_state,
                ConnectionLifecycleStateFact::New | ConnectionLifecycleStateFact::Connecting
            )
    }

    fn should_force_liveness_reconnect(
        &mut self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        observed_at_ms: f64,
    ) -> bool {
        let lifecycle_state = snapshot.connection.lifecycle_state;
        let connected_non_stable = lifecycle_state == ConnectionLifecycleStateFact::Connected
            && !Self::owner_state_is_steady_serving(owner_state);
        let in_recovery_surface = matches!(
            lifecycle_state,
            ConnectionLifecycleStateFact::Connecting | ConnectionLifecycleStateFact::Recovering
        ) || (connected_non_stable
            && self.has_connected_connectivity_failure_evidence(snapshot, observed_at_ms));
        if !in_recovery_surface {
            self.recovery_no_progress_since_ms = None;
            self.recovery_no_progress_last_frame_count = None;
            self.recovery_no_progress_last_transport_progress_token = None;
            self.reset_connected_render_stall_liveness();
            self.liveness_reconnect_attempts_without_progress = 0;
            self.reconnect_success_edge_at_last_grant = None;
            self.reconnect_grants_without_success_edge = 0;
            return false;
        }
        if self.should_force_connected_render_stall_reconnect(snapshot, owner_state, observed_at_ms)
        {
            return true;
        }
        // 仅把“真实媒体前进”视为进展，避免命令成功（例如 reconnect staged/consumed）
        // 持续重置 no-progress 计时，导致 Connecting + 无帧时无法进入终止态。
        let current_frame_count = snapshot.media.frame_count;
        match self.recovery_no_progress_last_frame_count {
            None => {
                self.recovery_no_progress_last_frame_count = Some(current_frame_count);
                self.recovery_no_progress_since_ms = Some(observed_at_ms);
                self.recovery_no_progress_last_transport_progress_token =
                    Some(Self::build_transport_progress_token(snapshot));
                self.liveness_reconnect_attempts_without_progress = 0;
                return false;
            }
            Some(last_frame_count) if current_frame_count > last_frame_count => {
                self.recovery_no_progress_last_frame_count = Some(current_frame_count);
                self.recovery_no_progress_since_ms = Some(observed_at_ms);
                self.recovery_no_progress_last_transport_progress_token =
                    Some(Self::build_transport_progress_token(snapshot));
                self.liveness_reconnect_attempts_without_progress = 0;
                return false;
            }
            _ => {}
        }

        // 方案 B：首帧前把“传输里程碑”也视为进展。
        //
        // 目的：避免 ICE/通道/RTT 等已经在推进，但 frame_count 仍为 0 时被误判为“无进展”，
        // 从而触发重连风暴。里程碑一旦稳定（token 不再变化），仍会在阈值后进入重连。
        if current_frame_count == 0 {
            let token = Self::build_transport_progress_token(snapshot);
            match self.recovery_no_progress_last_transport_progress_token {
                None => {
                    self.recovery_no_progress_last_transport_progress_token = Some(token);
                    self.recovery_no_progress_since_ms = Some(observed_at_ms);
                    self.liveness_reconnect_attempts_without_progress = 0;
                    return false;
                }
                Some(last_token) if token != last_token => {
                    self.recovery_no_progress_last_transport_progress_token = Some(token);
                    self.recovery_no_progress_since_ms = Some(observed_at_ms);
                    self.liveness_reconnect_attempts_without_progress = 0;
                    return false;
                }
                _ => {}
            }
        }

        // 首帧前统一走保守阈值，避免“无明显 transport 进展”被 4s 上界过早误杀。
        let fallback_threshold_ms = if snapshot.media.frame_count == 0 {
            self.pre_first_frame_reconnect_fallback_ms()
        } else {
            RECOVERY_NO_PROGRESS_RECONNECT_FALLBACK_MS
        };
        let stalled_since = self
            .recovery_no_progress_since_ms
            .get_or_insert(observed_at_ms);
        observed_at_ms - *stalled_since >= fallback_threshold_ms
    }

    fn should_force_connected_render_stall_reconnect(
        &mut self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        observed_at_ms: f64,
    ) -> bool {
        if snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Connected
            || Self::owner_state_is_steady_serving(owner_state)
        {
            self.reset_connected_render_stall_liveness();
            return false;
        }
        if !self.has_connected_connectivity_failure_evidence(snapshot, observed_at_ms) {
            return false;
        }
        let signal = self.read_connected_render_liveness_signal();
        let present_progressed = match (
            self.connected_render_last_present_time_ms,
            signal.latest_video_host_present_time_ms,
        ) {
            (Some(last), Some(current)) => current > last,
            _ => false,
        };
        let ingress_progressed = self
            .connected_render_last_inbound_video_bytes_total
            .is_some_and(|last| signal.inbound_primary_video_bytes_total > last);
        self.connected_render_last_present_time_ms = signal.latest_video_host_present_time_ms;
        self.connected_render_last_inbound_video_bytes_total =
            Some(signal.inbound_primary_video_bytes_total);
        if present_progressed {
            self.connected_render_stall_since_ms = None;
            self.connected_render_stall_has_ingress_progress = false;
            return false;
        }
        let present_age_ms = signal
            .latest_video_host_present_time_ms
            .map(|ts| (observed_at_ms - ts).max(0.0));
        let present_stale =
            present_age_ms.is_some_and(|age| age >= CONNECTED_PRESENT_STALL_MIN_AGE_MS);
        let pressure_or_hard_stale = signal.no_pending_pressure_is_high
            || present_age_ms.is_some_and(|age| age >= CONNECTED_PRESENT_STALL_HARD_AGE_MS);
        if !present_stale || !pressure_or_hard_stale {
            self.connected_render_stall_since_ms = None;
            self.connected_render_stall_has_ingress_progress = false;
            return false;
        }
        if self.connected_render_stall_since_ms.is_none() {
            self.connected_render_stall_since_ms = Some(observed_at_ms);
            self.connected_render_stall_has_ingress_progress = ingress_progressed;
            return false;
        }
        if ingress_progressed {
            self.connected_render_stall_has_ingress_progress = true;
        }
        if !self.connected_render_stall_has_ingress_progress {
            return false;
        }
        self.connected_render_stall_since_ms
            .is_some_and(|stalled_since| {
                (observed_at_ms - stalled_since).max(0.0)
                    >= CONNECTED_PRESENT_STALL_RECONNECT_FALLBACK_MS
            })
    }

    fn read_connected_render_liveness_signal(&self) -> ConnectedRenderLivenessSignal {
        RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            ConnectedRenderLivenessSignal {
                latest_video_host_present_time_ms: stats.latest_video_host_present_time_ms,
                inbound_primary_video_bytes_total: stats.inbound_primary_video_bytes_total,
                no_pending_pressure_is_high: stats
                    .host_no_pending_pressure_level
                    .as_deref()
                    .is_some_and(|level| matches!(level, "high" | "critical")),
            }
        })
        .unwrap_or_default()
    }

    fn reset_connected_render_stall_liveness(&mut self) {
        self.connected_render_stall_since_ms = None;
        self.connected_render_last_present_time_ms = None;
        self.connected_render_last_inbound_video_bytes_total = None;
        self.connected_render_stall_has_ingress_progress = false;
    }

    fn has_connected_connectivity_failure_evidence(
        &self,
        snapshot: &TransportSnapshot,
        observed_at_ms: f64,
    ) -> bool {
        if snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Connected {
            return true;
        }
        let has_data_channel = snapshot.connection.control_channel_open
            || snapshot.connection.message_channel_open
            || snapshot.connection.input_channel_open
            || snapshot.connection.chat_channel_open;
        let has_transport_signal = snapshot.connection.latest_transport_path.is_some()
            || snapshot.connection.latest_rtt_ms.is_some();
        let connection_signal_stale = snapshot.connection.last_observed_at_ms.is_none_or(|last| {
            (observed_at_ms - last).max(0.0) >= CONNECTED_CONNECTIVITY_EVIDENCE_STALE_MS
        });
        !has_data_channel && !has_transport_signal && connection_signal_stale
    }

    fn build_transport_progress_token(snapshot: &TransportSnapshot) -> u64 {
        // 一个单调稳定的“里程碑 bitset”。token 的目标不是精确描述状态，
        // 而是在关键里程碑发生变化时能检测到“确实在前进”。
        let mut token: u64 = 0;
        if snapshot.connection.latest_rtt_ms.is_some() {
            token |= 1 << 0;
        }
        if snapshot.connection.latest_transport_path.is_some() {
            token |= 1 << 1;
        }
        if snapshot.connection.control_channel_open {
            token |= 1 << 2;
        }
        if snapshot.connection.message_channel_open {
            token |= 1 << 3;
        }
        if snapshot.connection.input_channel_open {
            token |= 1 << 4;
        }
        if snapshot.connection.chat_channel_open {
            token |= 1 << 5;
        }
        // lifecycle_state 也纳入 token：Connecting -> Recovering -> Connected 的跃迁，
        // 在首帧前通常意味着传输仍在推进。
        token |= (snapshot.connection.lifecycle_state as u64) << 16;
        token
    }

    fn build_bwe_proposal(&mut self, snapshot: &TransportSnapshot) -> Option<BwePolicyProposal> {
        if !matches!(
            snapshot.connection.lifecycle_state,
            ConnectionLifecycleStateFact::Connected | ConnectionLifecycleStateFact::Recovering
        ) {
            return None;
        }
        let sample_tick_ms = snapshot.bwe.latest_sample_tick_ms?;
        if self
            .last_bwe_sample_tick_ms
            .is_some_and(|last| sample_tick_ms <= last)
        {
            return None;
        }
        self.last_bwe_sample_tick_ms = Some(sample_tick_ms);

        let loss_ratio = snapshot
            .bwe
            .latest_loss_ratio_1s
            .or(snapshot.connection.latest_loss_ratio_1s)
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        let rtt_ms = snapshot
            .bwe
            .latest_rtt_ms
            .or(snapshot.connection.latest_rtt_ms);
        let actual_kbps = snapshot.bwe.latest_actual_video_bitrate_kbps.unwrap_or(0.0);
        let webrtc_config = self
            .runtime_config
            .lock()
            .ok()
            .map(|config| config.webrtc.clone())
            .unwrap_or_default();
        let (baseline_remote_profile, session_target_type, twcc_observation, session_phase) =
            RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
                (
                    stats.baseline_remote_profile.clone(),
                    stats.session_target_type.clone(),
                    stats.latest_video_twcc_observation.clone(),
                    parse_session_phase(stats.session_phase.as_deref()),
                )
            })
            .unwrap_or((None, None, None, SessionPhase::Steady));
        let current_target_kbps = snapshot
            .bwe
            .target_remb_kbps
            .unwrap_or(self.last_sent_remb_kbps.max(DEFAULT_BWE_TARGET_KBPS));
        self.last_sent_remb_kbps = current_target_kbps;
        let bwe_decision = resolve_target_remb_kbps(
            &webrtc_config,
            snapshot.bwe.latest_observed_remb_kbps,
            actual_kbps,
            loss_ratio,
            rtt_ms,
            baseline_remote_profile.as_deref(),
            session_target_type.as_ref(),
            snapshot.connection.latest_transport_path.as_deref(),
            session_phase,
            twcc_observation.as_ref(),
            &mut self.last_sent_remb_kbps,
            &mut self.hybrid_ramp_cooldown_ticks,
        );
        let target_kbps = bwe_decision.target_kbps;
        let decision_reason = bwe_decision.reason;
        let reason_changed = self
            .last_bwe_reason
            .as_ref()
            .is_none_or(|last| last != &decision_reason);
        let is_unstable_hold = decision_reason.ends_with("unstable-hold");
        if is_unstable_hold && target_kbps == current_target_kbps {
            self.unstable_hold_streak = self.unstable_hold_streak.saturating_add(1);
            if self.unstable_hold_streak < BWE_UNSTABLE_HOLD_CONFIRMATION_TICKS {
                return None;
            }
        } else {
            self.unstable_hold_streak = 0;
        }
        if target_kbps == current_target_kbps && !reason_changed {
            return None;
        }
        self.last_bwe_reason = Some(decision_reason.clone());
        self.next_bwe_observation_id = self.next_bwe_observation_id.saturating_add(1);
        let evaluation = RtcBweEvaluation {
            target_remb_kbps: target_kbps,
            decision_reason,
            observation_id: self.next_bwe_observation_id,
        };
        Some(BwePolicyProposal { evaluation })
    }

    fn map_planned_command(
        &mut self,
        command: PlannedTransportCommand,
        bwe_observation_id: u64,
    ) -> Vec<SessionCommand> {
        map_planned_command_to_session_commands(command, bwe_observation_id)
    }

    fn evaluate_scheduling_owner(
        &mut self,
        snapshot: &TransportSnapshot,
        orchestration: &RtcSessionPolicyOrchestrationInput,
    ) -> crate::transport::rtc::policy::video_scheduling_owner::VideoSchedulingOwnerOutput {
        let observed_at_ms = Self::resolve_policy_observed_at_ms(snapshot);
        let sink = RuntimeStatsSink::new(self.runtime_stats.clone());
        let owner_facts = &orchestration.owner_facts;
        let owner_input = &orchestration.owner_input;
        let owner_output = self.scheduling_owner.evaluate(owner_input);
        let owner_current_clean_anchor_observed_at_ms = current_clean_anchor_observed_at_ms(
            owner_facts.clean_anchor_epoch,
            owner_facts.clean_anchor_observed_at_ms,
            owner_facts.clean_anchor_source_event.as_deref(),
            owner_facts.recovery_epoch,
        );
        let recovery_transport_await_unresolved = owner_facts
            .latest_video_timeline_observation
            .as_ref()
            .is_some_and(|timeline| {
                has_current_transport_await_issue_from_observation(
                    timeline,
                    owner_current_clean_anchor_observed_at_ms,
                )
            });
        let recovery_ingress_waiting = owner_facts
            .latest_video_timeline_observation
            .as_ref()
            .is_some_and(|timeline| {
                has_current_transport_await_issue_from_observation(
                    timeline,
                    owner_current_clean_anchor_observed_at_ms,
                ) && is_ingress_waiting_keyframe(
                    Some(timeline.chain.state.as_str()),
                    timeline.chain.reason.as_deref(),
                    Some(timeline.source_event.as_str()),
                )
            });
        let recovery_phase = match owner_output.state {
            VideoSchedulingOwnerState::SeekingAnchor | VideoSchedulingOwnerState::Priming => {
                "priming"
            }
            VideoSchedulingOwnerState::RebuildingSupply => "recovering",
            VideoSchedulingOwnerState::SupplyStarved => "starved",
            VideoSchedulingOwnerState::DegradedServing => "degraded",
            VideoSchedulingOwnerState::StableServing => "stable",
        };
        let recovery_exit_gate = if recovery_ingress_waiting {
            "blocked:ingress-waiting"
        } else if recovery_transport_await_unresolved {
            "blocked:transport-await-unresolved"
        } else if owner_output.state == VideoSchedulingOwnerState::StableServing {
            "ready"
        } else {
            "open"
        };
        // canonical owner contract 由 owner state machine 直接写入 runtime stats，
        // 不再维护 recovery coupling 的并行语义轴。
        sink.update(|stats| {
            stats.recovery_policy_profile =
                Some(orchestration.recovery_profile_kind.as_str().to_string());
            stats.video_owner_state = Some(owner_output.state.as_str().to_string());
            stats.video_owner_reason = Some(owner_output.diagnostics.reason_label.clone());
            stats.video_owner_source =
                Some(owner_output.diagnostics.reason_source.as_str().to_string());
            stats.video_owner_observed_at_ms = Some(owner_output.observed_at_ms);
            stats.recovery_phase = Some(recovery_phase.to_string());
            stats.recovery_exit_gate = Some(recovery_exit_gate.to_string());
            stats.recovery_ingress_waiting = Some(recovery_ingress_waiting);
            stats.recovery_transport_await_unresolved = Some(recovery_transport_await_unresolved);
        });
        let has_unresolved_transport_await_issue =
            RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
                Self::has_unresolved_transport_await_issue(stats)
            })
            .unwrap_or(false);
        let RecoveryRampResolution {
            should_acknowledge_clean_anchor,
            should_close_ramp_up,
        } = resolve_stable_recovery_settle(
            self.runtime_stats.as_ref(),
            owner_output.state,
            owner_input.clean_anchor_epoch,
            owner_input.recovery_epoch,
            observed_at_ms,
            has_unresolved_transport_await_issue,
        );
        if should_acknowledge_clean_anchor {
            self.recovery_coordinator.acknowledge_clean_anchor();
        }
        if should_close_ramp_up {
            RuntimeStatsSink::new(self.runtime_stats.clone())
                .complete_transport_recovery_after_stable_settle(observed_at_ms);
            self.recovery_coordinator.acknowledge_stable_recovery();
        }
        owner_output
    }

    fn record_recovery_decision_ledger(
        &mut self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        proposal: Option<&RecoveryPolicyProposal>,
    ) {
        let state_after = self.resolve_recovery_state(snapshot, owner_state, proposal);
        let state_before = self.last_recovery_state.unwrap_or(state_after);
        self.last_recovery_state = Some(state_after);
        let observed_at_ms = Self::resolve_policy_observed_at_ms(snapshot);
        let (decision_id, input_signal, gate_result, action_selected, budget_before, budget_after) =
            if let Some(proposal) = proposal {
                let local_probe_only =
                    self.is_non_escalating_keyframe_probe(proposal, observed_at_ms);
                let failed_terminal_reason = self.failed_terminal_reason.clone().filter(|reason| {
                    self.failed_terminal_since_ms.is_some()
                        && (reason != "reconnectBudgetExhausted"
                            || proposal.reason == VideoEscalationReason::LifecycleRecovering)
                });
                (
                    proposal.decision.observation_id,
                    format!(
                        "{}:{}",
                        proposal.reason.label(),
                        proposal.reason_label.as_str()
                    ),
                    proposal
                        .ledger_gate_result(failed_terminal_reason.as_deref(), local_probe_only),
                    proposal.ledger_action_selected(failed_terminal_reason.as_deref()),
                    Some(map_budget_snapshot(proposal.budget_before)),
                    Some(map_budget_snapshot(proposal.budget_after)),
                )
            } else {
                let terminal_reason = self.failed_terminal_reason.clone();
                (
                    self.next_recovery_decision_ledger_id(),
                    terminal_reason
                        .as_ref()
                        .map(|reason| format!("terminal:{reason}"))
                        .unwrap_or_else(|| "none".to_string()),
                    terminal_reason
                        .as_ref()
                        .map(|reason| format!("terminal:{reason}"))
                        .unwrap_or_else(|| "no-signal".to_string()),
                    if terminal_reason.is_some() {
                        RecoveryLedgerNarrativeState::FailedTerminal
                            .as_str()
                            .to_string()
                    } else {
                        "none".to_string()
                    },
                    None,
                    None,
                )
            };
        let ledger = XbxEngineRecoveryDecisionLedgerObservation {
            decision_id,
            state_before: state_before.as_str().to_string(),
            state_after: state_after.as_str().to_string(),
            input_signal,
            gate_result,
            action_selected,
            frame_value: None,
            gap_severity: None,
            recovery_episode_stage: None,
            recovery_episode_progress_at_ms: None,
            coalescing_mode: None,
            unlock_reason: None,
            preempt_reason: None,
            recovery_primary_action: None,
            budget_before,
            budget_after,
            trigger_observation_label: None,
            trigger_observation_summary: None,
            command_result: None,
            command_detail: None,
            observed_at_ms,
        };
        RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
            if let Some(p) = proposal {
                stats.recovery_rfc_authoritative_ceiling = Some(
                    session_cost_ceiling_for_recovery_action(p.decision.action)
                        .as_rfc_str()
                        .to_string(),
                );
                stats.recovery_rfc_authoritative_fault_domain = Some(
                    resolve_session_fault_domain(p.reason)
                        .as_rfc_str()
                        .to_string(),
                );
                stats.recovery_rfc_authoritative_stage = Some(
                    resolve_session_recovery_stage(owner_state)
                        .as_rfc_str()
                        .to_string(),
                );
                stats.recovery_active_escalation_reason = Some(p.reason.label().to_string());
            } else {
                stats.recovery_rfc_authoritative_ceiling = None;
                stats.recovery_rfc_authoritative_fault_domain = None;
                stats.recovery_rfc_authoritative_stage = None;
                stats.recovery_active_escalation_reason = None;
            }
            stats.recent_recovery_decision_ledgers.push(ledger.clone());
            if stats.recent_recovery_decision_ledgers.len()
                > RECENT_RECOVERY_DECISION_LEDGER_CAPACITY
            {
                let overflow = stats.recent_recovery_decision_ledgers.len()
                    - RECENT_RECOVERY_DECISION_LEDGER_CAPACITY;
                stats.recent_recovery_decision_ledgers.drain(0..overflow);
            }
            // 若 latest 仍表示「命令已选、等待 TransportCommand 回填」，而本拍推导出的 ledger 更弱（无 pending），
            // 默认保留 headline，避免在 command_result 回填前把叙事覆盖成「空窗」。
            let keep_existing_latest_pending = stats
                .latest_recovery_decision_ledger
                .as_ref()
                .is_some_and(|latest| {
                    Self::ledger_has_pending_command(latest)
                        && !Self::ledger_has_pending_command(&ledger)
                        && !matches!(
                            latest.action_selected.as_str(),
                            "coalesced:keyframeInFlight" | "coalesced:decoderResetInFlight",
                        )
                });
            if !keep_existing_latest_pending {
                stats.latest_recovery_decision_ledger = Some(ledger.clone());
            }
        });
    }

    fn resolve_recovery_state(
        &self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        proposal: Option<&RecoveryPolicyProposal>,
    ) -> RecoveryLedgerNarrativeState {
        if self.failed_terminal_since_ms.is_some() {
            return RecoveryLedgerNarrativeState::FailedTerminal;
        }
        match snapshot.connection.lifecycle_state {
            ConnectionLifecycleStateFact::Failed => RecoveryLedgerNarrativeState::FailedTerminal,
            ConnectionLifecycleStateFact::Recovering | ConnectionLifecycleStateFact::Connecting => {
                RecoveryLedgerNarrativeState::Reconnecting
            }
            ConnectionLifecycleStateFact::Connected => match owner_state {
                VideoSchedulingOwnerState::SeekingAnchor | VideoSchedulingOwnerState::Priming => {
                    if proposal.is_some_and(|proposal| {
                        self.is_non_escalating_keyframe_probe(proposal, snapshot.now_ms)
                    }) {
                        RecoveryLedgerNarrativeState::LocalSelfHealing
                    } else {
                        RecoveryLedgerNarrativeState::Observing
                    }
                }
                VideoSchedulingOwnerState::StableServing
                | VideoSchedulingOwnerState::DegradedServing => {
                    if ramp_up_active(self.runtime_stats.as_ref()) {
                        RecoveryLedgerNarrativeState::RampUp
                    } else if proposal.is_some_and(|proposal| {
                        self.is_non_escalating_keyframe_probe(proposal, snapshot.now_ms)
                    }) {
                        RecoveryLedgerNarrativeState::LocalSelfHealing
                    } else if proposal.is_some_and(|proposal| {
                        Self::is_blocked_recovery_action(proposal.decision.action)
                    }) {
                        RecoveryLedgerNarrativeState::LocalSelfHealing
                    } else {
                        RecoveryLedgerNarrativeState::Stable
                    }
                }
                VideoSchedulingOwnerState::RebuildingSupply
                | VideoSchedulingOwnerState::SupplyStarved => {
                    if let Some(proposal) = proposal {
                        if self.is_non_escalating_keyframe_probe(proposal, snapshot.now_ms) {
                            RecoveryLedgerNarrativeState::LocalSelfHealing
                        } else if Self::is_active_recovery_action(proposal.decision.action) {
                            RecoveryLedgerNarrativeState::ActiveRecovery
                        } else if Self::is_blocked_recovery_action(proposal.decision.action) {
                            RecoveryLedgerNarrativeState::RecoveryBlocked
                        } else {
                            RecoveryLedgerNarrativeState::RecoveryEligible
                        }
                    } else {
                        RecoveryLedgerNarrativeState::RecoveryEligible
                    }
                }
            },
            _ => RecoveryLedgerNarrativeState::Detecting,
        }
    }

    fn is_non_escalating_keyframe_probe(
        &self,
        proposal: &RecoveryPolicyProposal,
        observed_at_ms: f64,
    ) -> bool {
        self.is_local_keyframe_probe_action(proposal, observed_at_ms)
    }

    fn is_pre_first_frame_acquisition_probe(&self, proposal: &RecoveryPolicyProposal) -> bool {
        if !is_first_frame_acquisition_reason_label(proposal.reason_label.as_str()) {
            return false;
        }
        RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            stats.latest_video_host_present_time_ms.is_none()
                && stats.latest_video_decode_ok_time_ms.is_none()
        })
        .unwrap_or(false)
    }

    fn is_exploratory_transport_await_keyframe(
        &self,
        proposal: &RecoveryPolicyProposal,
        observed_at_ms: f64,
    ) -> bool {
        proposal.reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && proposal.decision.action == RecoveryAction::RequestKeyframe
            && !RecoveryCoordinator::transport_await_has_hard_recovery_evidence(
                self.runtime_stats.as_ref(),
                proposal.budget_before.recovery_epoch,
                observed_at_ms,
            )
    }

    fn is_local_keyframe_probe_action(
        &self,
        proposal: &RecoveryPolicyProposal,
        observed_at_ms: f64,
    ) -> bool {
        if proposal.decision.action != RecoveryAction::RequestKeyframe {
            return false;
        }
        self.is_pre_first_frame_acquisition_probe(proposal)
            || self.is_exploratory_transport_await_keyframe(proposal, observed_at_ms)
            || matches!(
                resolve_session_fault_domain(proposal.reason),
                SessionFaultDomain::ReferenceChain
                    | SessionFaultDomain::DecodePipeline
                    | SessionFaultDomain::DisplaySupply
            )
    }

    fn should_force_first_frame_acquisition_local_action(
        &self,
        snapshot: &TransportSnapshot,
        proposal: &RecoveryCoordinatorProposal,
        observed_at_ms: f64,
    ) -> bool {
        if snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Connected {
            return false;
        }
        if !is_first_frame_acquisition_reason_label(proposal.signal.reason_label.as_str()) {
            return false;
        }
        if !first_frame_acquisition_priority_active(
            snapshot,
            observed_at_ms,
            self.runtime_stats.as_ref(),
            self.pre_first_frame_reconnect_fallback_ms(),
        ) {
            return false;
        }
        matches!(
            proposal.decision.action,
            RecoveryAction::RequestDecoderReset
                | RecoveryAction::RequestReconnectCandidate
                | RecoveryAction::CoalescedDecoderResetInFlight
                | RecoveryAction::WaitForDecoderResetBurst
        )
    }

    fn resolve_first_frame_acquisition_local_action(
        &self,
        proposal: &RecoveryCoordinatorProposal,
        observed_at_ms: f64,
    ) -> RecoveryAction {
        if self.has_recent_first_frame_keyframe_attempt(observed_at_ms) {
            return RecoveryAction::CoalescedKeyframeInFlight;
        }
        match proposal.decision.action {
            RecoveryAction::CoalescedDecoderResetInFlight
            | RecoveryAction::WaitForDecoderResetBurst => RecoveryAction::WaitForBurst,
            _ => RecoveryAction::RequestKeyframe,
        }
    }

    fn has_recent_first_frame_keyframe_attempt(&self, observed_at_ms: f64) -> bool {
        const FIRST_FRAME_KEYFRAME_ATTEMPT_FRESH_MS: f64 = 1_500.0;
        RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            stats
                .latest_keyframe_request_episode
                .as_ref()
                .is_some_and(|episode| {
                    (observed_at_ms - episode.requested_at_ms).max(0.0)
                        <= FIRST_FRAME_KEYFRAME_ATTEMPT_FRESH_MS
                        && episode
                            .request_reason
                            .as_deref()
                            .is_some_and(is_first_frame_acquisition_reason_label)
                        && !matches!(episode.status.as_str(), "missed" | "late" | "timedOut")
                })
        })
        .unwrap_or(false)
    }

    fn is_active_recovery_action(action: RecoveryAction) -> bool {
        matches!(
            action,
            RecoveryAction::RequestDecoderReset | RecoveryAction::RequestReconnectCandidate
        )
    }

    fn is_blocked_recovery_action(action: RecoveryAction) -> bool {
        matches!(
            action,
            RecoveryAction::WaitForBurst
                | RecoveryAction::WaitForDecoderResetBurst
                | RecoveryAction::CooldownSuppressed
                | RecoveryAction::CoalescedKeyframeInFlight
                | RecoveryAction::CoalescedDecoderResetInFlight
                | RecoveryAction::StartupGraceSuppressed
        )
    }

    fn next_recovery_decision_ledger_id(&mut self) -> u64 {
        self.next_recovery_decision_ledger_id =
            self.next_recovery_decision_ledger_id.saturating_add(1);
        9_000_000 + self.next_recovery_decision_ledger_id
    }

    fn resolve_policy_observed_at_ms(snapshot: &TransportSnapshot) -> f64 {
        snapshot
            .recovery
            .last_observed_at_ms
            .filter(|ts| ts.is_finite())
            .map(|ts| ts.max(snapshot.now_ms))
            .unwrap_or(snapshot.now_ms)
    }
}

fn map_budget_snapshot(
    budget: crate::transport::rtc::recovery::escalation::RecoveryActionBudgetState,
) -> XbxEngineRecoveryBudgetSnapshot {
    XbxEngineRecoveryBudgetSnapshot {
        recovery_epoch: budget.recovery_epoch,
        keyframe_budget_used: budget.keyframe_budget_used,
        keyframe_budget_limit: budget.keyframe_budget_limit,
        decoder_reset_budget_used: budget.decoder_reset_budget_used,
        decoder_reset_budget_limit: budget.decoder_reset_budget_limit,
        reconnect_budget_used: budget.reconnect_budget_used,
        reconnect_budget_limit: budget.reconnect_budget_limit,
    }
}

#[cfg(test)]
#[path = "policy_tests/mod.rs"]
mod tests;
