use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::api::backend::{
    XbxEngineMediaRuntimeStats, XbxEngineRecoveryBudgetSnapshot,
    XbxEngineRecoveryDecisionLedgerObservation,
};
use crate::api::runtime::XbxEngineRuntimeConfig;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::bwe::evaluator::RtcBweEvaluation;
use crate::transport::rtc::bwe::policy::resolve_target_remb_kbps;
use crate::transport::rtc::facts::{ConnectionLifecycleStateFact, TransportCommand};
use crate::transport::rtc::policy::bwe::BwePolicyProposal;
use crate::transport::rtc::policy::display_supply::SchedulingDemandSignal;
use crate::transport::rtc::policy::planner::PlannedTransportCommand;
use crate::transport::rtc::policy::recovery::RecoveryPolicyProposal;
use crate::transport::rtc::policy::scheduling::{
    map_planned_command_to_transport_commands, SchedulingPolicyEngine, SchedulingPolicyInput,
    TwccWarmupState,
};
use crate::transport::rtc::policy::video_scheduling_owner::{
    RecoveryIntentContract, RecoveryIntentSource, VideoSchedulingOwner, VideoSchedulingOwnerInput,
    VideoSchedulingOwnerState,
};
use crate::transport::rtc::projection::TransportSnapshot;
use crate::transport::rtc::recovery::coordinator::{
    RecoveryCoordinator, RecoveryCoordinatorProposal, RecoveryOwnerSignal,
};
use crate::transport::rtc::recovery::escalation::{
    RecoveryAction, VideoEscalationController, VideoEscalationReason,
};
use crate::transport::rtc::recovery::policy::ScenarioPolicyProfileKind;
use crate::transport::rtc::recovery::remote_profile_runtime::persist_runtime_remote_profile_facts;
use crate::transport::rtc::recovery::runtime_state::{
    has_fresh_media_output, resolve_recovery_profile,
};
use crate::transport::rtc::recovery::startup::SessionPhase;
use crate::transport::rtc::session::actor::SessionPolicyHook;

const DEFAULT_BWE_TARGET_KBPS: u32 = 16_000;
const BWE_UNSTABLE_HOLD_CONFIRMATION_TICKS: u8 = 2;
const RECOVERY_STARTUP_GRACE_MS: u64 = 800;
const RECOVERING_RECONNECT_PROPOSAL_INTERVAL_MS: f64 = 1_500.0;
const CONNECTING_PRE_FIRST_FRAME_RECONNECT_PROPOSAL_INTERVAL_MS: f64 = 4_500.0;
const CLOUD_RECOVERING_RECONNECT_PROPOSAL_INTERVAL_MS: f64 = 2_500.0;
const CLOUD_BUILDER_CONFIGURED_RECONNECT_PROPOSAL_INTERVAL_MS: f64 = 4_500.0;
const CLOUD_MISSING_LOCAL_FEEDBACK_RECONNECT_PROPOSAL_INTERVAL_MS: f64 = 3_500.0;
const RECOVERY_NO_PROGRESS_RECONNECT_FALLBACK_MS: f64 = 4_000.0;
const RECOVERY_PRE_FIRST_FRAME_RECONNECT_FALLBACK_MS: f64 = 15_000.0;
const CLOUD_RECOVERY_PRE_FIRST_FRAME_RECONNECT_FALLBACK_MS: f64 = 35_000.0;
const CONNECTING_PRE_FIRST_FRAME_FAILED_TERMINAL_MIN_MS: f64 = 90_000.0;
const CONNECTED_PRESENT_STALL_RECONNECT_FALLBACK_MS: f64 = 10_000.0;
const CONNECTED_PRESENT_STALL_MIN_AGE_MS: f64 = 1_500.0;
const CONNECTED_PRESENT_STALL_HARD_AGE_MS: f64 = 4_000.0;
const CONNECTED_CONNECTIVITY_EVIDENCE_STALE_MS: f64 = 3_000.0;
const LIVENESS_RECONNECT_ATTEMPT_LIMIT: u8 = 3;
const CLOUD_LIVENESS_RECONNECT_ATTEMPT_LIMIT: u8 = 6;
const ADAPTER_IDLE_RENDER_SLACK_MIN_MS: f64 = 220.0;
const ADAPTER_IDLE_RENDER_SLACK_MAX_MS: f64 = 450.0;
const RECOVERY_RAMP_UP_LIGHT_SIGNAL_HOLD_MS: f64 = 1_500.0;
const RECENT_RECOVERY_DECISION_LEDGER_CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecoveryLivenessState {
    Detecting,
    Recovering,
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

impl RecoveryLivenessState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Detecting => "detecting",
            Self::Recovering => "recovering",
            Self::RampUp => "ramp-up",
            Self::Reconnecting => "reconnecting",
            Self::Stable => "stable",
            Self::FailedTerminal => "failed-terminal",
        }
    }
}

fn is_startup_display_phase(value: Option<&str>) -> bool {
    matches!(
        value,
        Some("startup" | "handshaking" | "priming" | "connecting")
    )
}

/// rtc session 主线策略：
/// - 统一把 reconnect/recovery/BWE proposal 收口到 session policy
/// - 复用 planner 的优先级（reconnect > recovery > bwe）
/// - stack 只做命令执行与 CommandResultFact 回写
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
    last_recovery_state: Option<RecoveryLivenessState>,
    next_recovery_decision_ledger_id: u64,
}

impl RtcSessionPolicy {
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
            last_recovery_state: None,
            next_recovery_decision_ledger_id: 0,
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
    fn on_snapshot(&mut self, snapshot: &TransportSnapshot) -> Vec<TransportCommand> {
        self.refresh_escalation_profile();
        self.sync_recovery_epoch();
        let demand = self.build_scheduling_demand_signal();
        let owner_output = self.evaluate_scheduling_owner(snapshot, demand.clone());
        let recovery = self.build_recovery_proposal(
            snapshot,
            owner_output.state,
            owner_output.recovery_intent.as_ref(),
            owner_output.reason_label.as_str(),
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
        if let Some(summary) = owner_output.temporary_diagnostic_summary.as_ref() {
            RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
                stats.latest_observation_label = Some("videoOwnerTempDiagnostic".to_string());
                stats.latest_observation_summary = Some(summary.clone());
            });
        }
        commands
    }
}

impl RtcSessionPolicy {
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
        // 云侧首帧前容忍更长 no-progress 窗口，降低误触发重连。
        if self.is_cloud_gaming_profile() {
            CLOUD_RECOVERY_PRE_FIRST_FRAME_RECONNECT_FALLBACK_MS
        } else {
            RECOVERY_PRE_FIRST_FRAME_RECONNECT_FALLBACK_MS
        }
    }

    fn liveness_reconnect_attempt_limit(&self) -> u8 {
        // 云侧提高尝试上限，给高抖动链路更多恢复机会。
        if self.is_cloud_gaming_profile() {
            CLOUD_LIVENESS_RECONNECT_ATTEMPT_LIMIT
        } else {
            LIVENESS_RECONNECT_ATTEMPT_LIMIT
        }
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
            self.last_recovery_state = None;
            self.next_recovery_decision_ledger_id = 0;
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
        _owner_reason_label: &str,
    ) -> Option<RecoveryPolicyProposal> {
        self.maybe_clear_failed_terminal(snapshot, owner_state);
        if self.failed_terminal_since_ms.is_some() {
            return None;
        }
        let observed_at_ms = Self::resolve_policy_observed_at_ms(snapshot);
        let twcc_warmup_state = self.resolve_twcc_warmup_state();
        let has_media_recovery_surface = recovery_intent.is_some();
        let active_media_recovery_intent = recovery_intent.filter(|intent| intent.emit);
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
            let reason_label = if lifecycle_disconnected {
                "rtcConnectionDisconnected".to_string()
            } else if recovering_connectivity_failure {
                "rtcConnectionRecovering".to_string()
            } else if force_lifecycle_reconnect {
                "livenessNoProgressTimeout".to_string()
            } else {
                "rtcConnectionRecovering".to_string()
            };
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
            } else if let Some(intent) = active_media_recovery_intent {
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
                if self.should_hold_pre_first_frame_startup_wait_keyframe(
                    snapshot,
                    owner_state,
                    intent.reason_label.as_str(),
                    observed_at_ms,
                ) {
                    return None;
                }
                let reason = map_owner_reason_label_to_escalation_reason(
                    intent.source,
                    intent.reason_label.as_str(),
                )?;
                RecoveryOwnerSignal {
                    reason,
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
                if self.should_hold_pre_first_frame_startup_wait_keyframe(
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
        let mut proposal = self
            .recovery_coordinator
            .propose_from_owner_signal(owner_signal, self.runtime_stats.as_ref());
        if proposal.signal.reason == VideoEscalationReason::TransportAwaitRecoveryKeyframe
            && self.should_absorb_stale_transport_await_replay(
                snapshot,
                owner_state,
                proposal.signal.reason_label.as_str(),
                observed_at_ms,
            )
        {
            return None;
        }
        if block_lifecycle_reconnect_candidate
            && proposal.decision.action == RecoveryAction::RequestReconnectCandidate
        {
            // Recovering + 媒体恢复意图场景禁止走生命周期重连候选，避免抢占媒体恢复收敛路径。
            proposal.decision.action = RecoveryAction::CooldownSuppressed;
        }
        if self.should_hold_media_reconnect_during_twcc_warmup(
            proposal.signal.reason,
            twcc_warmup_state,
            proposal.decision.action,
        ) {
            // cloud feedback 尚未进入 valid local-feedback 前，媒体域 reconnect 先收敛在本地恢复链，
            // 避免 builder-configured / missing-local-feedback 阶段过早把恢复升级到 reconnect。
            proposal.decision.action = RecoveryAction::CooldownSuppressed;
        }
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
        if self.should_absorb_light_recovery_signal_during_ramp_up(
            snapshot,
            owner_state,
            &proposal,
            observed_at_ms,
        ) {
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
        if self.should_enter_failed_terminal(&proposal) {
            self.mark_failed_terminal(snapshot, "reconnectBudgetExhausted");
        }
        let reason_domain = resolve_runtime_reconnect_reason_domain(
            proposal.signal.reason,
            proposal.decision.action,
        );
        Some(RecoveryPolicyProposal {
            decision: proposal.decision,
            reason: proposal.signal.reason,
            reason_label: proposal.signal.reason_label,
            reason_domain,
            budget_before: proposal.budget_before,
            budget_after: proposal.budget_after,
        })
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

    fn should_absorb_stale_transport_await_replay(
        &self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        diagnosis_label: &str,
        observed_at_ms: f64,
    ) -> bool {
        const STALE_TRANSPORT_AWAIT_REPLAY_MAX_AGE_MS: f64 = 220.0;
        if diagnosis_label != "transportAwaitRecoveryKeyframe" {
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
            let pipeline_not_stalled = !stats.video_decoder_stalled.unwrap_or(false)
                && !stats.video_renderer_stalled.unwrap_or(false);
            let healthy_media_baseline = chain_healthy
                && track_attached_with_video
                && pipeline_not_stalled
                && has_fresh_media_output(stats, observed_at_ms);
            if Self::owner_state_is_steady_serving(owner_state) && healthy_media_baseline {
                return true;
            }
            let diagnosis_is_stale = snapshot.recovery.last_observed_at_ms.is_some_and(|last| {
                (observed_at_ms - last).max(0.0) > STALE_TRANSPORT_AWAIT_REPLAY_MAX_AGE_MS
            });
            diagnosis_is_stale && current_clean_anchor && healthy_media_baseline
        })
        .unwrap_or(false)
    }

    fn has_unresolved_transport_await_issue(stats: &crate::XbxEngineMediaRuntimeStats) -> bool {
        const TRANSPORT_AWAIT_UNRESOLVED_REASONS: [&str; 3] = [
            "awaitingRecoveryKeyframe",
            "awaitRecoveryKeyframe",
            "referenceChainUnrecoverable",
        ];
        let timeline = match stats.latest_video_timeline_observation.as_ref() {
            Some(timeline) => timeline,
            None => return false,
        };
        if timeline
            .chain
            .reason
            .as_deref()
            .is_some_and(|reason| TRANSPORT_AWAIT_UNRESOLVED_REASONS.contains(&reason))
        {
            return true;
        }
        if timeline
            .frame
            .as_ref()
            .and_then(|frame| frame.close_reason.as_deref())
            .is_some_and(|reason| TRANSPORT_AWAIT_UNRESOLVED_REASONS.contains(&reason))
        {
            return true;
        }
        timeline.gap.as_ref().is_some_and(|gap| {
            !matches!(gap.state.as_str(), "resolved" | "expired")
                && timeline
                    .chain
                    .reason
                    .as_deref()
                    .is_some_and(|reason| TRANSPORT_AWAIT_UNRESOLVED_REASONS.contains(&reason))
        })
    }

    fn should_hold_pre_first_frame_connected_idle_timeout(
        &self,
        snapshot: &TransportSnapshot,
        _owner_state: VideoSchedulingOwnerState,
        diagnosis_label: &str,
        observed_at_ms: f64,
    ) -> bool {
        if diagnosis_label != "adapterIdleTimeout" {
            return false;
        }
        if snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Connected
            || snapshot.media.frame_count != 0
        {
            return false;
        }
        RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            if stats.transport_state != xbxengine_protocol::XbxEngineTransportStateDto::Connected {
                return false;
            }
            let Some(track) = stats.latest_video_track_status.as_ref() else {
                return false;
            };
            if track.state != "remoteTrackAttached" {
                return false;
            }
            let host_feedback_not_ready = stats.latest_video_host_present_time_ms.is_none()
                && stats.latest_video_decode_ok_time_ms.is_none();
            let pipeline_not_stalled = !stats.video_decoder_stalled.unwrap_or(false)
                && !stats.video_renderer_stalled.unwrap_or(false);
            let still_within_pre_first_frame_window = (observed_at_ms - track.observed_at_ms)
                .max(0.0)
                < self.pre_first_frame_reconnect_fallback_ms();
            host_feedback_not_ready && pipeline_not_stalled && still_within_pre_first_frame_window
        })
        .unwrap_or(false)
    }

    fn should_hold_pre_first_frame_startup_wait_keyframe(
        &self,
        snapshot: &TransportSnapshot,
        _owner_state: VideoSchedulingOwnerState,
        diagnosis_label: &str,
        observed_at_ms: f64,
    ) -> bool {
        if !matches!(
            diagnosis_label,
            "transportAwaitRecoveryKeyframe" | "ingressWaitKeyframe"
        ) {
            return false;
        }
        if snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Connected
            || snapshot.media.frame_count != 0
        {
            return false;
        }
        RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            if stats.transport_state != xbxengine_protocol::XbxEngineTransportStateDto::Connected {
                return false;
            }
            if !is_startup_display_phase(stats.session_phase.as_deref()) {
                return false;
            }
            let Some(track) = stats.latest_video_track_status.as_ref() else {
                return false;
            };
            if track.state != "remoteTrackAttached" || track.video_bytes_total == 0 {
                return false;
            }
            let first_frame_feedback_not_ready = stats.latest_video_host_present_time_ms.is_none()
                && stats.latest_video_decode_ok_time_ms.is_none();
            let pipeline_not_stalled = !stats.video_decoder_stalled.unwrap_or(false)
                && !stats.video_renderer_stalled.unwrap_or(false);
            let still_within_pre_first_frame_window = (observed_at_ms - track.observed_at_ms)
                .max(0.0)
                < self.pre_first_frame_reconnect_fallback_ms();
            first_frame_feedback_not_ready
                && pipeline_not_stalled
                && still_within_pre_first_frame_window
        })
        .unwrap_or(false)
    }

    fn should_hold_pre_first_frame_display_supply_degraded(
        &self,
        snapshot: &TransportSnapshot,
        _owner_state: VideoSchedulingOwnerState,
        source: RecoveryIntentSource,
        reason_label: &str,
        observed_at_ms: f64,
    ) -> bool {
        if source != RecoveryIntentSource::Supply
            || reason_label != "displaySupplyDegraded"
            || snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Connected
            || snapshot.media.frame_count != 0
        {
            return false;
        }
        RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            if stats.transport_state != xbxengine_protocol::XbxEngineTransportStateDto::Connected {
                return false;
            }
            if !is_startup_display_phase(stats.session_phase.as_deref()) {
                return false;
            }
            let Some(track) = stats.latest_video_track_status.as_ref() else {
                return false;
            };
            if track.state != "remoteTrackAttached" || track.video_bytes_total == 0 {
                return false;
            }
            let first_frame_feedback_not_ready = stats.latest_video_host_present_time_ms.is_none()
                && stats.latest_video_decode_ok_time_ms.is_none();
            let pipeline_not_stalled = !stats.video_decoder_stalled.unwrap_or(false)
                && !stats.video_renderer_stalled.unwrap_or(false);
            let still_within_pre_first_frame_window = (observed_at_ms - track.observed_at_ms)
                .max(0.0)
                < self.pre_first_frame_reconnect_fallback_ms();
            first_frame_feedback_not_ready
                && pipeline_not_stalled
                && still_within_pre_first_frame_window
        })
        .unwrap_or(false)
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
            || owner_state != VideoSchedulingOwnerState::SupplyStarved
            || proposal.signal.reason != VideoEscalationReason::AdapterThinStream
            || proposal.signal.reason_label != "displaySupplyDegraded"
            || proposal.decision.action != RecoveryAction::RequestKeyframe
        {
            return false;
        }
        RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            let overlapping_transport_await = stats
                .latest_video_escalation_observation
                .as_ref()
                .is_some_and(|observation| {
                    observation.reason == "transportAwaitRecoveryKeyframe"
                        && observation.recovery_window_source == "transport-await-window"
                        && (observed_at_ms - observation.observed_at_ms).max(0.0)
                            <= STALE_TRANSPORT_AWAIT_OVERLAP_MAX_AGE_MS
                })
                || stats
                    .latest_keyframe_request_episode
                    .as_ref()
                    .is_some_and(|episode| {
                        episode.request_reason.as_deref() == Some("transportAwaitRecoveryKeyframe")
                            && episode.status != "decoded"
                            && (observed_at_ms - episode.requested_at_ms).max(0.0)
                                <= STALE_TRANSPORT_AWAIT_OVERLAP_MAX_AGE_MS
                    })
                || stats.session_phase.as_deref() == Some("recovering");
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

    fn should_absorb_light_recovery_signal_during_ramp_up(
        &self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        proposal: &RecoveryCoordinatorProposal,
        observed_at_ms: f64,
    ) -> bool {
        if snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Connected
            || owner_state != VideoSchedulingOwnerState::StableServing
            || matches!(
                proposal.decision.action,
                RecoveryAction::RequestReconnectCandidate | RecoveryAction::RequestDecoderReset
            )
        {
            return false;
        }
        if !matches!(
            proposal.signal.reason,
            VideoEscalationReason::AdapterIdleTimeout
                | VideoEscalationReason::AdapterThinStream
                | VideoEscalationReason::TransportAwaitRecoveryKeyframe
        ) {
            return false;
        }
        RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            let current_clean_anchor = stats
                .video_anchor_clean_epoch
                .is_some_and(|epoch| epoch == stats.transport_recovery_epoch)
                && stats.video_anchor_clean_source_event.as_deref()
                    == Some("chain-clean-keyframe-submitted");
            if !current_clean_anchor || !stats.transport_recovery_episode_active {
                return false;
            }
            let still_in_ramp_up_window =
                stats
                    .video_anchor_clean_observed_at_ms
                    .is_some_and(|anchor_at_ms| {
                        (observed_at_ms - anchor_at_ms).max(0.0)
                            <= RECOVERY_RAMP_UP_LIGHT_SIGNAL_HOLD_MS
                    });
            if !still_in_ramp_up_window {
                return false;
            }
            let pipeline_not_stalled = !stats.video_decoder_stalled.unwrap_or(false)
                && !stats.video_renderer_stalled.unwrap_or(false);
            if !pipeline_not_stalled {
                return false;
            }
            match proposal.signal.reason {
                VideoEscalationReason::AdapterIdleTimeout
                | VideoEscalationReason::AdapterThinStream => {
                    let slack_window_ms = self.adapter_idle_render_slack_window_ms();
                    let present_or_decode_fresh = stats
                        .latest_video_host_present_time_ms
                        .is_some_and(|at_ms| (observed_at_ms - at_ms).max(0.0) <= slack_window_ms)
                        || stats.latest_video_decode_ok_time_ms.is_some_and(|at_ms| {
                            (observed_at_ms - at_ms).max(0.0) <= slack_window_ms
                        });
                    present_or_decode_fresh
                }
                VideoEscalationReason::TransportAwaitRecoveryKeyframe => {
                    let chain_healthy = stats
                        .latest_video_timeline_observation
                        .as_ref()
                        .is_some_and(|timeline| timeline.chain.state == "healthy");
                    let track_attached_with_video = stats
                        .latest_video_track_status
                        .as_ref()
                        .is_some_and(|track| {
                            track.state == "remoteTrackAttached" && track.video_bytes_total > 0
                        });
                    let diagnosis_is_short = snapshot
                        .recovery
                        .last_observed_at_ms
                        .is_some_and(|last| (observed_at_ms - last).max(0.0) <= 220.0);
                    diagnosis_is_short
                        && chain_healthy
                        && track_attached_with_video
                        && has_fresh_media_output(stats, observed_at_ms)
                }
                _ => false,
            }
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

    fn should_hold_media_reconnect_during_twcc_warmup(
        &self,
        reason: VideoEscalationReason,
        twcc_warmup_state: TwccWarmupState,
        action: RecoveryAction,
    ) -> bool {
        if !self.is_cloud_gaming_profile() || !twcc_warmup_state.blocks_bwe_updates() {
            return false;
        }
        if action != RecoveryAction::RequestReconnectCandidate {
            return false;
        }
        reason != VideoEscalationReason::LifecycleRecovering
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
        }
    }

    fn should_enter_failed_terminal(
        &self,
        proposal: &crate::transport::rtc::recovery::coordinator::RecoveryCoordinatorProposal,
    ) -> bool {
        proposal.signal.reason == VideoEscalationReason::LifecycleRecovering
            && proposal.decision.action == RecoveryAction::CooldownSuppressed
            && proposal.budget_after.reconnect_budget_used
                >= proposal.budget_after.reconnect_budget_limit
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
    ) -> Vec<TransportCommand> {
        map_planned_command_to_transport_commands(command, bwe_observation_id)
    }

    fn build_scheduling_demand_signal(&self) -> SchedulingDemandSignal {
        let (
            no_pending_pressure_level,
            no_pending_streak,
            present_age_ms,
            decode_age_ms,
            video_renderer_stalled,
            host_display_tick_epoch,
            host_present_epoch,
            host_cadence_phase,
            present_submit_count_total,
            present_drop_count_total,
            present_overwrite_count_total,
            pacer_submit_count_total,
            pacer_drop_count_total,
            renderer_submit_count_total,
            renderer_drop_count_total,
        ) = RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            (
                stats.host_no_pending_pressure_level.clone(),
                Some(stats.host_no_pending_streak),
                // present freshness 统一使用 host telemetry 时间戳，避免与 render ack 口径混杂。
                stats
                    .latest_video_host_present_time_ms
                    .map(|ts| (now_ms - ts).max(0.0)),
                stats
                    .latest_video_decode_ok_time_ms
                    .map(|ts| (now_ms - ts).max(0.0)),
                stats.video_renderer_stalled.unwrap_or(false),
                Some(stats.host_display_tick_epoch),
                Some(stats.video_present_epoch),
                stats.host_cadence_phase.clone(),
                Some(stats.video_present_submit_count_total),
                Some(stats.video_present_drop_count_total),
                Some(stats.video_present_overwrite_count_total),
                Some(stats.video_pacer_submit_count_total),
                Some(stats.video_pacer_drop_count_total),
                Some(stats.video_renderer_submit_count_total),
                Some(stats.video_renderer_drop_count_total),
            )
        })
        .unwrap_or((
            None, None, None, None, false, None, None, None, None, None, None, None, None, None,
            None,
        ));
        SchedulingDemandSignal {
            no_pending_pressure_level,
            no_pending_streak,
            present_age_ms,
            decode_age_ms,
            video_renderer_stalled,
            host_display_tick_epoch,
            host_present_epoch,
            host_cadence_phase,
            present_submit_count_total,
            present_drop_count_total,
            present_overwrite_count_total,
            pacer_submit_count_total,
            pacer_drop_count_total,
            renderer_submit_count_total,
            renderer_drop_count_total,
        }
    }

    fn evaluate_scheduling_owner(
        &mut self,
        snapshot: &TransportSnapshot,
        demand: SchedulingDemandSignal,
    ) -> crate::transport::rtc::policy::video_scheduling_owner::VideoSchedulingOwnerOutput {
        let observed_at_ms = Self::resolve_policy_observed_at_ms(snapshot);
        let sink = RuntimeStatsSink::new(self.runtime_stats.clone());
        sink.update(|stats| {
            // owner state machine 对外只消费已固化的运行期画像事实，避免各处临时重算口径漂移。
            persist_runtime_remote_profile_facts(stats, observed_at_ms);
        });
        let profile = resolve_recovery_profile(self.runtime_stats.as_ref());
        #[derive(Clone, Debug, Default)]
        struct OwnerRuntimeFacts {
            recovery_epoch: u64,
            latest_video_timeline_observation: Option<crate::XbxEngineVideoTimelineObservation>,
            clean_anchor_epoch: Option<u64>,
            clean_anchor_observed_at_ms: Option<f64>,
            clean_anchor_source_event: Option<String>,
            latest_anchor_candidate_ledger: Option<crate::XbxEngineAnchorCandidateLedger>,
            latest_video_track_status: Option<crate::XbxEngineVideoTrackStatus>,
        }

        let owner_facts =
            RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| OwnerRuntimeFacts {
                recovery_epoch: stats.transport_recovery_epoch,
                latest_video_timeline_observation: stats.latest_video_timeline_observation.clone(),
                clean_anchor_epoch: stats.video_anchor_clean_epoch,
                clean_anchor_observed_at_ms: stats.video_anchor_clean_observed_at_ms,
                clean_anchor_source_event: stats.video_anchor_clean_source_event.clone(),
                latest_anchor_candidate_ledger: stats.latest_anchor_candidate_ledger.clone(),
                latest_video_track_status: stats.latest_video_track_status.clone(),
            })
            .unwrap_or_default();
        let anchor_reason_label = owner_facts
            .latest_video_timeline_observation
            .as_ref()
            .and_then(|timeline| {
                resolve_anchor_reason_label_from_timeline(
                    timeline.chain.state.as_str(),
                    timeline.chain.reason.as_deref(),
                    timeline.source_event.as_str(),
                )
            });
        let latest_timeline_chain_state = owner_facts
            .latest_video_timeline_observation
            .as_ref()
            .map(|observation| observation.chain.state.clone());
        let latest_timeline_source_event = owner_facts
            .latest_video_timeline_observation
            .as_ref()
            .map(|observation| observation.source_event.clone());
        let latest_track_state = owner_facts
            .latest_video_track_status
            .as_ref()
            .map(|status| status.state.clone());
        let latest_track_video_bytes_total = owner_facts
            .latest_video_track_status
            .as_ref()
            .map(|status| status.video_bytes_total);
        let owner_input = VideoSchedulingOwnerInput {
            connection_state: snapshot.connection.lifecycle_state,
            recovery_epoch: owner_facts.recovery_epoch,
            anchor_reason_label,
            demand,
            clean_anchor_epoch: owner_facts.clean_anchor_epoch,
            clean_anchor_observed_at_ms: owner_facts.clean_anchor_observed_at_ms,
            clean_anchor_source_event: owner_facts.clean_anchor_source_event,
            latest_anchor_candidate_ledger: owner_facts.latest_anchor_candidate_ledger,
            latest_timeline_chain_state,
            latest_timeline_source_event,
            latest_track_state,
            latest_track_video_bytes_total,
            display_supply_thresholds: profile.display_supply_thresholds,
            observed_at_ms,
        };
        let owner_output = self.scheduling_owner.evaluate(&owner_input);
        // canonical owner contract 由 owner state machine 直接写入 runtime stats，
        // 不再维护 recovery coupling 的并行语义轴。
        sink.update(|stats| {
            stats.recovery_policy_profile = Some(profile.kind.as_str().to_string());
            stats.video_owner_state = Some(owner_output.state.as_str().to_string());
            stats.video_owner_reason = Some(owner_output.reason_label.clone());
            stats.video_owner_source = Some(owner_output.reason_source.as_str().to_string());
            stats.video_owner_observed_at_ms = Some(owner_output.observed_at_ms);
        });
        if owner_output.state == VideoSchedulingOwnerState::StableServing
            && owner_input
                .clean_anchor_epoch
                .is_some_and(|epoch| epoch == owner_input.recovery_epoch)
        {
            let should_close_ramp_up =
                RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
                    if !stats.transport_recovery_episode_active {
                        return false;
                    }
                    let anchor_age_ms = stats
                        .video_anchor_clean_observed_at_ms
                        .map(|anchor_at_ms| (observed_at_ms - anchor_at_ms).max(0.0))
                        .unwrap_or(f64::INFINITY);
                    let pipeline_not_stalled = !stats.video_decoder_stalled.unwrap_or(false)
                        && !stats.video_renderer_stalled.unwrap_or(false);
                    let unresolved_transport_await =
                        Self::has_unresolved_transport_await_issue(stats);
                    pipeline_not_stalled
                        && !unresolved_transport_await
                        && has_fresh_media_output(stats, observed_at_ms)
                        && anchor_age_ms >= RECOVERY_RAMP_UP_LIGHT_SIGNAL_HOLD_MS
                })
                .unwrap_or(false);
            let should_acknowledge_clean_anchor =
                RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
                    !Self::has_unresolved_transport_await_issue(stats)
                })
                .unwrap_or(true);
            if should_acknowledge_clean_anchor {
                self.recovery_coordinator.acknowledge_clean_anchor();
            }
            if should_close_ramp_up {
                RuntimeStatsSink::new(self.runtime_stats.clone())
                    .complete_transport_recovery_after_stable_settle(observed_at_ms);
                self.recovery_coordinator.acknowledge_stable_recovery();
            }
        }
        owner_output
    }

    fn record_recovery_decision_ledger(
        &mut self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        proposal: Option<&RecoveryPolicyProposal>,
    ) {
        let state_after = self.resolve_recovery_state(snapshot, owner_state);
        let state_before = self.last_recovery_state.unwrap_or(state_after);
        self.last_recovery_state = Some(state_after);
        let observed_at_ms = Self::resolve_policy_observed_at_ms(snapshot);
        let (decision_id, input_signal, gate_result, action_selected, budget_before, budget_after) =
            if let Some(proposal) = proposal {
                let contract = VideoEscalationController::action_contract(proposal.decision.action);
                let failed_terminal = self.failed_terminal_since_ms.is_some()
                    && self.failed_terminal_reason.as_deref() == Some("reconnectBudgetExhausted")
                    && proposal.reason == VideoEscalationReason::LifecycleRecovering;
                let gate_result = if failed_terminal {
                    "terminal:reconnectBudgetExhausted".to_string()
                } else if matches!(
                    proposal.decision.action,
                    RecoveryAction::CoalescedKeyframeInFlight
                        | RecoveryAction::CoalescedDecoderResetInFlight
                ) {
                    proposal.decision.action.label().to_string()
                } else if contract.owner.is_some() {
                    "pass".to_string()
                } else {
                    format!("suppressed:{}", proposal.decision.action.label())
                };
                (
                    proposal.decision.observation_id,
                    format!(
                        "{}:{}",
                        proposal.reason.label(),
                        proposal.reason_label.as_str()
                    ),
                    gate_result,
                    if failed_terminal {
                        RecoveryLivenessState::FailedTerminal.as_str().to_string()
                    } else {
                        proposal.decision.action.label().to_string()
                    },
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
                        RecoveryLivenessState::FailedTerminal.as_str().to_string()
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
            budget_before,
            budget_after,
            command_result: None,
            command_detail: None,
            observed_at_ms,
        };
        RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
            stats.latest_recovery_decision_ledger = Some(ledger.clone());
            stats.recent_recovery_decision_ledgers.push(ledger.clone());
            if stats.recent_recovery_decision_ledgers.len()
                > RECENT_RECOVERY_DECISION_LEDGER_CAPACITY
            {
                let overflow = stats.recent_recovery_decision_ledgers.len()
                    - RECENT_RECOVERY_DECISION_LEDGER_CAPACITY;
                stats.recent_recovery_decision_ledgers.drain(0..overflow);
            }
        });
    }

    fn resolve_recovery_state(
        &self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
    ) -> RecoveryLivenessState {
        if self.failed_terminal_since_ms.is_some() {
            return RecoveryLivenessState::FailedTerminal;
        }
        match snapshot.connection.lifecycle_state {
            ConnectionLifecycleStateFact::Failed => RecoveryLivenessState::FailedTerminal,
            ConnectionLifecycleStateFact::Recovering | ConnectionLifecycleStateFact::Connecting => {
                RecoveryLivenessState::Reconnecting
            }
            ConnectionLifecycleStateFact::Connected => match owner_state {
                VideoSchedulingOwnerState::StableServing => {
                    let ramp_up_active =
                        RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
                            stats.transport_recovery_episode_active
                                && stats
                                    .video_anchor_clean_epoch
                                    .is_some_and(|epoch| epoch == stats.transport_recovery_epoch)
                        })
                        .unwrap_or(false);
                    if ramp_up_active {
                        RecoveryLivenessState::RampUp
                    } else {
                        RecoveryLivenessState::Stable
                    }
                }
                VideoSchedulingOwnerState::DegradedServing
                | VideoSchedulingOwnerState::SeekingAnchor
                | VideoSchedulingOwnerState::Priming
                | VideoSchedulingOwnerState::RebuildingSupply
                | VideoSchedulingOwnerState::SupplyStarved => RecoveryLivenessState::Recovering,
            },
            _ => RecoveryLivenessState::Detecting,
        }
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

fn resolve_runtime_reconnect_reason_domain(
    reason: VideoEscalationReason,
    action: RecoveryAction,
) -> crate::XbxEngineRecoveryReasonDomain {
    if action != RecoveryAction::RequestReconnectCandidate {
        return reason.reconnect_domain();
    }
    match reason {
        VideoEscalationReason::LifecycleRecovering
        | VideoEscalationReason::TransportExpiredDeadline
        | VideoEscalationReason::TransportSevereDeadline
        | VideoEscalationReason::TransportRecoveredLate
        | VideoEscalationReason::TransportSampleLoss => {
            crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
        }
        VideoEscalationReason::WaitKeyframe
        | VideoEscalationReason::TransportAwaitRecoveryKeyframe
        | VideoEscalationReason::DisplaySupplyCritical
        | VideoEscalationReason::Reconfigure
        | VideoEscalationReason::DecoderBackendFailure
        | VideoEscalationReason::AdapterIdleTimeout
        | VideoEscalationReason::AdapterThinStream => crate::XbxEngineRecoveryReasonDomain::Local,
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

fn parse_session_phase(value: Option<&str>) -> SessionPhase {
    match value {
        Some("startup" | "connecting" | "handshaking" | "priming") => SessionPhase::Startup,
        Some("recovering" | "ramp-up" | "degraded") => SessionPhase::Recovering,
        _ => SessionPhase::Steady,
    }
}

fn map_label_to_escalation_reason(label: &str) -> Option<VideoEscalationReason> {
    match label {
        "ingressWaitKeyframe" => Some(VideoEscalationReason::WaitKeyframe),
        "ingressFrameAbandoned" => Some(VideoEscalationReason::WaitKeyframe),
        "waitKeyframeEntered" => Some(VideoEscalationReason::WaitKeyframe),
        "frameAbandoned" => Some(VideoEscalationReason::WaitKeyframe),
        "transportAwaitRecoveryKeyframe" => {
            Some(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
        }
        "displaySupplyCritical" => Some(VideoEscalationReason::DisplaySupplyCritical),
        "ingressReconfigure" => Some(VideoEscalationReason::Reconfigure),
        "decoderBackendFailure" => Some(VideoEscalationReason::DecoderBackendFailure),
        "adapterIdleTimeout" => Some(VideoEscalationReason::AdapterIdleTimeout),
        "adapterThinStream" => Some(VideoEscalationReason::AdapterThinStream),
        "transportExpiredDeadline" => Some(VideoEscalationReason::TransportExpiredDeadline),
        "transportSevereDeadline" => Some(VideoEscalationReason::TransportSevereDeadline),
        "transportRecoveredLate" => Some(VideoEscalationReason::TransportRecoveredLate),
        "transportSampleLoss" => Some(VideoEscalationReason::TransportSampleLoss),
        _ => None,
    }
}

fn resolve_connectivity_fallback_reason(label: &str) -> Option<VideoEscalationReason> {
    let reason = map_label_to_escalation_reason(label)?;
    match reason {
        VideoEscalationReason::TransportExpiredDeadline
        | VideoEscalationReason::TransportSevereDeadline
        | VideoEscalationReason::TransportRecoveredLate
        | VideoEscalationReason::TransportSampleLoss => Some(reason),
        _ => None,
    }
}

fn resolve_anchor_reason_label_from_timeline(
    chain_state: &str,
    chain_reason: Option<&str>,
    source_event: &str,
) -> Option<String> {
    let label = match (chain_state, chain_reason, source_event) {
        ("broken", Some(reason), _) | ("recovering", Some(reason), _) => reason,
        (_, _, "frame-await-recovery-keyframe") => "transportAwaitRecoveryKeyframe",
        (_, _, "frame-inspection-rejected-await-keyframe") => "transportAwaitRecoveryKeyframe",
        _ => return None,
    };
    map_label_to_escalation_reason(label).map(|_| label.to_string())
}

fn map_owner_reason_label_to_escalation_reason(
    source: RecoveryIntentSource,
    label: &str,
) -> Option<VideoEscalationReason> {
    match source {
        RecoveryIntentSource::Anchor => map_label_to_escalation_reason(label),
        RecoveryIntentSource::Supply => match label {
            "displaySupplyCritical" => Some(VideoEscalationReason::DisplaySupplyCritical),
            "displaySupplyDegraded" => Some(VideoEscalationReason::AdapterThinStream),
            _ => None,
        },
    }
}

#[cfg(test)]
#[path = "policy.test.rs"]
mod tests;
