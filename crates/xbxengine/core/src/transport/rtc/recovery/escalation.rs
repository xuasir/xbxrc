//! 视频恢复升级：`RecoveryAction` 与 `VideoEscalationReason`。
//! RFC `CostCeiling` 单调梯子：`Absorb`（等待/抑制/合并）→ `LocalRecover`（关键帧/解码器重置）
//! → `TransportRecover`（`RequestReconnectCandidate`）；门控在 `session::policy` / `ExpensiveRecoveryGate`。

use std::time::{Duration, Instant};

use crate::XbxEngineRecoveryReasonDomain;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryAction {
    WaitForBurst,
    WaitForDecoderResetBurst,
    CooldownSuppressed,
    CoalescedKeyframeInFlight,
    CoalescedDecoderResetInFlight,
    StartupGraceSuppressed,
    RequestPli,
    RequestFir,
    RequestDecoderReset,
    RequestReconnectCandidate,
}

impl RecoveryAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::WaitForBurst => "waitForBurst",
            Self::WaitForDecoderResetBurst => "waitForDecoderResetBurst",
            Self::CooldownSuppressed => "cooldownSuppressed",
            Self::CoalescedKeyframeInFlight => "coalesced:keyframeInFlight",
            Self::CoalescedDecoderResetInFlight => "coalesced:decoderResetInFlight",
            Self::StartupGraceSuppressed => "startupGraceSuppressed",
            Self::RequestPli => "requestPli",
            Self::RequestFir => "requestFir",
            Self::RequestDecoderReset => "requestDecoderReset",
            Self::RequestReconnectCandidate => "requestReconnectCandidate",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum KeyframeReasonClass {
    WaitKeyframe,
    TransportAwaitRecoveryKeyframe,
    DisplaySupplyCritical,
    AdapterIdleTimeout,
    AdapterThinStream,
    TransportExpiredDeadline,
    TransportRecoveredLate,
    TransportSampleLoss,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DecoderResetReasonClass {
    Reconfigure,
    DecoderBackendFailure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoEscalationReason {
    LifecycleRecovering,
    WaitKeyframe,
    TransportAwaitRecoveryKeyframe,
    DisplaySupplyCritical,
    Reconfigure,
    DecoderBackendFailure,
    AdapterIdleTimeout,
    AdapterThinStream,
    TransportExpiredDeadline,
    TransportSevereDeadline,
    TransportRecoveredLate,
    TransportSampleLoss,
}

impl VideoEscalationReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::LifecycleRecovering => "rtcConnectionRecovering",
            Self::WaitKeyframe => "waitKeyframe",
            Self::TransportAwaitRecoveryKeyframe => "transportAwaitRecoveryAnchor",
            Self::DisplaySupplyCritical => "displaySupplyCritical",
            Self::Reconfigure => "reconfigure",
            Self::DecoderBackendFailure => "decoderBackendFailure",
            Self::AdapterIdleTimeout => "adapterIdleTimeout",
            Self::AdapterThinStream => "adapterThinStream",
            Self::TransportExpiredDeadline => "transportExpiredDeadline",
            Self::TransportSevereDeadline => "transportSevereDeadline",
            Self::TransportRecoveredLate => "transportRecoveredLate",
            Self::TransportSampleLoss => "transportSampleLoss",
        }
    }

    pub fn reconnect_domain(self) -> XbxEngineRecoveryReasonDomain {
        match self {
            Self::LifecycleRecovering => XbxEngineRecoveryReasonDomain::ConnectivityTransport,
            Self::WaitKeyframe
            | Self::TransportAwaitRecoveryKeyframe
            | Self::DisplaySupplyCritical
            | Self::Reconfigure
            | Self::DecoderBackendFailure
            | Self::AdapterIdleTimeout
            | Self::AdapterThinStream => XbxEngineRecoveryReasonDomain::Local,
            Self::TransportExpiredDeadline
            | Self::TransportSevereDeadline
            | Self::TransportRecoveredLate
            | Self::TransportSampleLoss => XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        }
    }

    pub fn from_recovery_reason_label(label: &str) -> Option<Self> {
        match label {
            "rtcConnectionRecovering" => Some(Self::LifecycleRecovering),
            "waitKeyframe"
            | "ingressWaitKeyframe"
            | "ingressFrameAbandoned"
            | "waitKeyframeEntered"
            | "frameAbandoned" => Some(Self::WaitKeyframe),
            "transportAwaitRecoveryAnchor"
            | "bootstrapMissingSps"
            | "bootstrapMissingPps"
            | "inspectionRejectInvalidSliceHeader" => Some(Self::TransportAwaitRecoveryKeyframe),
            "displaySupplyCritical" | "hostPresentStalled" => Some(Self::DisplaySupplyCritical),
            "displaySupplyDegraded" | "adapterThinStream" => Some(Self::AdapterThinStream),
            "ingressReconfigure" | "reconfigure" => Some(Self::Reconfigure),
            "decoderBackendFailure" => Some(Self::DecoderBackendFailure),
            "adapterIdleTimeout" => Some(Self::AdapterIdleTimeout),
            "transportExpiredDeadline" => Some(Self::TransportExpiredDeadline),
            "transportSevereDeadline" => Some(Self::TransportSevereDeadline),
            "transportRecoveredLate" => Some(Self::TransportRecoveredLate),
            "transportSampleLoss" => Some(Self::TransportSampleLoss),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryActionOwner {
    Nack,
    Pli,
    Fir,
    DecoderReset,
    Reconnect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryBudgetKind {
    Keyframe,
    DecoderReset,
    Reconnect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryActionContract {
    pub owner: Option<RecoveryActionOwner>,
    pub budget_kind: Option<RecoveryBudgetKind>,
    pub budget_recorded_on_execution: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryActionBudgetState {
    pub recovery_epoch: u64,
    pub keyframe_budget_used: u8,
    pub keyframe_budget_limit: u8,
    pub decoder_reset_budget_used: u8,
    pub decoder_reset_budget_limit: u8,
    pub reconnect_budget_used: u8,
    pub reconnect_budget_limit: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct VideoEscalationConfig {
    pub cooldown_ms: u64,
    pub keyframe_burst_threshold: u8,
    pub decoder_reset_burst_threshold: u8,
    pub keyframe_min_interval_ms: u64,
    pub escalation_window_ms: u64,
    pub keyframe_upgrade_min_delay_ms: u64,
}

impl Default for VideoEscalationConfig {
    fn default() -> Self {
        Self {
            cooldown_ms: 320,
            keyframe_burst_threshold: 2,
            decoder_reset_burst_threshold: 3,
            keyframe_min_interval_ms: 0,
            escalation_window_ms: 960,
            keyframe_upgrade_min_delay_ms: 0,
        }
    }
}

pub struct VideoEscalationController {
    cooldown: Duration,
    burst_window: Duration,
    keyframe_min_interval: Duration,
    escalation_window: Duration,
    keyframe_upgrade_min_delay: Duration,
    keyframe_burst_threshold: u8,
    decoder_reset_burst_threshold: u8,
    severe_deadline_reconnect_window: Duration,
    pending_keyframe_signals: u8,
    pending_decoder_reset_signals: u8,
    reconnect_candidate_signals: u8,
    last_keyframe_request_at: Option<Instant>,
    last_decoder_reset_at: Option<Instant>,
    last_severe_deadline_at: Option<Instant>,
    last_keyframe_signal_at: Option<Instant>,
    last_decoder_reset_signal_at: Option<Instant>,
    last_keyframe_reason_class: Option<KeyframeReasonClass>,
    last_decoder_reset_reason_class: Option<DecoderResetReasonClass>,
    wait_keyframe_started_at: Option<Instant>,
    transport_await_recovery_started_at: Option<Instant>,
    transport_deadline_window_started_at: Option<Instant>,
    transport_deadline_window_count: u8,
    keyframe_epoch_active: bool,
    keyframe_epoch_started_at: Option<Instant>,
    keyframe_epoch_reason_class: Option<KeyframeReasonClass>,
    keyframe_budget_reservation_active: bool,
    recovery_epoch: u64,
    keyframe_budget_used: u8,
    decoder_reset_budget_used: u8,
    reconnect_budget_used: u8,
    keyframe_budget_limit: u8,
    decoder_reset_budget_limit: u8,
    reconnect_budget_limit: u8,
    next_observation_id: u64,
}

const CONNECTIVITY_DEADLINE_RECONNECT_HIT_THRESHOLD: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoEscalationDecision {
    pub observation_id: u64,
    pub action: RecoveryAction,
}

/// `RecoveryCoordinator` 可能在 `on_reason_with_epoch_policy` 之后把决策压成 WaitForBurst /
impl VideoEscalationController {
    pub fn new(config: VideoEscalationConfig) -> Self {
        let cooldown = Duration::from_millis(config.cooldown_ms.max(20));
        let cooldown_ms = cooldown.as_millis() as u64;
        let keyframe_min_interval_ms = if config.keyframe_min_interval_ms == 0 {
            cooldown_ms
        } else {
            config.keyframe_min_interval_ms.max(20)
        };
        let escalation_window_ms = if config.escalation_window_ms == 0 {
            cooldown_ms.saturating_mul(3)
        } else {
            config
                .escalation_window_ms
                .max(cooldown_ms.saturating_mul(2))
        };
        let keyframe_upgrade_min_delay_ms = config
            .keyframe_upgrade_min_delay_ms
            .min(keyframe_min_interval_ms);
        let keyframe_min_interval = Duration::from_millis(keyframe_min_interval_ms);
        let escalation_window = Duration::from_millis(escalation_window_ms);
        let keyframe_upgrade_min_delay = Duration::from_millis(keyframe_upgrade_min_delay_ms);
        Self {
            cooldown,
            burst_window: cooldown.clamp(Duration::from_millis(200), Duration::from_millis(350)),
            keyframe_min_interval,
            escalation_window,
            keyframe_upgrade_min_delay,
            keyframe_burst_threshold: config.keyframe_burst_threshold.max(1),
            decoder_reset_burst_threshold: config.decoder_reset_burst_threshold.max(1),
            severe_deadline_reconnect_window: cooldown.mul_f32(3.0),
            pending_keyframe_signals: 0,
            pending_decoder_reset_signals: 0,
            reconnect_candidate_signals: 0,
            last_keyframe_request_at: None,
            last_decoder_reset_at: None,
            last_severe_deadline_at: None,
            last_keyframe_signal_at: None,
            last_decoder_reset_signal_at: None,
            last_keyframe_reason_class: None,
            last_decoder_reset_reason_class: None,
            wait_keyframe_started_at: None,
            transport_await_recovery_started_at: None,
            transport_deadline_window_started_at: None,
            transport_deadline_window_count: 0,
            keyframe_epoch_active: false,
            keyframe_epoch_started_at: None,
            keyframe_epoch_reason_class: None,
            keyframe_budget_reservation_active: false,
            recovery_epoch: 0,
            keyframe_budget_used: 0,
            decoder_reset_budget_used: 0,
            reconnect_budget_used: 0,
            keyframe_budget_limit: config.keyframe_burst_threshold.max(1).saturating_add(1),
            decoder_reset_budget_limit: config.decoder_reset_burst_threshold.max(1),
            reconnect_budget_limit: 1,
            next_observation_id: 0,
        }
    }

    /// 手动重置 keyframe epoch（例如会话确认恢复到稳定 healthy 时）。
    pub fn reset_keyframe_epoch(&mut self) {
        self.clear_keyframe_epoch();
    }

    pub fn acknowledge_stable_recovery(&mut self) {
        self.pending_keyframe_signals = 0;
        self.pending_decoder_reset_signals = 0;
        self.reconnect_candidate_signals = 0;
        self.last_keyframe_request_at = None;
        self.last_decoder_reset_at = None;
        self.last_severe_deadline_at = None;
        self.last_keyframe_signal_at = None;
        self.last_decoder_reset_signal_at = None;
        self.last_keyframe_reason_class = None;
        self.last_decoder_reset_reason_class = None;
        self.wait_keyframe_started_at = None;
        self.transport_await_recovery_started_at = None;
        self.transport_deadline_window_started_at = None;
        self.transport_deadline_window_count = 0;
        self.clear_keyframe_epoch();
        self.keyframe_budget_reservation_active = false;
    }

    pub fn begin_recovery_epoch(&mut self, recovery_epoch: u64) {
        if self.recovery_epoch == recovery_epoch {
            return;
        }
        self.recovery_epoch = recovery_epoch;
        self.pending_keyframe_signals = 0;
        self.pending_decoder_reset_signals = 0;
        self.reconnect_candidate_signals = 0;
        self.wait_keyframe_started_at = None;
        self.transport_await_recovery_started_at = None;
        self.transport_deadline_window_started_at = None;
        self.transport_deadline_window_count = 0;
        self.last_keyframe_request_at = None;
        self.last_decoder_reset_at = None;
        self.last_severe_deadline_at = None;
        self.last_keyframe_signal_at = None;
        self.last_decoder_reset_signal_at = None;
        self.last_keyframe_reason_class = None;
        self.last_decoder_reset_reason_class = None;
        self.keyframe_budget_used = 0;
        self.decoder_reset_budget_used = 0;
        self.reconnect_budget_used = 0;
        self.clear_keyframe_epoch();
        self.keyframe_budget_reservation_active = false;
    }

    pub fn on_reason_with_epoch(
        &mut self,
        reason: VideoEscalationReason,
        recovery_epoch: u64,
    ) -> VideoEscalationDecision {
        self.on_reason_with_epoch_policy(reason, recovery_epoch, true, true, true, true)
    }

    pub fn on_reason_with_epoch_policy(
        &mut self,
        reason: VideoEscalationReason,
        recovery_epoch: u64,
        allow_reconnect: bool,
        allow_transport_await_stage_escalation: bool,
        allow_wait_keyframe_stage_escalation: bool,
        allow_reconfigure_stage_escalation: bool,
    ) -> VideoEscalationDecision {
        self.begin_recovery_epoch(recovery_epoch);
        self.on_reason_with_policy_controlled(
            reason,
            allow_reconnect,
            allow_transport_await_stage_escalation,
            allow_wait_keyframe_stage_escalation,
            allow_reconfigure_stage_escalation,
        )
    }

    pub fn reopen_transport_await_keyframe(
        &mut self,
        recovery_epoch: u64,
    ) -> VideoEscalationDecision {
        self.begin_recovery_epoch(recovery_epoch);
        self.next_observation_id = self.next_observation_id.saturating_add(1);
        let now = Instant::now();
        self.pending_keyframe_signals = 0;
        self.pending_decoder_reset_signals = 0;
        self.reconnect_candidate_signals = 0;
        self.wait_keyframe_started_at = None;
        self.transport_await_recovery_started_at = Some(now);
        self.last_keyframe_signal_at = Some(now);
        self.last_keyframe_reason_class = Some(KeyframeReasonClass::TransportAwaitRecoveryKeyframe);
        self.clear_keyframe_epoch();
        self.keyframe_budget_reservation_active = false;
        let action = if self.can_allocate_keyframe_attempt() {
            self.last_keyframe_request_at = Some(now);
            RecoveryAction::RequestPli
        } else {
            RecoveryAction::CooldownSuppressed
        };
        VideoEscalationDecision {
            observation_id: self.next_observation_id,
            action,
        }
    }

    pub fn budget_state(&self) -> RecoveryActionBudgetState {
        RecoveryActionBudgetState {
            recovery_epoch: self.recovery_epoch,
            keyframe_budget_used: self.keyframe_budget_used,
            keyframe_budget_limit: self.keyframe_budget_limit,
            decoder_reset_budget_used: self.decoder_reset_budget_used,
            decoder_reset_budget_limit: self.decoder_reset_budget_limit,
            reconnect_budget_used: self.reconnect_budget_used,
            reconnect_budget_limit: self.reconnect_budget_limit,
        }
    }

    pub fn action_owner(action: RecoveryAction) -> Option<RecoveryActionOwner> {
        Self::action_contract(action).owner
    }

    pub fn action_contract(action: RecoveryAction) -> RecoveryActionContract {
        match action {
            RecoveryAction::RequestPli => RecoveryActionContract {
                owner: Some(RecoveryActionOwner::Pli),
                budget_kind: Some(RecoveryBudgetKind::Keyframe),
                budget_recorded_on_execution: true,
            },
            RecoveryAction::RequestFir => RecoveryActionContract {
                owner: Some(RecoveryActionOwner::Fir),
                budget_kind: Some(RecoveryBudgetKind::Keyframe),
                budget_recorded_on_execution: true,
            },
            RecoveryAction::RequestDecoderReset => RecoveryActionContract {
                owner: Some(RecoveryActionOwner::DecoderReset),
                budget_kind: Some(RecoveryBudgetKind::DecoderReset),
                budget_recorded_on_execution: true,
            },
            RecoveryAction::RequestReconnectCandidate => RecoveryActionContract {
                owner: Some(RecoveryActionOwner::Reconnect),
                budget_kind: Some(RecoveryBudgetKind::Reconnect),
                budget_recorded_on_execution: true,
            },
            RecoveryAction::WaitForBurst
            | RecoveryAction::WaitForDecoderResetBurst
            | RecoveryAction::CooldownSuppressed
            | RecoveryAction::CoalescedKeyframeInFlight
            | RecoveryAction::CoalescedDecoderResetInFlight
            | RecoveryAction::StartupGraceSuppressed => RecoveryActionContract {
                owner: None,
                budget_kind: None,
                budget_recorded_on_execution: false,
            },
        }
    }

    pub fn action_success_advances_transport_recovery_epoch(
        action: RecoveryAction,
        _reason: Option<VideoEscalationReason>,
    ) -> bool {
        match action {
            RecoveryAction::RequestReconnectCandidate => true,
            RecoveryAction::RequestDecoderReset => false,
            RecoveryAction::RequestPli
            | RecoveryAction::RequestFir
            | RecoveryAction::WaitForBurst
            | RecoveryAction::WaitForDecoderResetBurst
            | RecoveryAction::CooldownSuppressed
            | RecoveryAction::CoalescedKeyframeInFlight
            | RecoveryAction::CoalescedDecoderResetInFlight
            | RecoveryAction::StartupGraceSuppressed => false,
        }
    }

    pub fn register_action_applied(&mut self, action: RecoveryAction) {
        match action {
            RecoveryAction::RequestPli | RecoveryAction::RequestFir => {
                self.keyframe_budget_reservation_active = true;
            }
            RecoveryAction::RequestDecoderReset => {}
            RecoveryAction::RequestReconnectCandidate => {}
            RecoveryAction::WaitForBurst
            | RecoveryAction::WaitForDecoderResetBurst
            | RecoveryAction::CooldownSuppressed
            | RecoveryAction::CoalescedKeyframeInFlight
            | RecoveryAction::CoalescedDecoderResetInFlight
            | RecoveryAction::StartupGraceSuppressed => {}
        }
    }

    pub fn register_reconnect_started(&mut self) {
        self.reconnect_budget_used = self.reconnect_budget_used.saturating_add(1);
        self.pending_keyframe_signals = 0;
        self.pending_decoder_reset_signals = 0;
        self.reconnect_candidate_signals = 0;
        self.clear_keyframe_epoch();
        self.keyframe_budget_reservation_active = false;
    }

    pub fn register_decoder_reset_started(&mut self) {
        let now = Instant::now();
        self.decoder_reset_budget_used = self.decoder_reset_budget_used.saturating_add(1);
        self.last_decoder_reset_at = Some(now);
        self.last_keyframe_request_at = Some(now);
        self.clear_keyframe_epoch();
        self.keyframe_budget_reservation_active = false;
    }

    pub fn reconcile_keyframe_transport_feedback(&mut self, feedback: KeyframeTransportFeedback) {
        match feedback {
            KeyframeTransportFeedback::UnsentPending => {
                self.keyframe_budget_reservation_active = false;
                // requested 但尚未 sent 不应继续占着 keyframe epoch，否则 owner 会被假在飞态锁住。
                self.clear_keyframe_epoch();
            }
            KeyframeTransportFeedback::SentPending => {
                if self.keyframe_budget_reservation_active {
                    self.keyframe_budget_used = self.keyframe_budget_used.saturating_add(1);
                }
                self.keyframe_budget_reservation_active = false;
            }
            KeyframeTransportFeedback::Terminal | KeyframeTransportFeedback::None => {
                self.keyframe_budget_reservation_active = false;
                // terminal 代表这一轮 family 已经结束；
                // 不论是成功、deferred、packet-seen 还是失败，都不能继续占着 in-flight family。
                self.clear_keyframe_epoch();
            }
        }
    }

    pub fn on_reason(&mut self, reason: VideoEscalationReason) -> VideoEscalationDecision {
        self.on_reason_with_policy(reason, true)
    }

    pub fn on_reason_with_policy(
        &mut self,
        reason: VideoEscalationReason,
        allow_reconnect: bool,
    ) -> VideoEscalationDecision {
        self.on_reason_with_policy_controlled(reason, allow_reconnect, true, true, true)
    }

    fn on_reason_with_policy_controlled(
        &mut self,
        reason: VideoEscalationReason,
        allow_reconnect: bool,
        allow_transport_await_stage_escalation: bool,
        allow_wait_keyframe_stage_escalation: bool,
        allow_reconfigure_stage_escalation: bool,
    ) -> VideoEscalationDecision {
        self.next_observation_id = self.next_observation_id.saturating_add(1);
        let now = Instant::now();
        let allow_reconnect = allow_reconnect && reason_allows_connectivity_fallback(reason);
        let action = match reason {
            VideoEscalationReason::LifecycleRecovering => {
                self.wait_keyframe_started_at = None;
                self.transport_await_recovery_started_at = None;
                self.pending_keyframe_signals = 0;
                self.pending_decoder_reset_signals = 0;
                self.reconnect_candidate_signals = 0;
                self.clear_keyframe_epoch();
                self.resolve_reconnect_or_decoder_reset_fallback(
                    now,
                    allow_reconnect,
                    VideoEscalationReason::LifecycleRecovering,
                )
            }
            VideoEscalationReason::WaitKeyframe
            | VideoEscalationReason::TransportAwaitRecoveryKeyframe
            | VideoEscalationReason::DisplaySupplyCritical
            | VideoEscalationReason::AdapterIdleTimeout
            | VideoEscalationReason::AdapterThinStream
            | VideoEscalationReason::TransportExpiredDeadline
            | VideoEscalationReason::TransportRecoveredLate
            | VideoEscalationReason::TransportSampleLoss => {
                let reason_class = match reason {
                    VideoEscalationReason::WaitKeyframe => KeyframeReasonClass::WaitKeyframe,
                    VideoEscalationReason::TransportAwaitRecoveryKeyframe => {
                        KeyframeReasonClass::TransportAwaitRecoveryKeyframe
                    }
                    VideoEscalationReason::DisplaySupplyCritical => {
                        KeyframeReasonClass::DisplaySupplyCritical
                    }
                    VideoEscalationReason::AdapterIdleTimeout => {
                        KeyframeReasonClass::AdapterIdleTimeout
                    }
                    VideoEscalationReason::AdapterThinStream => {
                        KeyframeReasonClass::AdapterThinStream
                    }
                    VideoEscalationReason::TransportExpiredDeadline => {
                        KeyframeReasonClass::TransportExpiredDeadline
                    }
                    VideoEscalationReason::TransportRecoveredLate => {
                        KeyframeReasonClass::TransportRecoveredLate
                    }
                    VideoEscalationReason::TransportSampleLoss => {
                        KeyframeReasonClass::TransportSampleLoss
                    }
                    _ => unreachable!(),
                };
                let immediate_keyframe_reason = matches!(
                    reason,
                    VideoEscalationReason::TransportAwaitRecoveryKeyframe
                        | VideoEscalationReason::DisplaySupplyCritical
                        | VideoEscalationReason::TransportExpiredDeadline
                        | VideoEscalationReason::TransportSampleLoss
                        | VideoEscalationReason::AdapterIdleTimeout
                        | VideoEscalationReason::AdapterThinStream
                );
                let severe_deadline_reconnect =
                    matches!(reason, VideoEscalationReason::AdapterIdleTimeout)
                        && self.last_severe_deadline_at.map_or(false, |last| {
                            last.elapsed() <= self.severe_deadline_reconnect_window
                        });
                if severe_deadline_reconnect {
                    self.pending_keyframe_signals = 0;
                    self.pending_decoder_reset_signals = 0;
                    self.reconnect_candidate_signals =
                        self.reconnect_candidate_signals.saturating_add(1);
                    self.clear_keyframe_epoch();
                    // severe deadline 窗口内的 idle 按连接域坏窗处理，优先 reconnect，禁止退回本地 decoder reset。
                    self.resolve_reconnect_or_decoder_reset_fallback(now, true, reason)
                } else {
                    if matches!(reason, VideoEscalationReason::WaitKeyframe) {
                        // WaitKeyframe 的“持续时长”要跨 burst 保留，
                        // 这里仅在原因类型变化时重置起点。
                        if self.last_keyframe_reason_class != Some(reason_class) {
                            self.wait_keyframe_started_at = Some(now);
                        } else {
                            self.wait_keyframe_started_at.get_or_insert(now);
                        }
                    } else {
                        self.wait_keyframe_started_at = None;
                    }
                    if matches!(
                        reason,
                        VideoEscalationReason::TransportAwaitRecoveryKeyframe
                    ) {
                        if self.last_keyframe_reason_class != Some(reason_class) {
                            self.transport_await_recovery_started_at = Some(now);
                        } else {
                            self.transport_await_recovery_started_at.get_or_insert(now);
                        }
                    } else {
                        self.transport_await_recovery_started_at = None;
                    }
                    if self
                        .last_keyframe_signal_at
                        .map_or(true, |last| last.elapsed() > self.burst_window)
                        || self.last_keyframe_reason_class != Some(reason_class)
                    {
                        self.pending_keyframe_signals = 0;
                    }
                    self.last_keyframe_signal_at = Some(now);
                    self.last_keyframe_reason_class = Some(reason_class);
                    self.pending_keyframe_signals = if immediate_keyframe_reason {
                        self.keyframe_burst_threshold
                    } else {
                        self.pending_keyframe_signals.saturating_add(1)
                    };
                    self.pending_decoder_reset_signals = 0;
                    let persistent_wait_keyframe =
                        matches!(reason, VideoEscalationReason::WaitKeyframe)
                            && self.wait_keyframe_started_at.map_or(false, |started_at| {
                                now.duration_since(started_at)
                                    >= self
                                        .cooldown
                                        .mul_f32(2.0)
                                        .max(self.keyframe_upgrade_min_delay)
                            });
                    let persistent_transport_await_recovery_keyframe = matches!(
                        reason,
                        VideoEscalationReason::TransportAwaitRecoveryKeyframe
                    ) && self
                        .transport_await_recovery_started_at
                        .map_or(false, |started_at| {
                            now.duration_since(started_at)
                                >= self.escalation_window.max(self.keyframe_upgrade_min_delay)
                        });
                    let hard_stuck_transport_await_recovery_keyframe = matches!(
                        reason,
                        VideoEscalationReason::TransportAwaitRecoveryKeyframe
                    ) && self
                        .transport_await_recovery_started_at
                        .map_or(false, |started_at| {
                            now.duration_since(started_at)
                                >= (self.escalation_window + self.cooldown.mul_f32(2.0))
                        });
                    if matches!(reason, VideoEscalationReason::TransportExpiredDeadline) {
                        let reset_deadline_windows = self
                            .transport_deadline_window_started_at
                            .map_or(true, |started_at| {
                                now.duration_since(started_at)
                                    > self.severe_deadline_reconnect_window
                            });
                        let new_deadline_window = self
                            .transport_deadline_window_started_at
                            .map_or(true, |started_at| {
                                now.duration_since(started_at) >= self.cooldown
                            });
                        if reset_deadline_windows {
                            self.transport_deadline_window_count = 0;
                        }
                        if new_deadline_window {
                            self.transport_deadline_window_started_at = Some(now);
                            self.transport_deadline_window_count =
                                self.transport_deadline_window_count.saturating_add(1);
                        }
                        if self.transport_deadline_window_count
                            >= CONNECTIVITY_DEADLINE_RECONNECT_HIT_THRESHOLD
                        {
                            self.pending_keyframe_signals = 0;
                            self.pending_decoder_reset_signals = 0;
                            self.reconnect_candidate_signals =
                                self.reconnect_candidate_signals.saturating_add(1);
                            self.clear_keyframe_epoch();
                            self.resolve_reconnect_or_decoder_reset_fallback(
                                now,
                                allow_reconnect,
                                VideoEscalationReason::TransportExpiredDeadline,
                            )
                        } else if self
                            .last_keyframe_request_at
                            .map_or(true, |last| last.elapsed() >= self.cooldown)
                            && self.try_enter_keyframe_epoch(reason_class, now)
                        {
                            self.last_keyframe_request_at = Some(now);
                            self.pending_keyframe_signals = 0;
                            self.reconnect_candidate_signals = 0;
                            if self.can_allocate_keyframe_attempt() {
                                RecoveryAction::RequestPli
                            } else {
                                self.coalesced_keyframe_in_flight()
                            }
                        } else {
                            self.coalesced_keyframe_in_flight()
                        }
                    } else if persistent_wait_keyframe
                        && allow_wait_keyframe_stage_escalation
                        && self
                            .last_decoder_reset_at
                            .map_or(true, |last| last.elapsed() >= self.cooldown)
                    {
                        self.pending_keyframe_signals = 0;
                        self.pending_decoder_reset_signals = 0;
                        self.reconnect_candidate_signals =
                            self.reconnect_candidate_signals.saturating_add(1);
                        if self.decoder_reset_budget_used < self.decoder_reset_budget_limit {
                            RecoveryAction::RequestDecoderReset
                        } else {
                            RecoveryAction::CooldownSuppressed
                        }
                    } else if persistent_wait_keyframe && allow_wait_keyframe_stage_escalation {
                        self.coalesced_decoder_reset_in_flight()
                    } else if hard_stuck_transport_await_recovery_keyframe
                        && allow_transport_await_stage_escalation
                        && (self.decoder_reset_budget_used >= self.decoder_reset_budget_limit
                            || self
                                .last_decoder_reset_at
                                .map_or(false, |last| last.elapsed() >= self.cooldown))
                    {
                        self.pending_keyframe_signals = 0;
                        self.pending_decoder_reset_signals = 0;
                        self.reconnect_candidate_signals =
                            self.reconnect_candidate_signals.saturating_add(1);
                        self.clear_keyframe_epoch();
                        self.resolve_reconnect_or_decoder_reset_fallback(
                            now,
                            allow_reconnect,
                            VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                        )
                    } else if persistent_transport_await_recovery_keyframe
                        && allow_transport_await_stage_escalation
                        && self
                            .last_decoder_reset_at
                            .map_or(true, |last| last.elapsed() >= self.cooldown)
                    {
                        self.pending_keyframe_signals = 0;
                        self.pending_decoder_reset_signals = 0;
                        self.reconnect_candidate_signals =
                            self.reconnect_candidate_signals.saturating_add(1);
                        if self.decoder_reset_budget_used < self.decoder_reset_budget_limit {
                            RecoveryAction::RequestDecoderReset
                        } else {
                            RecoveryAction::CooldownSuppressed
                        }
                    } else if persistent_transport_await_recovery_keyframe
                        && allow_transport_await_stage_escalation
                    {
                        self.coalesced_decoder_reset_in_flight()
                    } else if self.pending_keyframe_signals < self.keyframe_burst_threshold {
                        RecoveryAction::WaitForBurst
                    } else if self.try_release_keyframe_epoch_for_same_reason(reason_class, now) {
                        if matches!(
                            reason,
                            VideoEscalationReason::TransportAwaitRecoveryKeyframe
                        ) {
                            self.transport_await_recovery_started_at = Some(now);
                        }
                        self.last_keyframe_request_at = Some(now);
                        self.pending_keyframe_signals = 0;
                        self.reconnect_candidate_signals = 0;
                        if self.can_allocate_keyframe_attempt() {
                            RecoveryAction::RequestFir
                        } else {
                            RecoveryAction::CooldownSuppressed
                        }
                    } else if self
                        .last_keyframe_request_at
                        .map_or(true, |last| last.elapsed() >= self.keyframe_min_interval)
                        && self.try_enter_keyframe_epoch(reason_class, now)
                    {
                        self.last_keyframe_request_at = Some(now);
                        self.pending_keyframe_signals = 0;
                        self.reconnect_candidate_signals = 0;
                        if self.can_allocate_keyframe_attempt() {
                            RecoveryAction::RequestPli
                        } else {
                            RecoveryAction::CooldownSuppressed
                        }
                    } else {
                        self.coalesced_keyframe_in_flight()
                    }
                }
            }
            VideoEscalationReason::Reconfigure | VideoEscalationReason::DecoderBackendFailure => {
                self.wait_keyframe_started_at = None;
                self.transport_await_recovery_started_at = None;
                self.transport_deadline_window_started_at = None;
                self.transport_deadline_window_count = 0;
                let reason_class = match reason {
                    VideoEscalationReason::Reconfigure => DecoderResetReasonClass::Reconfigure,
                    VideoEscalationReason::DecoderBackendFailure => {
                        DecoderResetReasonClass::DecoderBackendFailure
                    }
                    _ => unreachable!(),
                };
                let immediate_decoder_reset_reason =
                    matches!(reason, VideoEscalationReason::DecoderBackendFailure);
                self.pending_keyframe_signals = 0;
                if self
                    .last_decoder_reset_signal_at
                    .map_or(true, |last| last.elapsed() > self.burst_window)
                    || self.last_decoder_reset_reason_class != Some(reason_class)
                {
                    self.pending_decoder_reset_signals = 0;
                }
                self.last_decoder_reset_signal_at = Some(now);
                self.last_decoder_reset_reason_class = Some(reason_class);
                self.pending_decoder_reset_signals = if immediate_decoder_reset_reason {
                    self.decoder_reset_burst_threshold
                } else {
                    self.pending_decoder_reset_signals.saturating_add(1)
                };
                if matches!(reason, VideoEscalationReason::Reconfigure)
                    && !allow_reconfigure_stage_escalation
                {
                    // 保守门禁：reconfigure 缺少失败证据时只留在观察/局部自愈，不升级昂贵 reset。
                    RecoveryAction::WaitForDecoderResetBurst
                } else if self.pending_decoder_reset_signals < self.decoder_reset_burst_threshold {
                    RecoveryAction::WaitForDecoderResetBurst
                } else if self
                    .last_decoder_reset_at
                    .map_or(true, |last| last.elapsed() >= self.cooldown)
                {
                    self.pending_decoder_reset_signals = 0;
                    self.reconnect_candidate_signals =
                        self.reconnect_candidate_signals.saturating_add(1);
                    if self.decoder_reset_budget_used < self.decoder_reset_budget_limit {
                        RecoveryAction::RequestDecoderReset
                    } else {
                        RecoveryAction::CooldownSuppressed
                    }
                } else {
                    self.coalesced_decoder_reset_in_flight()
                }
            }
            VideoEscalationReason::TransportSevereDeadline => {
                self.wait_keyframe_started_at = None;
                self.transport_await_recovery_started_at = None;
                let severe_signal_is_stale = self.last_severe_deadline_at.map_or(true, |last| {
                    last.elapsed() > self.severe_deadline_reconnect_window
                });
                if severe_signal_is_stale {
                    self.reconnect_candidate_signals = 0;
                }
                // 大洞 deadline 失效通常说明这一段视频已经不可救，
                // 这里直接跳过 keyframe burst，优先推到更高一级恢复。
                self.pending_keyframe_signals = 0;
                self.pending_decoder_reset_signals = 0;
                self.reconnect_candidate_signals =
                    self.reconnect_candidate_signals.saturating_add(1);
                self.last_severe_deadline_at = Some(now);
                if self.reconnect_candidate_signals >= CONNECTIVITY_DEADLINE_RECONNECT_HIT_THRESHOLD
                {
                    self.clear_keyframe_epoch();
                    self.resolve_reconnect_or_decoder_reset_fallback(
                        now,
                        allow_reconnect,
                        VideoEscalationReason::TransportSevereDeadline,
                    )
                } else {
                    RecoveryAction::CooldownSuppressed
                }
            }
        };
        self.register_action_applied(action);
        VideoEscalationDecision {
            observation_id: self.next_observation_id,
            action,
        }
    }

    fn resolve_reconnect_or_decoder_reset_fallback(
        &mut self,
        _now: Instant,
        allow_reconnect: bool,
        escalation_reason: VideoEscalationReason,
    ) -> RecoveryAction {
        if allow_reconnect {
            return if self.reconnect_budget_used < self.reconnect_budget_limit {
                RecoveryAction::RequestReconnectCandidate
            } else {
                RecoveryAction::CooldownSuppressed
            };
        }
        // 连接域升级在策略禁止 reconnect 时不得吸收为本地 decoder reset（传输坏窗 ≠ 解码器故障）。
        if matches!(
            escalation_reason,
            VideoEscalationReason::LifecycleRecovering
                | VideoEscalationReason::TransportExpiredDeadline
                | VideoEscalationReason::TransportSevereDeadline
        ) {
            return RecoveryAction::CooldownSuppressed;
        }
        if self
            .last_decoder_reset_at
            .map_or(true, |last| last.elapsed() >= self.cooldown)
        {
            if self.decoder_reset_budget_used < self.decoder_reset_budget_limit {
                RecoveryAction::RequestDecoderReset
            } else {
                RecoveryAction::CooldownSuppressed
            }
        } else {
            RecoveryAction::CooldownSuppressed
        }
    }

    pub fn suppressed(&mut self, action: RecoveryAction) -> VideoEscalationDecision {
        self.next_observation_id = self.next_observation_id.saturating_add(1);
        VideoEscalationDecision {
            observation_id: self.next_observation_id,
            action,
        }
    }

    fn try_enter_keyframe_epoch(
        &mut self,
        reason_class: KeyframeReasonClass,
        now: Instant,
    ) -> bool {
        if self.keyframe_epoch_active {
            let reason_changed = self.keyframe_epoch_reason_class != Some(reason_class);
            if reason_changed {
                self.clear_keyframe_epoch();
            }
        }
        if self.keyframe_epoch_active {
            return false;
        }
        self.keyframe_epoch_active = true;
        self.keyframe_epoch_started_at = Some(now);
        self.keyframe_epoch_reason_class = Some(reason_class);
        true
    }

    fn try_release_keyframe_epoch_for_same_reason(
        &mut self,
        reason_class: KeyframeReasonClass,
        now: Instant,
    ) -> bool {
        let can_auto_release = matches!(
            reason_class,
            KeyframeReasonClass::DisplaySupplyCritical
                | KeyframeReasonClass::AdapterIdleTimeout
                | KeyframeReasonClass::TransportAwaitRecoveryKeyframe
        );
        if !can_auto_release
            || !self.keyframe_epoch_active
            || self.keyframe_epoch_reason_class != Some(reason_class)
        {
            return false;
        }
        let should_release = self.keyframe_epoch_started_at.map_or(false, |started_at| {
            now.duration_since(started_at) >= self.escalation_window
        });
        if should_release {
            // 超过升级窗口后自动释放同 reason 的 keyframe epoch，解除长期抑制自锁。
            self.clear_keyframe_epoch();
            return true;
        }
        false
    }

    fn clear_keyframe_epoch(&mut self) {
        self.keyframe_epoch_active = false;
        self.keyframe_epoch_started_at = None;
        self.keyframe_epoch_reason_class = None;
    }

    fn can_allocate_keyframe_attempt(&self) -> bool {
        let provisional = self
            .keyframe_budget_used
            .saturating_add(u8::from(self.keyframe_budget_reservation_active));
        provisional < self.keyframe_budget_limit
    }

    fn coalesced_keyframe_in_flight(&self) -> RecoveryAction {
        RecoveryAction::CoalescedKeyframeInFlight
    }

    fn coalesced_decoder_reset_in_flight(&self) -> RecoveryAction {
        RecoveryAction::CoalescedDecoderResetInFlight
    }
}

fn reason_allows_connectivity_fallback(reason: VideoEscalationReason) -> bool {
    matches!(
        reason,
        VideoEscalationReason::LifecycleRecovering
            | VideoEscalationReason::TransportExpiredDeadline
            | VideoEscalationReason::TransportSevereDeadline
            | VideoEscalationReason::TransportRecoveredLate
            | VideoEscalationReason::TransportSampleLoss
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyframeTransportFeedback {
    None,
    UnsentPending,
    SentPending,
    Terminal,
}

#[cfg(test)]
#[path = "escalation.test.rs"]
mod tests;
