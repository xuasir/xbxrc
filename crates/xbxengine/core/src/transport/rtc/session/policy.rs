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
};
use crate::transport::rtc::policy::video_scheduling_owner::{
    RecoveryIntentContract, RecoveryIntentSource, VideoSchedulingOwner, VideoSchedulingOwnerInput,
    VideoSchedulingOwnerState,
};
use crate::transport::rtc::projection::TransportSnapshot;
use crate::transport::rtc::recovery::coordinator::{RecoveryCoordinator, RecoveryOwnerSignal};
use crate::transport::rtc::recovery::escalation::{
    RecoveryAction, VideoEscalationController, VideoEscalationReason,
};
use crate::transport::rtc::recovery::policy::ScenarioPolicyProfileKind;
use crate::transport::rtc::recovery::runtime_state::resolve_recovery_profile;
use crate::transport::rtc::recovery::startup::SessionPhase;
use crate::transport::rtc::session::actor::SessionPolicyHook;

const DEFAULT_BWE_TARGET_KBPS: u32 = 16_000;
const BWE_UNSTABLE_HOLD_CONFIRMATION_TICKS: u8 = 2;
const RECOVERY_STARTUP_GRACE_MS: u64 = 800;
const RECOVERING_RECONNECT_PROPOSAL_INTERVAL_MS: f64 = 1_500.0;
const RECOVERY_NO_PROGRESS_RECONNECT_FALLBACK_MS: f64 = 4_000.0;
const RECOVERY_PRE_FIRST_FRAME_RECONNECT_FALLBACK_MS: f64 = 15_000.0;
const CONNECTED_PRESENT_STALL_RECONNECT_FALLBACK_MS: f64 = 10_000.0;
const CONNECTED_PRESENT_STALL_MIN_AGE_MS: f64 = 1_500.0;
const CONNECTED_PRESENT_STALL_HARD_AGE_MS: f64 = 4_000.0;
const LIVENESS_RECONNECT_ATTEMPT_LIMIT: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecoveryLivenessState {
    Detecting,
    Recovering,
    Reconnecting,
    Recovered,
    FailedTerminal,
}

#[derive(Clone, Copy, Debug, Default)]
struct ConnectedRenderLivenessSignal {
    latest_video_present_time_ms: Option<f64>,
    inbound_primary_video_bytes_total: u64,
    no_pending_pressure_is_high: bool,
}

impl RecoveryLivenessState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Detecting => "detecting",
            Self::Recovering => "recovering",
            Self::Reconnecting => "reconnecting",
            Self::Recovered => "recovered",
            Self::FailedTerminal => "failed-terminal",
        }
    }
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
        let bwe = if reconnect_selected_by_recovery {
            None
        } else {
            self.build_bwe_proposal(snapshot)
        };
        let bwe_observation_id = bwe
            .as_ref()
            .map(|proposal| proposal.evaluation.observation_id)
            .unwrap_or(0);

        self.scheduling_engine
            .plan(SchedulingPolicyInput {
                owner_state: owner_output.state,
                owner_health: owner_output.health,
                recovery,
                bwe,
            })
            .into_iter()
            .flat_map(|command| self.map_planned_command(command, bwe_observation_id))
            .collect()
    }
}

impl RtcSessionPolicy {
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
        owner_reason_label: &str,
    ) -> Option<RecoveryPolicyProposal> {
        self.maybe_clear_failed_terminal(snapshot, owner_state);
        if self.failed_terminal_since_ms.is_some() {
            return None;
        }
        let observed_at_ms = Self::resolve_policy_observed_at_ms(snapshot);
        let force_lifecycle_reconnect =
            self.should_force_liveness_reconnect(snapshot, owner_state, observed_at_ms);
        let lifecycle_recovering =
            snapshot.connection.lifecycle_state == ConnectionLifecycleStateFact::Recovering;
        let allow_periodic_lifecycle_reconnect =
            lifecycle_recovering && snapshot.media.frame_count > 0;
        let owner_signal = if force_lifecycle_reconnect || allow_periodic_lifecycle_reconnect {
            RuntimeStatsSink::new(self.runtime_stats.clone())
                .complete_transport_recovery_for_lifecycle_recovering(observed_at_ms);
            if self.liveness_reconnect_attempts_without_progress >= LIVENESS_RECONNECT_ATTEMPT_LIMIT
            {
                self.mark_failed_terminal(snapshot, "livenessReconnectAttemptLimitExceeded");
                return None;
            }
            if !self.should_emit_lifecycle_reconnect(observed_at_ms) {
                return None;
            }
            // 持续 recovering 期间按固定间隔推进恢复 episode，允许 reconnect 预算周期性重试。
            RuntimeStatsSink::new(self.runtime_stats.clone())
                .advance_transport_recovery_episode(observed_at_ms);
            let reason_label = if force_lifecycle_reconnect {
                "livenessNoProgressTimeout".to_string()
            } else if owner_reason_label.is_empty() {
                "rtcConnectionRecovering".to_string()
            } else {
                owner_reason_label.to_string()
            };
            RecoveryOwnerSignal {
                reason: VideoEscalationReason::LifecycleRecovering,
                reason_label,
                observed_at_ms,
            }
        } else {
            if let Some(intent) = recovery_intent {
                if !intent.emit {
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
            } else {
                let fallback_label = snapshot.recovery.latest_diagnosis_label.as_deref()?;
                let fallback_reason = map_label_to_escalation_reason(fallback_label)?;
                RecoveryOwnerSignal {
                    reason: fallback_reason,
                    reason_label: fallback_label.to_string(),
                    observed_at_ms,
                }
            }
        };
        let proposal = self
            .recovery_coordinator
            .propose_from_owner_signal(owner_signal, self.runtime_stats.as_ref());
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
        Some(RecoveryPolicyProposal {
            decision: proposal.decision,
            reason: proposal.signal.reason,
            reason_label: proposal.signal.reason_label,
            budget_before: proposal.budget_before,
            budget_after: proposal.budget_after,
        })
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
            && owner_state == VideoSchedulingOwnerState::StableServing;
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

    fn should_emit_lifecycle_reconnect(&mut self, observed_at_ms: f64) -> bool {
        if self
            .last_lifecycle_reconnect_proposal_at_ms
            .is_some_and(|last| {
                (observed_at_ms - last).max(0.0) < RECOVERING_RECONNECT_PROPOSAL_INTERVAL_MS
            })
        {
            return false;
        }
        self.last_lifecycle_reconnect_proposal_at_ms = Some(observed_at_ms);
        true
    }

    fn should_force_liveness_reconnect(
        &mut self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        observed_at_ms: f64,
    ) -> bool {
        let lifecycle_state = snapshot.connection.lifecycle_state;
        let in_recovery_surface = matches!(
            lifecycle_state,
            ConnectionLifecycleStateFact::New
                | ConnectionLifecycleStateFact::Connecting
                | ConnectionLifecycleStateFact::Recovering
        ) || (lifecycle_state == ConnectionLifecycleStateFact::Connected
            && owner_state != VideoSchedulingOwnerState::StableServing);
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

        let stalled_since = self
            .recovery_no_progress_since_ms
            .get_or_insert(observed_at_ms);
        // 首帧前统一走保守阈值，避免“无明显 transport 进展”被 4s 上界过早误杀。
        let fallback_threshold_ms = if snapshot.media.frame_count == 0 {
            RECOVERY_PRE_FIRST_FRAME_RECONNECT_FALLBACK_MS
        } else {
            RECOVERY_NO_PROGRESS_RECONNECT_FALLBACK_MS
        };
        observed_at_ms - *stalled_since >= fallback_threshold_ms
    }

    fn should_force_connected_render_stall_reconnect(
        &mut self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        observed_at_ms: f64,
    ) -> bool {
        if snapshot.connection.lifecycle_state != ConnectionLifecycleStateFact::Connected
            || owner_state == VideoSchedulingOwnerState::StableServing
        {
            self.reset_connected_render_stall_liveness();
            return false;
        }
        let signal = self.read_connected_render_liveness_signal();
        let present_progressed = match (
            self.connected_render_last_present_time_ms,
            signal.latest_video_present_time_ms,
        ) {
            (Some(last), Some(current)) => current > last,
            _ => false,
        };
        let ingress_progressed = self
            .connected_render_last_inbound_video_bytes_total
            .is_some_and(|last| signal.inbound_primary_video_bytes_total > last);
        self.connected_render_last_present_time_ms = signal.latest_video_present_time_ms;
        self.connected_render_last_inbound_video_bytes_total =
            Some(signal.inbound_primary_video_bytes_total);
        if present_progressed {
            self.connected_render_stall_since_ms = None;
            self.connected_render_stall_has_ingress_progress = false;
            return false;
        }
        let present_age_ms = signal
            .latest_video_present_time_ms
            .map(|ts| (observed_at_ms - ts).max(0.0));
        let present_stale = present_age_ms
            .is_some_and(|age| age >= CONNECTED_PRESENT_STALL_MIN_AGE_MS);
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
                latest_video_present_time_ms: stats.latest_video_present_time_ms,
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
        let (session_target_type, twcc_observation, session_phase) =
            RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
                (
                    stats.session_target_type.clone(),
                    stats.latest_video_twcc_observation.clone(),
                    parse_session_phase(stats.session_phase.as_deref()),
                )
            })
            .unwrap_or((None, None, SessionPhase::Steady));
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
            session_target_type.as_ref(),
            snapshot.connection.latest_transport_path.as_deref(),
            session_phase,
            None,
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
                stats
                    .latest_video_present_time_ms
                    .map(|ts| (now_ms - ts).max(0.0)),
                stats
                    .latest_video_decode_ok_time_ms
                    .map(|ts| (now_ms - ts).max(0.0)),
                stats.video_renderer_stalled.unwrap_or(false),
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
            None, None, None, None, false, None, None, None, None, None, None, None,
        ));
        SchedulingDemandSignal {
            no_pending_pressure_level,
            no_pending_streak,
            present_age_ms,
            decode_age_ms,
            video_renderer_stalled,
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
        let profile = resolve_recovery_profile(self.runtime_stats.as_ref());
        let recovery_epoch = RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
            stats.transport_recovery_epoch
        })
        .unwrap_or(0);
        let anchor_reason_label =
            RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
                stats
                    .latest_video_timeline_observation
                    .as_ref()
                    .and_then(|timeline| {
                        resolve_anchor_reason_label_from_timeline(
                            timeline.chain.state.as_str(),
                            timeline.chain.reason.as_deref(),
                            timeline.source_event.as_str(),
                        )
                    })
            })
            .unwrap_or(None);
        let owner_input = VideoSchedulingOwnerInput {
            connection_state: snapshot.connection.lifecycle_state,
            recovery_epoch,
            anchor_reason_label,
            demand,
            clean_anchor_epoch: RuntimeStatsSink::read_shared(
                self.runtime_stats.as_ref(),
                |stats| stats.video_anchor_clean_epoch,
            )
            .unwrap_or(None),
            clean_anchor_observed_at_ms: RuntimeStatsSink::read_shared(
                self.runtime_stats.as_ref(),
                |stats| stats.video_anchor_clean_observed_at_ms,
            )
            .unwrap_or(None),
            clean_anchor_source_event: RuntimeStatsSink::read_shared(
                self.runtime_stats.as_ref(),
                |stats| stats.video_anchor_clean_source_event.clone(),
            )
            .unwrap_or(None),
            latest_anchor_candidate_ledger: RuntimeStatsSink::read_shared(
                self.runtime_stats.as_ref(),
                |stats| stats.latest_anchor_candidate_ledger.clone(),
            )
            .unwrap_or(None),
            latest_timeline_chain_state: RuntimeStatsSink::read_shared(
                self.runtime_stats.as_ref(),
                |stats| {
                    stats
                        .latest_video_timeline_observation
                        .as_ref()
                        .map(|observation| observation.chain.state.clone())
                },
            )
            .unwrap_or(None),
            latest_timeline_source_event: RuntimeStatsSink::read_shared(
                self.runtime_stats.as_ref(),
                |stats| {
                    stats
                        .latest_video_timeline_observation
                        .as_ref()
                        .map(|observation| observation.source_event.clone())
                },
            )
            .unwrap_or(None),
            latest_track_state: RuntimeStatsSink::read_shared(
                self.runtime_stats.as_ref(),
                |stats| {
                    stats
                        .latest_video_track_status
                        .as_ref()
                        .map(|status| status.state.clone())
                },
            )
            .unwrap_or(None),
            latest_track_video_bytes_total: RuntimeStatsSink::read_shared(
                self.runtime_stats.as_ref(),
                |stats| {
                    stats
                        .latest_video_track_status
                        .as_ref()
                        .map(|status| status.video_bytes_total)
                },
            )
            .unwrap_or(None),
            display_supply_thresholds: profile.display_supply_thresholds,
            observed_at_ms: Self::resolve_policy_observed_at_ms(snapshot),
        };
        let owner_output = self.scheduling_owner.evaluate(&owner_input);
        let sink = RuntimeStatsSink::new(self.runtime_stats.clone());
        // canonical owner contract 由 owner state machine 直接写入 runtime stats；
        // recovery_coupling_mode 仍保留兼容用途，不再充当 owner 主语义。
        sink.update(|stats| {
            stats.video_owner_state = Some(owner_output.state.as_str().to_string());
            stats.video_owner_reason = Some(owner_output.reason_label.clone());
            stats.video_owner_source = Some(owner_output.reason_source.as_str().to_string());
            stats.video_owner_observed_at_ms = Some(owner_output.observed_at_ms);
        });
        if owner_output.state
            == crate::transport::rtc::policy::video_scheduling_owner::VideoSchedulingOwnerState::StableServing
            && owner_input
                .clean_anchor_epoch
                .is_some_and(|epoch| epoch == owner_input.recovery_epoch)
        {
            self.recovery_coordinator.acknowledge_clean_anchor();
        }
        owner_output
    }

    fn record_recovery_decision_ledger(
        &mut self,
        snapshot: &TransportSnapshot,
        owner_state: VideoSchedulingOwnerState,
        proposal: Option<&RecoveryPolicyProposal>,
    ) {
        // 避免在“上一条 ledger 的命令尚未落地（command_result=None）”时，
        // 被无信号/心跳类的刷新覆盖，导致命令结果无法回填到对应 decision_id。
        if proposal.is_none()
            && self.failed_terminal_since_ms.is_none()
            && RuntimeStatsSink::read_shared(self.runtime_stats.as_ref(), |stats| {
                stats
                    .latest_recovery_decision_ledger
                    .as_ref()
                    .is_some_and(|ledger| {
                        ledger.command_result.is_none() && ledger.action_selected != "none"
                    })
            })
            .unwrap_or(false)
        {
            return;
        }
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
        RuntimeStatsSink::new(self.runtime_stats.clone()).update(|stats| {
            stats.latest_recovery_decision_ledger =
                Some(XbxEngineRecoveryDecisionLedgerObservation {
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
                });
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
                VideoSchedulingOwnerState::StableServing => RecoveryLivenessState::Recovered,
                VideoSchedulingOwnerState::SeekingAnchor
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
        Some("startup") => SessionPhase::Startup,
        Some("recovering") => SessionPhase::Recovering,
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
            "displaySupplyCritical" => Some(VideoEscalationReason::AdapterIdleTimeout),
            "displaySupplyDegraded" => Some(VideoEscalationReason::AdapterThinStream),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::RtcSessionPolicy;
    use crate::api::backend::{XbxEngineMediaRuntimeStats, XbxEngineVideoTwccObservation};
    use crate::api::runtime::XbxEngineRuntimeConfig;
    use crate::transport::rtc::facts::{ConnectionLifecycleStateFact, TransportCommand};
    use crate::transport::rtc::policy::display_supply::SchedulingDemandSignal;
    use crate::transport::rtc::projection::{
        BweProjection, ConnectionProjection, DiagnosticsProjection, MediaProjection,
        RecoveryProjection, TransportSnapshot,
    };
    use crate::transport::rtc::session::actor::SessionPolicyHook;
    use std::sync::{Arc, Mutex};

    fn build_demand_for_stats(
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
    ) -> SchedulingDemandSignal {
        SchedulingDemandSignal {
            no_pending_pressure_level: stats.host_no_pending_pressure_level.clone(),
            no_pending_streak: Some(stats.host_no_pending_streak),
            present_age_ms: stats
                .latest_video_present_time_ms
                .map(|ts| (now_ms - ts).max(0.0)),
            decode_age_ms: stats
                .latest_video_decode_ok_time_ms
                .map(|ts| (now_ms - ts).max(0.0)),
            video_renderer_stalled: stats.video_renderer_stalled.unwrap_or(false),
            present_submit_count_total: Some(stats.video_present_submit_count_total),
            present_drop_count_total: Some(stats.video_present_drop_count_total),
            present_overwrite_count_total: Some(stats.video_present_overwrite_count_total),
            pacer_submit_count_total: Some(stats.video_pacer_submit_count_total),
            pacer_drop_count_total: Some(stats.video_pacer_drop_count_total),
            renderer_submit_count_total: Some(stats.video_renderer_submit_count_total),
            renderer_drop_count_total: Some(stats.video_renderer_drop_count_total),
        }
    }

    fn classify_supply_state_with_profile(
        stats: &XbxEngineMediaRuntimeStats,
    ) -> crate::transport::rtc::policy::display_supply::DisplaySupplyState {
        let profile = crate::transport::rtc::recovery::policy::ScenarioPolicyResolver::resolve_recovery_profile(
            stats.session_target_type.as_ref(),
            stats.transport_path.as_deref(),
        );
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let demand = build_demand_for_stats(stats, now_ms);
        demand.classify_display_supply_state(&profile.display_supply_thresholds)
    }

    #[test]
    fn reconnect_command_is_throttled_and_re_emitted_during_continuous_recovering() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        let mut recovery = RecoveryProjection::default();
        recovery.latest_diagnosis_label = Some("rtcPeerConnectionFailed".to_string());
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        let media = MediaProjection {
            frame_count: 1,
            ..MediaProjection::default()
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let second = TransportSnapshot::new(
            2,
            1_200.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let third = TransportSnapshot::new(
            3,
            2_701.0,
            connection,
            media,
            recovery,
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn fallback_transport_await_recovery_keyframe_is_not_blocked_before_coordinator() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let snapshot = build_snapshot(
            ConnectionLifecycleStateFact::Connected,
            "transportAwaitRecoveryKeyframe",
            100.0,
        );
        let commands = policy.on_snapshot(&snapshot);
        assert!(commands
            .iter()
            .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));
    }

    #[test]
    fn connecting_startup_without_progress_triggers_lifecycle_reconnect() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_phase = Some("startup".to_string());
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            MediaProjection::default(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let second = TransportSnapshot::new(
            2,
            4_200.0,
            connection.clone(),
            MediaProjection::default(),
            RecoveryProjection {
                last_observed_at_ms: Some(4_200.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let third = TransportSnapshot::new(
            3,
            15_600.0,
            connection.clone(),
            MediaProjection::default(),
            RecoveryProjection {
                last_observed_at_ms: Some(15_600.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let fourth = TransportSnapshot::new(
            4,
            16_200.0,
            connection.clone(),
            MediaProjection::default(),
            RecoveryProjection {
                last_observed_at_ms: Some(16_200.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let fourth_commands = policy.on_snapshot(&fourth);
        assert!(fourth_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let fifth = TransportSnapshot::new(
            5,
            17_200.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection {
                last_observed_at_ms: Some(17_200.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let fifth_commands = policy.on_snapshot(&fifth);
        assert!(fifth_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn connecting_seeking_anchor_without_progress_triggers_lifecycle_reconnect() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.video_owner_state = Some("seeking-anchor".to_string());
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            MediaProjection::default(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let second = TransportSnapshot::new(
            2,
            15_600.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection {
                last_observed_at_ms: Some(15_600.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn connecting_without_semantic_hints_still_triggers_liveness_reconnect() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            MediaProjection::default(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let second = TransportSnapshot::new(
            2,
            4_220.0,
            connection.clone(),
            MediaProjection::default(),
            RecoveryProjection {
                last_observed_at_ms: Some(4_220.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let third = TransportSnapshot::new(
            3,
            15_600.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection {
                last_observed_at_ms: Some(15_600.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn new_state_does_not_emit_liveness_reconnect_before_connecting() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::New;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            MediaProjection::default(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&first);

        let second = TransportSnapshot::new(
            2,
            10_000.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection {
                last_observed_at_ms: Some(10_000.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn lifecycle_reconnect_attempt_limit_enters_failed_terminal() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("rtcPeerConnectionFailed".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let media = MediaProjection {
            frame_count: 1,
            ..MediaProjection::default()
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let second = TransportSnapshot::new(
            2,
            2_000.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(2_000.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let third = TransportSnapshot::new(
            3,
            3_800.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(3_800.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let fourth = TransportSnapshot::new(
            4,
            5_600.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(5_600.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let fourth_commands = policy.on_snapshot(&fourth);
        assert!(
            fourth_commands.is_empty(),
            "attempts exhausted should enter failed-terminal without emitting more commands"
        );

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.state_after, "failed-terminal");
        assert_eq!(
            ledger.gate_result,
            "terminal:livenessReconnectAttemptLimitExceeded"
        );
        assert_eq!(ledger.action_selected, "failed-terminal");
        drop(stats);

        let fifth = TransportSnapshot::new(
            5,
            7_300.0,
            connection,
            media,
            RecoveryProjection {
                last_observed_at_ms: Some(7_300.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let fifth_commands = policy.on_snapshot(&fifth);
        assert!(fifth_commands.is_empty());
    }

    #[test]
    fn failed_terminal_clears_after_successful_progress_and_rearms_reconnect() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        let base_recovery = RecoveryProjection {
            latest_diagnosis_label: Some("rtcPeerConnectionFailed".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let media = MediaProjection {
            frame_count: 1,
            ..MediaProjection::default()
        };
        let timeline = [100.0, 2_000.0, 3_800.0, 5_600.0];
        for (idx, ts) in timeline.into_iter().enumerate() {
            let snapshot = TransportSnapshot::new(
                (idx as u64) + 1,
                ts,
                connection.clone(),
                media.clone(),
                RecoveryProjection {
                    last_observed_at_ms: Some(ts),
                    ..base_recovery.clone()
                },
                BweProjection::default(),
                DiagnosticsProjection::default(),
            );
            let _ = policy.on_snapshot(&snapshot);
        }
        {
            let stats = runtime_stats.lock().expect("runtime stats lock");
            let ledger = stats
                .latest_recovery_decision_ledger
                .as_ref()
                .expect("recovery decision ledger");
            assert_eq!(ledger.state_after, "failed-terminal");
        }

        let resumed = TransportSnapshot::new(
            5,
            7_800.0,
            connection,
            MediaProjection {
                frame_count: 2,
                ..media
            },
            RecoveryProjection {
                last_observed_at_ms: Some(7_800.0),
                ..base_recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let resumed_commands = policy.on_snapshot(&resumed);
        assert!(resumed_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.state_after, "reconnecting");
    }

    #[test]
    fn no_progress_upper_bound_applies_to_connecting_and_recovering_surfaces() {
        let cases = [
            (
                ConnectionLifecycleStateFact::Connecting,
                Some("none".to_string()),
            ),
            (
                ConnectionLifecycleStateFact::Recovering,
                Some("rtcPeerConnectionFailed".to_string()),
            ),
        ];
        for (idx, (lifecycle_state, diagnosis)) in cases.into_iter().enumerate() {
            let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
            let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
            let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
            let mut connection = ConnectionProjection::default();
            connection.lifecycle_state = lifecycle_state;
            let recovery = RecoveryProjection {
                latest_diagnosis_label: diagnosis,
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(100.0),
            };
            let media = MediaProjection {
                frame_count: if lifecycle_state == ConnectionLifecycleStateFact::Recovering {
                    1
                } else {
                    0
                },
                ..MediaProjection::default()
            };
            let first = TransportSnapshot::new(
                ((idx as u64) * 10) + 1,
                100.0,
                connection.clone(),
                media.clone(),
                recovery.clone(),
                BweProjection::default(),
                DiagnosticsProjection::default(),
            );
            let _ = policy.on_snapshot(&first);
            let second_ts = if lifecycle_state == ConnectionLifecycleStateFact::Connecting {
                15_600.0
            } else {
                4_300.0
            };
            let second = TransportSnapshot::new(
                ((idx as u64) * 10) + 2,
                second_ts,
                connection,
                media,
                RecoveryProjection {
                    last_observed_at_ms: Some(second_ts),
                    ..recovery
                },
                BweProjection::default(),
                DiagnosticsProjection::default(),
            );
            let second_commands = policy.on_snapshot(&second);
            assert!(
                second_commands.iter().any(|command| {
                    matches!(command, TransportCommand::RequestReconnectCandidate { .. })
                }),
                "case idx={} should emit reconnect under no-progress upper bound",
                idx
            );
        }
    }

    #[test]
    fn pre_first_frame_transport_progress_uses_relaxed_liveness_timeout() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
        connection.latest_transport_path = Some("Direct".to_string());
        connection.latest_rtt_ms = Some(9.0);
        let media = MediaProjection {
            frame_count: 0,
            ..MediaProjection::default()
        };
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&first);

        let second = TransportSnapshot::new(
            2,
            4_300.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(4_300.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(
            second_commands.iter().all(|command| {
                !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
            }),
            "transport 已有进展但尚未首帧时，不应在 4s 上界内过早重连"
        );

        let third = TransportSnapshot::new(
            3,
            15_600.0,
            connection,
            media,
            RecoveryProjection {
                last_observed_at_ms: Some(15_600.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn recovering_pre_first_frame_without_transport_progress_uses_relaxed_liveness_timeout() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        let media = MediaProjection {
            frame_count: 0,
            ..MediaProjection::default()
        };
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&first);

        let second = TransportSnapshot::new(
            2,
            4_300.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(4_300.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(
            second_commands.iter().all(|command| {
                !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
            }),
            "首帧前即便尚无 transport 进展，也不应在 4s 内过早重连"
        );

        let third = TransportSnapshot::new(
            3,
            15_600.0,
            connection,
            media,
            RecoveryProjection {
                last_observed_at_ms: Some(15_600.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn recovering_without_first_frame_does_not_emit_periodic_reconnect() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        let media = MediaProjection {
            frame_count: 0,
            ..MediaProjection::default()
        };
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("rtcPeerConnectionFailed".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&first);

        let second = TransportSnapshot::new(
            2,
            2_000.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                last_observed_at_ms: Some(2_000.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(
            second_commands.iter().all(|command| {
                !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
            }),
            "Recovering + 首帧前不应按 1.5s 节流周期反复触发 reconnect"
        );
    }

    #[test]
    fn liveness_uses_snapshot_now_when_last_observed_stalls() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let media = MediaProjection {
            frame_count: 0,
            ..MediaProjection::default()
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            media.clone(),
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&first);

        // 模拟 recovery.last_observed_at_ms 卡住不变，但 snapshot.now_ms 持续推进。
        let second = TransportSnapshot::new(
            2,
            15_600.0,
            connection,
            media,
            recovery,
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn command_success_without_frames_does_not_reset_liveness_budget() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
        let base_recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let media = MediaProjection {
            frame_count: 0,
            ..MediaProjection::default()
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            media.clone(),
            base_recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let second = TransportSnapshot::new(
            2,
            15_600.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                successful_action_count: 1,
                last_observed_at_ms: Some(15_600.0),
                ..base_recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let third = TransportSnapshot::new(
            3,
            17_500.0,
            connection.clone(),
            media.clone(),
            RecoveryProjection {
                successful_action_count: 2,
                last_observed_at_ms: Some(17_500.0),
                ..base_recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        let fourth = TransportSnapshot::new(
            4,
            19_400.0,
            connection.clone(),
            media,
            RecoveryProjection {
                successful_action_count: 3,
                last_observed_at_ms: Some(19_400.0),
                ..base_recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let fourth_commands = policy.on_snapshot(&fourth);
        assert!(
            fourth_commands.iter().any(|command| {
                matches!(command, TransportCommand::RequestReconnectCandidate { .. })
            }),
            "third no-progress reconnect is still allowed before terminal closes the loop"
        );

        let fifth = TransportSnapshot::new(
            5,
            21_300.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection {
                successful_action_count: 4,
                last_observed_at_ms: Some(21_300.0),
                ..RecoveryProjection::default()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let fifth_commands = policy.on_snapshot(&fifth);
        assert!(
            fifth_commands.is_empty(),
            "no media progress should still exhaust liveness attempts and stop reconnect loop"
        );
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.state_after, "failed-terminal");
        assert_eq!(
            ledger.gate_result,
            "terminal:livenessReconnectAttemptLimitExceeded"
        );
    }

    #[test]
    fn connected_ingress_progress_without_present_progress_triggers_liveness_reconnect() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 260;
            stats.latest_video_present_time_ms = Some(now_ms - 5_000.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4_000.0);
            stats.inbound_primary_video_bytes_total = 1_000;
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };

        let first = TransportSnapshot::new(
            1,
            100.0,
            connection.clone(),
            MediaProjection {
                frame_count: 10,
                ..MediaProjection::default()
            },
            recovery.clone(),
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        if let Ok(mut stats) = runtime_stats.lock() {
            stats.inbound_primary_video_bytes_total = 2_000;
        }
        let second = TransportSnapshot::new(
            2,
            5_000.0,
            connection.clone(),
            MediaProjection {
                frame_count: 11,
                ..MediaProjection::default()
            },
            RecoveryProjection {
                last_observed_at_ms: Some(5_000.0),
                ..recovery.clone()
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&second);
        assert!(second_commands.iter().all(|command| {
            !matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));

        if let Ok(mut stats) = runtime_stats.lock() {
            stats.inbound_primary_video_bytes_total = 3_000;
        }
        let third = TransportSnapshot::new(
            3,
            10_400.0,
            connection,
            MediaProjection {
                frame_count: 12,
                ..MediaProjection::default()
            },
            RecoveryProjection {
                last_observed_at_ms: Some(10_400.0),
                ..recovery
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let third_commands = policy.on_snapshot(&third);
        assert!(third_commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestReconnectCandidate { .. })
        }));
    }

    #[test]
    fn recovery_decision_ledger_is_written_with_budget_snapshot() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let snapshot = build_snapshot(
            ConnectionLifecycleStateFact::Connected,
            "transportAwaitRecoveryKeyframe",
            320.0,
        );
        let commands = policy.on_snapshot(&snapshot);
        assert!(commands
            .iter()
            .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));

        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportAwaitRecoveryKeyframe:transportAwaitRecoveryKeyframe"
        );
        assert_eq!(ledger.action_selected, "requestKeyframe");
        assert_eq!(ledger.gate_result, "pass");
        assert!(ledger.budget_before.is_some());
        assert!(ledger.budget_after.is_some());
        assert_eq!(ledger.command_result, None);
    }

    #[test]
    fn recovery_decision_ledger_still_updates_when_proposal_is_none() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

        let first = build_snapshot(
            ConnectionLifecycleStateFact::Connected,
            "transportAwaitRecoveryKeyframe",
            320.0,
        );
        let first_commands = policy.on_snapshot(&first);
        assert!(first_commands
            .iter()
            .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));
        let first_decision_id = runtime_stats
            .lock()
            .expect("runtime stats lock")
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger")
            .decision_id;

        // 下一 tick 明确无恢复信号时：
        // - 如果上一条 ledger 仍在等待 command_result 回填（command_result=None 且 action!=none），
        //   则必须保留该 ledger，避免覆盖导致 command_result 无法按 decision_id 回填。
        let second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 340.0);
        let _ = policy.on_snapshot(&second);
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.decision_id, first_decision_id);
        assert_eq!(
            ledger.input_signal,
            "transportAwaitRecoveryKeyframe:transportAwaitRecoveryKeyframe"
        );
        assert_eq!(ledger.gate_result, "pass");
        assert_eq!(ledger.action_selected, "requestKeyframe");
    }

    #[test]
    fn high_no_pending_but_fresh_present_does_not_force_keyframe() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 88;
            stats.latest_video_present_time_ms = Some(now_ms - 14.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 10.0);
            stats.video_renderer_stalled = Some(false);
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let snapshot = build_snapshot(
            ConnectionLifecycleStateFact::Connected,
            "waitKeyframeEntered",
            220.0,
        );
        let commands = policy.on_snapshot(&snapshot);
        assert!(commands
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })));
    }

    #[test]
    fn critical_display_supply_uses_recovery_controller_budget() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 220;
            stats.latest_video_present_time_ms = Some(now_ms - 980.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 520.0);
            stats.video_renderer_stalled = Some(true);
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut snapshot = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 360.0);
        let first = policy.on_snapshot(&snapshot);
        assert!(first
            .iter()
            .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));

        snapshot.version = 2;
        snapshot.now_ms = 361.0;
        snapshot.recovery.last_observed_at_ms = Some(361.0);
        let second = policy.on_snapshot(&snapshot);
        assert!(
            second
                .iter()
                .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })),
            "second snapshot should be suppressed by escalation cooldown budget"
        );
    }

    #[test]
    fn owner_contract_is_persisted_to_runtime_stats() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 240;
            stats.latest_video_present_time_ms = Some(now_ms - 1000.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 540.0);
            stats.video_renderer_stalled = Some(true);
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let snapshot = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 450.0);
        let _ = policy.on_snapshot(&snapshot);
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_owner_state.as_deref(), Some("supply-starved"));
        assert_eq!(stats.video_owner_source.as_deref(), Some("supply"));
        assert_eq!(
            stats.video_owner_reason.as_deref(),
            Some("displaySupplyCritical")
        );
        assert_eq!(stats.video_owner_observed_at_ms, Some(450.0));
    }

    #[test]
    fn recovery_intent_is_suppressed_within_same_epoch_via_coordinator_chain() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.transport_recovery_epoch = 2;
            stats.transport_recovery_epoch_at_last_escalation = 2;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 220;
            stats.latest_video_present_time_ms = Some(now_ms - 1200.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 600.0);
            stats.video_renderer_stalled = Some(true);
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut first = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 500.0);
        first.version = 1;
        let first_cmds = policy.on_snapshot(&first);
        assert!(first_cmds
            .iter()
            .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));

        let mut second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 501.0);
        second.version = 2;
        let second_cmds = policy.on_snapshot(&second);
        assert!(second_cmds
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })));
    }

    #[test]
    fn new_recovery_epoch_does_not_bypass_existing_recovery_suppression_chain() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.transport_recovery_epoch = 3;
            stats.transport_recovery_epoch_at_last_escalation = 3;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 240;
            stats.latest_video_present_time_ms = Some(now_ms - 1300.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 700.0);
            stats.video_renderer_stalled = Some(true);
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut first = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 700.0);
        first.version = 1;
        let first_cmds = policy.on_snapshot(&first);
        assert!(first_cmds
            .iter()
            .any(|command| matches!(command, TransportCommand::RequestKeyframe { .. })));

        let mut second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 701.0);
        second.version = 2;
        let second_cmds = policy.on_snapshot(&second);
        assert!(second_cmds
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })));

        if let Ok(mut stats) = runtime_stats.lock() {
            stats.transport_recovery_epoch = 4;
            stats.transport_recovery_epoch_at_last_escalation = 3;
        }
        let mut third = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 703.0);
        third.version = 3;
        let third_cmds = policy.on_snapshot(&third);
        assert!(third_cmds
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestKeyframe { .. })));
    }

    #[test]
    fn owner_contract_drives_display_supply_recovery_reason() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 260;
            stats.latest_video_present_time_ms = Some(now_ms - 1200.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 600.0);
            stats.video_renderer_stalled = Some(true);
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let snapshot = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 400.0);
        let commands = policy.on_snapshot(&snapshot);
        let reason = commands.into_iter().find_map(|cmd| match cmd {
            TransportCommand::RequestKeyframe { reason, .. } => Some(reason),
            _ => None,
        });
        assert_eq!(reason.as_deref(), Some("displaySupplyCritical"));
    }

    #[test]
    fn owner_does_not_enter_stable_serving_when_audio_only_and_no_pending_critical() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 980;
            stats.latest_video_present_time_ms = None;
            stats.latest_video_decode_ok_time_ms = None;
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "audioOnly".to_string(),
                video_width: None,
                video_height: None,
                mime_type: None,
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 0,
                video_packet_count_total: 0,
                audio_bytes_total: 128,
                observed_at_ms: 700.0,
            });
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let snapshot = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 700.0);
        let _ = policy.on_snapshot(&snapshot);
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_owner_state.as_deref(), Some("supply-starved"));
    }

    #[test]
    fn owner_keeps_rebuilding_supply_when_timeline_keeps_awaiting_recovery_keyframe() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 24;
            stats.latest_video_present_time_ms = Some(now_ms - 220.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 180.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: None,
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 120_000,
                video_packet_count_total: 900,
                audio_bytes_total: 32_000,
                observed_at_ms: 810.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 810.0,
                    },
                    observed_at_ms: 810.0,
                });
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let first = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 810.0);
        let _ = policy.on_snapshot(&first);

        let second = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 820.0);
        let _ = policy.on_snapshot(&second);
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            stats.video_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
        assert_eq!(
            stats.video_owner_reason.as_deref(),
            Some("transportAwaitRecoveryKeyframe")
        );
    }

    #[test]
    fn owner_anchor_reason_is_derived_from_timeline_chain_reason_not_recovery_diagnosis() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 8;
            stats.latest_video_present_time_ms = Some(now_ms - 240.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 180.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: None,
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 120_000,
                video_packet_count_total: 900,
                audio_bytes_total: 32_000,
                observed_at_ms: 910.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 11,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("awaitRecoveryKeyframe".to_string()),
                        observed_at_ms: 910.0,
                    },
                    observed_at_ms: 910.0,
                });
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let snapshot = build_snapshot(
            ConnectionLifecycleStateFact::Connected,
            "decoderBackendFailure",
            920.0,
        );
        let _ = policy.on_snapshot(&snapshot);
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            stats.video_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
        assert_eq!(
            stats.video_owner_reason.as_deref(),
            Some("transportAwaitRecoveryKeyframe")
        );
    }

    #[test]
    fn owner_exits_recovering_after_recovery_completion_evidence() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 30;
            stats.latest_video_present_time_ms = Some(now_ms - 240.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 170.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: None,
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 140_000,
                video_packet_count_total: 1000,
                audio_bytes_total: 36_000,
                observed_at_ms: 900.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-complete-candidate".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: 900.0,
                    },
                    observed_at_ms: 900.0,
                });
        }
        let recovering = build_snapshot(
            ConnectionLifecycleStateFact::Connected,
            "transportAwaitRecoveryKeyframe",
            900.0,
        );
        let _ = policy.on_snapshot(&recovering);

        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_streak = 0;
            stats.latest_video_present_time_ms = Some(now_ms - 18.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 12.0);
            stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 15.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
            }
        }
        let healed = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 930.0);
        let _ = policy.on_snapshot(&healed);
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
        assert_eq!(stats.video_owner_reason.as_deref(), Some("steady"));
    }

    #[test]
    fn frame_observed_without_clean_anchor_fact_cannot_exit_recovering() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 30;
            stats.latest_video_present_time_ms = Some(now_ms - 240.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 170.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: None,
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 140_000,
                video_packet_count_total: 1000,
                audio_bytes_total: 36_000,
                observed_at_ms: 900.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: 900.0,
                    },
                    observed_at_ms: 900.0,
                });
        }
        let recovering = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 900.0);
        let _ = policy.on_snapshot(&recovering);

        if let Ok(mut stats) = runtime_stats.lock() {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_streak = 0;
            stats.latest_video_present_time_ms = Some(now_ms - 18.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 12.0);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
            }
        }
        let healed = build_snapshot(ConnectionLifecycleStateFact::Connected, "none", 930.0);
        let _ = policy.on_snapshot(&healed);
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            stats.video_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
    }

    #[test]
    fn lifecycle_recovering_clears_stale_clean_anchor_fact() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.video_anchor_clean_epoch = Some(5);
            stats.video_anchor_clean_observed_at_ms = Some(1000.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        let snapshot = TransportSnapshot::new(
            1,
            1100.0,
            connection,
            MediaProjection {
                frame_count: 1,
                ..MediaProjection::default()
            },
            RecoveryProjection {
                latest_diagnosis_label: Some("rtcConnectionRecovering".to_string()),
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(1100.0),
            },
            BweProjection::default(),
            DiagnosticsProjection::default(),
        );
        let _ = policy.on_snapshot(&snapshot);
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_anchor_clean_epoch, None);
        assert_eq!(stats.video_anchor_clean_observed_at_ms, None);
        assert_eq!(stats.video_anchor_clean_source_event, None);
    }

    #[test]
    fn display_supply_thresholds_differ_between_home_and_cloud_profiles() {
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let base = XbxEngineMediaRuntimeStats {
            host_no_pending_pressure_level: Some("critical".to_string()),
            host_no_pending_streak: 100,
            latest_video_present_time_ms: Some(now_ms - 630.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 340.0),
            video_renderer_stalled: Some(false),
            ..XbxEngineMediaRuntimeStats::default()
        };
        let cloud_stats = XbxEngineMediaRuntimeStats {
            session_target_type: Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud),
            ..base.clone()
        };
        let home_stats = XbxEngineMediaRuntimeStats {
            session_target_type: Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home),
            transport_path: Some("direct".to_string()),
            ..base
        };

        assert_eq!(
            classify_supply_state_with_profile(&cloud_stats),
            crate::transport::rtc::policy::display_supply::DisplaySupplyState::Critical
        );
        assert_eq!(
            classify_supply_state_with_profile(&home_stats),
            crate::transport::rtc::policy::display_supply::DisplaySupplyState::Degraded
        );
    }

    #[test]
    fn decoder_backend_failure_can_emit_decoder_reset_command() {
        let mut policy = RtcSessionPolicy::default();
        let snapshot = build_snapshot(
            ConnectionLifecycleStateFact::Connected,
            "decoderBackendFailure",
            180.0,
        );
        let commands = policy.on_snapshot(&snapshot);
        assert!(commands
            .iter()
            .any(|command| matches!(command, TransportCommand::RequestDecoderReset { .. })));
    }

    #[test]
    fn bwe_tick_emits_target_remb_update_when_metrics_are_healthy() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        if let Ok(mut config) = runtime_config.lock() {
            config.webrtc.bwe_mode = "observed-remb".to_string();
        }
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.latest_loss_ratio_1s = Some(0.01);
        connection.latest_rtt_ms = Some(40.0);
        connection.latest_transport_path = Some("udp-direct".to_string());
        let bwe = BweProjection {
            latest_rtt_ms: Some(40.0),
            latest_loss_ratio_1s: Some(0.01),
            latest_actual_video_bitrate_kbps: Some(16_000.0),
            latest_observed_remb_kbps: Some(20_000),
            latest_transport_path: Some("udp-direct".to_string()),
            latest_sample_tick_ms: Some(300.0),
            target_remb_kbps: Some(16_000),
            last_observed_at_ms: Some(300.0),
        };
        let snapshot = TransportSnapshot::new(
            1,
            300.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection::default(),
            bwe,
            DiagnosticsProjection::default(),
        );
        let commands = policy.on_snapshot(&snapshot);
        let command = commands
            .into_iter()
            .find_map(|command| {
                if let TransportCommand::SetTargetRembKbps { target_kbps, .. } = command {
                    Some(target_kbps)
                } else {
                    None
                }
            })
            .unwrap_or(0);
        assert!(command > 16_000);
    }

    #[test]
    fn runtime_config_floor_is_respected() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        if let Ok(mut config) = runtime_config.lock() {
            config.webrtc.bwe_mode = "observed-remb".to_string();
            config.webrtc.remb_floor_kbps = 25_000;
            config.webrtc.remb_ceiling_kbps = 150_000;
        }
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.latest_loss_ratio_1s = Some(0.0);
        connection.latest_rtt_ms = Some(35.0);
        connection.latest_transport_path = Some("Direct".to_string());
        let bwe = BweProjection {
            latest_rtt_ms: Some(35.0),
            latest_loss_ratio_1s: Some(0.0),
            latest_actual_video_bitrate_kbps: Some(14_000.0),
            latest_observed_remb_kbps: Some(16_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(400.0),
            target_remb_kbps: Some(12_000),
            last_observed_at_ms: Some(400.0),
        };
        let snapshot = TransportSnapshot::new(
            2,
            400.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection::default(),
            bwe,
            DiagnosticsProjection::default(),
        );
        let target = policy
            .on_snapshot(&snapshot)
            .into_iter()
            .find_map(|command| {
                if let TransportCommand::SetTargetRembKbps { target_kbps, .. } = command {
                    Some(target_kbps)
                } else {
                    None
                }
            })
            .unwrap_or(0);
        assert_eq!(target, 25_000);
    }

    #[test]
    fn session_target_type_and_twcc_input_flow_into_new_bwe_policy() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        if let Ok(mut config) = runtime_config.lock() {
            config.webrtc.bwe_mode = "twcc-gcc".to_string();
            config.webrtc.remb_floor_kbps = 8_000;
            config.webrtc.remb_ceiling_kbps = 150_000;
        }
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
            stats.latest_video_twcc_observation = Some(XbxEngineVideoTwccObservation {
                observation_id: 1,
                source: "local-feedback".to_string(),
                feedback_packet_count: 3,
                covered_sequence_start: 100,
                covered_sequence_end: 120,
                covered_sequence_span: 20,
                observed_packet_count: 20,
                observed_byte_count: 30_000,
                coverage_ratio: None,
                ledger_hit_ratio: None,
                feedback_interval_ms: Some(80.0),
                arrival_span_ms: Some(70.0),
                receive_bitrate_kbps: Some(28_000.0),
                twcc_sample_valid: true,

                twcc_invalid_reason: None,

                quality: crate::XbxEngineTwccObservationQuality::Stable,
                delivery_ratio: 0.995,
                packet_loss_ratio: 0.0,
                observed_at_ms: 10.0,
            });
            stats.session_phase = Some("steady".to_string());
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.latest_loss_ratio_1s = Some(0.0);
        connection.latest_rtt_ms = Some(40.0);
        connection.latest_transport_path = Some("Direct".to_string());
        let bwe = BweProjection {
            latest_rtt_ms: Some(40.0),
            latest_loss_ratio_1s: Some(0.0),
            latest_actual_video_bitrate_kbps: Some(18_000.0),
            latest_observed_remb_kbps: Some(28_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(1.0),
            target_remb_kbps: Some(18_000),
            last_observed_at_ms: Some(1.0),
        };
        let snapshot = TransportSnapshot::new(
            1,
            1.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection::default(),
            bwe,
            DiagnosticsProjection::default(),
        );
        let reason = policy
            .on_snapshot(&snapshot)
            .into_iter()
            .find_map(|command| {
                if let TransportCommand::SetTargetRembKbps { reason, .. } = command {
                    Some(reason)
                } else {
                    None
                }
            });
        assert!(reason.is_some_and(|value| value.starts_with("twcc-gcc-cloud-")));
    }

    #[test]
    fn bwe_emits_reason_update_even_when_target_is_unchanged() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        if let Ok(mut config) = runtime_config.lock() {
            config.webrtc.bwe_mode = "twcc-gcc".to_string();
            config.webrtc.remb_floor_kbps = 8_000;
            config.webrtc.remb_ceiling_kbps = 50_000;
            config.webrtc.video_pipeline.feedback_interval_ms = 1_000;
        }
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
            stats.session_phase = Some("steady".to_string());
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
        policy.last_sent_remb_kbps = 25_000;
        policy.last_bwe_reason = Some("twcc-gcc-cloud-await-feedback".to_string());

        if let Ok(mut stats) = runtime_stats.lock() {
            stats.latest_video_twcc_observation = Some(XbxEngineVideoTwccObservation {
                observation_id: 1,
                source: "local-feedback".to_string(),
                feedback_packet_count: 3,
                covered_sequence_start: 100,
                covered_sequence_end: 220,
                covered_sequence_span: 120,
                observed_packet_count: 120,
                observed_byte_count: 180_000,
                coverage_ratio: None,
                ledger_hit_ratio: None,
                feedback_interval_ms: Some(1_000.0),
                arrival_span_ms: Some(1_000.0),
                receive_bitrate_kbps: Some(24_500.0),
                twcc_sample_valid: true,

                twcc_invalid_reason: None,

                quality: crate::XbxEngineTwccObservationQuality::Stable,
                delivery_ratio: 1.0,
                packet_loss_ratio: 0.0,
                observed_at_ms: 10.0,
            });
        }

        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.latest_loss_ratio_1s = Some(0.0);
        connection.latest_rtt_ms = Some(40.0);
        connection.latest_transport_path = Some("Direct".to_string());
        let bwe = BweProjection {
            latest_rtt_ms: Some(40.0),
            latest_loss_ratio_1s: Some(0.0),
            latest_actual_video_bitrate_kbps: Some(18_000.0),
            latest_observed_remb_kbps: Some(25_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(1.0),
            target_remb_kbps: Some(25_000),
            last_observed_at_ms: Some(1.0),
        };
        let snapshot = TransportSnapshot::new(
            1,
            1.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection::default(),
            bwe,
            DiagnosticsProjection::default(),
        );

        let reason = policy
            .on_snapshot(&snapshot)
            .into_iter()
            .find_map(|command| {
                if let TransportCommand::SetTargetRembKbps { reason, .. } = command {
                    Some(reason)
                } else {
                    None
                }
            });

        assert!(reason.is_some());
        assert_ne!(reason.as_deref(), Some("twcc-gcc-cloud-await-feedback"));
    }

    #[test]
    fn reconnect_keeps_priority_over_recovery_and_bwe() {
        let mut policy = RtcSessionPolicy::default();
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Recovering;
        connection.latest_loss_ratio_1s = Some(0.01);
        connection.latest_rtt_ms = Some(40.0);
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some("transportAwaitRecoveryKeyframe".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(100.0),
        };
        let bwe = BweProjection {
            latest_rtt_ms: Some(40.0),
            latest_loss_ratio_1s: Some(0.01),
            latest_actual_video_bitrate_kbps: Some(12_000.0),
            latest_observed_remb_kbps: Some(18_000),
            latest_transport_path: Some("udp-direct".to_string()),
            latest_sample_tick_ms: Some(100.0),
            target_remb_kbps: Some(12_000),
            last_observed_at_ms: Some(100.0),
        };
        let snapshot = TransportSnapshot::new(
            1,
            100.0,
            connection,
            MediaProjection {
                frame_count: 1,
                ..MediaProjection::default()
            },
            recovery,
            bwe,
            DiagnosticsProjection::default(),
        );
        let commands = policy.on_snapshot(&snapshot);
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            commands[0],
            TransportCommand::RequestReconnectCandidate { .. }
        ));
    }

    #[test]
    fn unstable_hold_requires_consecutive_confirmation_before_emit() {
        let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
        if let Ok(mut config) = runtime_config.lock() {
            config.webrtc.bwe_mode = "twcc-gcc".to_string();
            config.webrtc.video_pipeline.feedback_interval_ms = 100;
        }
        let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
            stats.session_phase = Some("steady".to_string());
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_present_time_ms = Some(now_ms - 12.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 10.0);
            stats.video_anchor_clean_epoch = Some(0);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 8.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: None,
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 64_000,
                video_packet_count_total: 1_200,
                audio_bytes_total: 32_000,
                observed_at_ms: now_ms,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: now_ms - 6.0,
                    },
                    observed_at_ms: now_ms - 6.0,
                });
            stats.latest_video_twcc_observation = Some(XbxEngineVideoTwccObservation {
                observation_id: 1,
                source: "local-feedback".to_string(),
                feedback_packet_count: 1,
                covered_sequence_start: 1,
                covered_sequence_end: 2,
                covered_sequence_span: 2,
                observed_packet_count: 1,
                observed_byte_count: 1200,
                coverage_ratio: None,
                ledger_hit_ratio: None,
                feedback_interval_ms: None,
                arrival_span_ms: None,
                receive_bitrate_kbps: Some(0.0),
                twcc_sample_valid: true,

                twcc_invalid_reason: None,

                quality: crate::XbxEngineTwccObservationQuality::Stable,
                delivery_ratio: 1.0,
                packet_loss_ratio: 0.0,
                observed_at_ms: 1.0,
            });
        }
        let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
        policy.last_sent_remb_kbps = 25_000;

        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
        connection.latest_transport_path = Some("Direct".to_string());
        let snapshot_first = TransportSnapshot::new(
            1,
            1.0,
            connection.clone(),
            MediaProjection::default(),
            RecoveryProjection::default(),
            BweProjection {
                latest_rtt_ms: Some(180.0),
                latest_loss_ratio_1s: Some(0.0),
                latest_actual_video_bitrate_kbps: Some(1_000.0),
                latest_observed_remb_kbps: Some(25_000),
                latest_transport_path: Some("Direct".to_string()),
                latest_sample_tick_ms: Some(1.0),
                target_remb_kbps: Some(25_000),
                last_observed_at_ms: Some(1.0),
            },
            DiagnosticsProjection::default(),
        );
        let first_commands = policy.on_snapshot(&snapshot_first);
        assert!(first_commands
            .iter()
            .all(|command| !matches!(command, TransportCommand::SetTargetRembKbps { .. })));

        let snapshot_second = TransportSnapshot::new(
            2,
            2.0,
            connection,
            MediaProjection::default(),
            RecoveryProjection::default(),
            BweProjection {
                latest_rtt_ms: Some(180.0),
                latest_loss_ratio_1s: Some(0.0),
                latest_actual_video_bitrate_kbps: Some(1_000.0),
                latest_observed_remb_kbps: Some(25_000),
                latest_transport_path: Some("Direct".to_string()),
                latest_sample_tick_ms: Some(2.0),
                target_remb_kbps: Some(25_000),
                last_observed_at_ms: Some(2.0),
            },
            DiagnosticsProjection::default(),
        );
        let second_commands = policy.on_snapshot(&snapshot_second);
        assert!(second_commands.iter().any(|command| {
            matches!(
                command,
                TransportCommand::SetTargetRembKbps { reason, .. }
                    if reason.contains("unstable-hold")
            )
        }));
    }

    fn build_snapshot(
        lifecycle_state: ConnectionLifecycleStateFact,
        diagnosis: &str,
        observed_at_ms: f64,
    ) -> TransportSnapshot {
        let mut connection = ConnectionProjection::default();
        connection.lifecycle_state = lifecycle_state;
        let recovery = RecoveryProjection {
            latest_diagnosis_label: Some(diagnosis.to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(observed_at_ms),
        };
        TransportSnapshot::new(
            1,
            observed_at_ms,
            connection,
            MediaProjection::default(),
            recovery,
            BweProjection::default(),
            DiagnosticsProjection::default(),
        )
    }
}
