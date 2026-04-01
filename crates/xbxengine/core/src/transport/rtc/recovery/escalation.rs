use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryAction {
    WaitForBurst,
    WaitForDecoderResetBurst,
    CooldownSuppressed,
    StartupGraceSuppressed,
    RequestKeyframe,
    RequestDecoderReset,
    RequestReconnectCandidate,
    RequestKeyframeAndDecoderReset,
    StartupLowQualityRetry,
}

impl RecoveryAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::WaitForBurst => "waitForBurst",
            Self::WaitForDecoderResetBurst => "waitForDecoderResetBurst",
            Self::CooldownSuppressed => "cooldownSuppressed",
            Self::StartupGraceSuppressed => "startupGraceSuppressed",
            Self::RequestKeyframe => "requestKeyframe",
            Self::RequestDecoderReset => "requestDecoderReset",
            Self::RequestReconnectCandidate => "requestReconnectCandidate",
            Self::RequestKeyframeAndDecoderReset => "requestKeyframe+decoderReset",
            Self::StartupLowQualityRetry => "requestKeyframe+decoderReset(startupLowQualityRetry)",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KeyframeReasonClass {
    WaitKeyframe,
    TransportAwaitRecoveryKeyframe,
    AdapterIdleTimeout,
    AdapterThinStream,
    TransportExpiredDeadline,
    TransportRecoveredLate,
    TransportSampleLoss,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DecoderResetReasonClass {
    Reconfigure,
    DecoderBackendFailure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoEscalationReason {
    LifecycleRecovering,
    WaitKeyframe,
    TransportAwaitRecoveryKeyframe,
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
            Self::TransportAwaitRecoveryKeyframe => "transportAwaitRecoveryKeyframe",
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryActionOwner {
    Nack,
    Keyframe,
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
    pub budget_consumed_on_proposal: bool,
    pub advances_recovery_epoch_on_success: bool,
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
    recovery_epoch: u64,
    keyframe_budget_used: u8,
    decoder_reset_budget_used: u8,
    reconnect_budget_used: u8,
    keyframe_budget_limit: u8,
    decoder_reset_budget_limit: u8,
    reconnect_budget_limit: u8,
    next_observation_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoEscalationDecision {
    pub observation_id: u64,
    pub action: RecoveryAction,
}

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
        self.last_keyframe_signal_at = None;
        self.last_decoder_reset_signal_at = None;
        self.last_keyframe_reason_class = None;
        self.last_decoder_reset_reason_class = None;
        self.keyframe_budget_used = 0;
        self.decoder_reset_budget_used = 0;
        self.reconnect_budget_used = 0;
        self.clear_keyframe_epoch();
    }

    pub fn on_reason_with_epoch(
        &mut self,
        reason: VideoEscalationReason,
        recovery_epoch: u64,
    ) -> VideoEscalationDecision {
        self.begin_recovery_epoch(recovery_epoch);
        self.on_reason(reason)
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
            RecoveryAction::RequestKeyframe => RecoveryActionContract {
                owner: Some(RecoveryActionOwner::Keyframe),
                budget_kind: Some(RecoveryBudgetKind::Keyframe),
                budget_consumed_on_proposal: true,
                advances_recovery_epoch_on_success: false,
            },
            RecoveryAction::RequestDecoderReset => RecoveryActionContract {
                owner: Some(RecoveryActionOwner::DecoderReset),
                budget_kind: Some(RecoveryBudgetKind::DecoderReset),
                budget_consumed_on_proposal: true,
                advances_recovery_epoch_on_success: true,
            },
            RecoveryAction::RequestReconnectCandidate => RecoveryActionContract {
                owner: Some(RecoveryActionOwner::Reconnect),
                budget_kind: Some(RecoveryBudgetKind::Reconnect),
                budget_consumed_on_proposal: true,
                advances_recovery_epoch_on_success: true,
            },
            RecoveryAction::RequestKeyframeAndDecoderReset => RecoveryActionContract {
                owner: Some(RecoveryActionOwner::DecoderReset),
                budget_kind: Some(RecoveryBudgetKind::DecoderReset),
                budget_consumed_on_proposal: true,
                advances_recovery_epoch_on_success: true,
            },
            RecoveryAction::WaitForBurst
            | RecoveryAction::WaitForDecoderResetBurst
            | RecoveryAction::CooldownSuppressed
            | RecoveryAction::StartupGraceSuppressed
            | RecoveryAction::StartupLowQualityRetry => RecoveryActionContract {
                owner: None,
                budget_kind: None,
                budget_consumed_on_proposal: false,
                advances_recovery_epoch_on_success: false,
            },
        }
    }

    pub fn register_action_applied(&mut self, action: RecoveryAction) {
        match action {
            RecoveryAction::RequestKeyframe => {
                self.keyframe_budget_used = self.keyframe_budget_used.saturating_add(1);
            }
            RecoveryAction::RequestDecoderReset => {
                self.decoder_reset_budget_used = self.decoder_reset_budget_used.saturating_add(1);
                self.clear_keyframe_epoch();
            }
            RecoveryAction::RequestReconnectCandidate => {
                self.reconnect_budget_used = self.reconnect_budget_used.saturating_add(1);
                self.clear_keyframe_epoch();
            }
            RecoveryAction::RequestKeyframeAndDecoderReset => {
                self.keyframe_budget_used = self.keyframe_budget_used.saturating_add(1);
                self.decoder_reset_budget_used = self.decoder_reset_budget_used.saturating_add(1);
                self.clear_keyframe_epoch();
            }
            RecoveryAction::WaitForBurst
            | RecoveryAction::WaitForDecoderResetBurst
            | RecoveryAction::CooldownSuppressed
            | RecoveryAction::StartupGraceSuppressed
            | RecoveryAction::StartupLowQualityRetry => {}
        }
    }

    pub fn on_reason(&mut self, reason: VideoEscalationReason) -> VideoEscalationDecision {
        self.next_observation_id = self.next_observation_id.saturating_add(1);
        let now = Instant::now();
        let action = match reason {
            VideoEscalationReason::LifecycleRecovering => {
                self.wait_keyframe_started_at = None;
                self.transport_await_recovery_started_at = None;
                self.pending_keyframe_signals = 0;
                self.pending_decoder_reset_signals = 0;
                self.reconnect_candidate_signals = 0;
                self.clear_keyframe_epoch();
                if self.reconnect_budget_used < self.reconnect_budget_limit {
                    RecoveryAction::RequestReconnectCandidate
                } else {
                    RecoveryAction::CooldownSuppressed
                }
            }
            VideoEscalationReason::WaitKeyframe
            | VideoEscalationReason::TransportAwaitRecoveryKeyframe
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
                    if self.reconnect_budget_used < self.reconnect_budget_limit {
                        RecoveryAction::RequestReconnectCandidate
                    } else {
                        RecoveryAction::CooldownSuppressed
                    }
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
                    let repeated_thin_stream_after_keyframe =
                        matches!(reason, VideoEscalationReason::AdapterThinStream)
                            && self.last_keyframe_request_at.map_or(false, |last| {
                                let elapsed = last.elapsed();
                                elapsed >= self.keyframe_upgrade_min_delay
                                    && elapsed <= self.escalation_window
                            });
                    let repeated_idle_timeout_after_keyframe =
                        matches!(reason, VideoEscalationReason::AdapterIdleTimeout)
                            && self.last_keyframe_request_at.map_or(false, |last| {
                                let elapsed = last.elapsed();
                                elapsed >= self.keyframe_upgrade_min_delay
                                    && elapsed <= self.escalation_window
                            });
                    let repeated_sample_loss_after_keyframe =
                        matches!(reason, VideoEscalationReason::TransportSampleLoss)
                            && self.last_keyframe_request_at.map_or(false, |last| {
                                let elapsed = last.elapsed();
                                elapsed >= self.keyframe_upgrade_min_delay
                                    && elapsed <= self.escalation_window
                            });
                    let persistent_wait_keyframe =
                        matches!(reason, VideoEscalationReason::WaitKeyframe)
                            && self.wait_keyframe_started_at.map_or(false, |started_at| {
                                now.duration_since(started_at) >= self.cooldown.mul_f32(2.0)
                            });
                    let persistent_transport_await_recovery_keyframe = matches!(
                        reason,
                        VideoEscalationReason::TransportAwaitRecoveryKeyframe
                    ) && self
                        .transport_await_recovery_started_at
                        .map_or(false, |started_at| {
                            now.duration_since(started_at) >= self.escalation_window
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
                        if self.transport_deadline_window_count >= 3 {
                            self.pending_keyframe_signals = 0;
                            self.pending_decoder_reset_signals = 0;
                            self.reconnect_candidate_signals =
                                self.reconnect_candidate_signals.saturating_add(1);
                            self.clear_keyframe_epoch();
                            if self.reconnect_budget_used < self.reconnect_budget_limit {
                                RecoveryAction::RequestReconnectCandidate
                            } else {
                                RecoveryAction::CooldownSuppressed
                            }
                        } else if self
                            .last_keyframe_request_at
                            .map_or(true, |last| last.elapsed() >= self.cooldown)
                            && self.try_enter_keyframe_epoch(reason_class, now)
                        {
                            self.last_keyframe_request_at = Some(now);
                            self.pending_keyframe_signals = 0;
                            self.reconnect_candidate_signals = 0;
                            if self.keyframe_budget_used < self.keyframe_budget_limit {
                                RecoveryAction::RequestKeyframe
                            } else {
                                RecoveryAction::CooldownSuppressed
                            }
                        } else {
                            RecoveryAction::CooldownSuppressed
                        }
                    } else if repeated_thin_stream_after_keyframe
                        && self
                            .last_decoder_reset_at
                            .map_or(true, |last| last.elapsed() >= self.cooldown)
                    {
                        self.pending_keyframe_signals = 0;
                        self.pending_decoder_reset_signals = 0;
                        self.reconnect_candidate_signals =
                            self.reconnect_candidate_signals.saturating_add(1);
                        self.last_decoder_reset_at = Some(now);
                        self.last_keyframe_request_at = Some(now);
                        self.clear_keyframe_epoch();
                        if self.decoder_reset_budget_used < self.decoder_reset_budget_limit {
                            RecoveryAction::RequestDecoderReset
                        } else {
                            RecoveryAction::CooldownSuppressed
                        }
                    } else if repeated_idle_timeout_after_keyframe
                        && self
                            .last_decoder_reset_at
                            .map_or(true, |last| last.elapsed() >= self.cooldown)
                    {
                        self.pending_keyframe_signals = 0;
                        self.pending_decoder_reset_signals = 0;
                        self.reconnect_candidate_signals =
                            self.reconnect_candidate_signals.saturating_add(1);
                        self.last_decoder_reset_at = Some(now);
                        self.last_keyframe_request_at = Some(now);
                        self.clear_keyframe_epoch();
                        if self.decoder_reset_budget_used < self.decoder_reset_budget_limit {
                            RecoveryAction::RequestDecoderReset
                        } else {
                            RecoveryAction::CooldownSuppressed
                        }
                    } else if repeated_sample_loss_after_keyframe
                        && self
                            .last_decoder_reset_at
                            .map_or(true, |last| last.elapsed() >= self.cooldown.mul_f32(0.5))
                    {
                        self.pending_keyframe_signals = 0;
                        self.pending_decoder_reset_signals = 0;
                        self.reconnect_candidate_signals =
                            self.reconnect_candidate_signals.saturating_add(1);
                        self.last_decoder_reset_at = Some(now);
                        self.last_keyframe_request_at = Some(now);
                        self.clear_keyframe_epoch();
                        if self.decoder_reset_budget_used < self.decoder_reset_budget_limit {
                            RecoveryAction::RequestDecoderReset
                        } else {
                            RecoveryAction::CooldownSuppressed
                        }
                    } else if persistent_wait_keyframe
                        && self
                            .last_decoder_reset_at
                            .map_or(true, |last| last.elapsed() >= self.cooldown)
                    {
                        self.pending_keyframe_signals = 0;
                        self.pending_decoder_reset_signals = 0;
                        self.reconnect_candidate_signals =
                            self.reconnect_candidate_signals.saturating_add(1);
                        self.last_decoder_reset_at = Some(now);
                        self.last_keyframe_request_at = Some(now);
                        self.clear_keyframe_epoch();
                        if self.decoder_reset_budget_used < self.decoder_reset_budget_limit {
                            RecoveryAction::RequestDecoderReset
                        } else {
                            RecoveryAction::CooldownSuppressed
                        }
                    } else if hard_stuck_transport_await_recovery_keyframe
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
                        if self.reconnect_budget_used < self.reconnect_budget_limit {
                            RecoveryAction::RequestReconnectCandidate
                        } else {
                            RecoveryAction::CooldownSuppressed
                        }
                    } else if persistent_transport_await_recovery_keyframe
                        && self
                            .last_decoder_reset_at
                            .map_or(true, |last| last.elapsed() >= self.cooldown)
                    {
                        self.pending_keyframe_signals = 0;
                        self.pending_decoder_reset_signals = 0;
                        self.reconnect_candidate_signals =
                            self.reconnect_candidate_signals.saturating_add(1);
                        self.last_decoder_reset_at = Some(now);
                        self.last_keyframe_request_at = Some(now);
                        self.clear_keyframe_epoch();
                        if self.decoder_reset_budget_used < self.decoder_reset_budget_limit {
                            RecoveryAction::RequestDecoderReset
                        } else {
                            RecoveryAction::CooldownSuppressed
                        }
                    } else if self.pending_keyframe_signals < self.keyframe_burst_threshold {
                        RecoveryAction::WaitForBurst
                    } else if self
                        .last_keyframe_request_at
                        .map_or(true, |last| last.elapsed() >= self.keyframe_min_interval)
                        && self.try_enter_keyframe_epoch(reason_class, now)
                    {
                        self.last_keyframe_request_at = Some(now);
                        self.pending_keyframe_signals = 0;
                        self.reconnect_candidate_signals = 0;
                        if self.keyframe_budget_used < self.keyframe_budget_limit {
                            RecoveryAction::RequestKeyframe
                        } else {
                            RecoveryAction::CooldownSuppressed
                        }
                    } else {
                        RecoveryAction::CooldownSuppressed
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
                if self.pending_decoder_reset_signals < self.decoder_reset_burst_threshold {
                    RecoveryAction::WaitForDecoderResetBurst
                } else if self
                    .last_decoder_reset_at
                    .map_or(true, |last| last.elapsed() >= self.cooldown)
                {
                    self.pending_decoder_reset_signals = 0;
                    self.reconnect_candidate_signals =
                        self.reconnect_candidate_signals.saturating_add(1);
                    self.last_decoder_reset_at = Some(now);
                    self.last_keyframe_request_at = Some(now);
                    self.clear_keyframe_epoch();
                    if self.decoder_reset_budget_used < self.decoder_reset_budget_limit {
                        RecoveryAction::RequestDecoderReset
                    } else {
                        RecoveryAction::CooldownSuppressed
                    }
                } else {
                    RecoveryAction::CooldownSuppressed
                }
            }
            VideoEscalationReason::TransportSevereDeadline => {
                self.wait_keyframe_started_at = None;
                self.transport_await_recovery_started_at = None;
                // 大洞 deadline 失效通常说明这一段视频已经不可救，
                // 这里直接跳过 keyframe burst，优先推到更高一级恢复。
                self.pending_keyframe_signals = 0;
                self.pending_decoder_reset_signals = 0;
                self.reconnect_candidate_signals =
                    self.reconnect_candidate_signals.saturating_add(1);
                self.last_severe_deadline_at = Some(now);
                if self.reconnect_candidate_signals >= 2 {
                    self.clear_keyframe_epoch();
                    if self.reconnect_budget_used < self.reconnect_budget_limit {
                        RecoveryAction::RequestReconnectCandidate
                    } else {
                        RecoveryAction::CooldownSuppressed
                    }
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

    fn clear_keyframe_epoch(&mut self) {
        self.keyframe_epoch_active = false;
        self.keyframe_epoch_started_at = None;
        self.keyframe_epoch_reason_class = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RecoveryAction, RecoveryBudgetKind, VideoEscalationConfig, VideoEscalationController,
        VideoEscalationReason,
    };
    use std::time::Duration;

    #[test]
    fn waits_for_burst_before_requesting_keyframe() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 250,
            keyframe_burst_threshold: 2,
            decoder_reset_burst_threshold: 2,
            ..VideoEscalationConfig::default()
        });

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::WaitKeyframe)
                .action,
            RecoveryAction::WaitForBurst
        );
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::AdapterIdleTimeout)
                .action,
            RecoveryAction::RequestKeyframe
        );
    }

    #[test]
    fn idle_timeout_requests_keyframe_immediately() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 250,
            keyframe_burst_threshold: 2,
            decoder_reset_burst_threshold: 2,
            ..VideoEscalationConfig::default()
        });

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::AdapterIdleTimeout)
                .action,
            RecoveryAction::RequestKeyframe
        );
    }

    #[test]
    fn reconfigure_burst_expires_before_requesting_decoder_reset() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 250,
            keyframe_burst_threshold: 2,
            decoder_reset_burst_threshold: 2,
            ..VideoEscalationConfig::default()
        });

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::Reconfigure)
                .action,
            RecoveryAction::WaitForDecoderResetBurst
        );
        std::thread::sleep(Duration::from_millis(380));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::Reconfigure)
                .action,
            RecoveryAction::WaitForDecoderResetBurst
        );
    }

    #[test]
    fn decoder_backend_failure_requests_decoder_reset_immediately() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 250,
            keyframe_burst_threshold: 2,
            decoder_reset_burst_threshold: 2,
            ..VideoEscalationConfig::default()
        });

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::DecoderBackendFailure)
                .action,
            RecoveryAction::RequestDecoderReset
        );
    }

    #[test]
    fn repeated_transport_deadline_failures_are_throttled_within_epoch() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 40,
            keyframe_burst_threshold: 2,
            decoder_reset_burst_threshold: 1,
            keyframe_min_interval_ms: 40,
            escalation_window_ms: 180,
            keyframe_upgrade_min_delay_ms: 10,
        });

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportExpiredDeadline)
                .action,
            RecoveryAction::RequestKeyframe
        );
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportExpiredDeadline)
                .action,
            RecoveryAction::CooldownSuppressed
        );
        // keyframe_min_interval 在控制器内部有最小下限，需跨过窗口后才能再次发 keyframe。
        std::thread::sleep(Duration::from_millis(130));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportExpiredDeadline)
                .action,
            RecoveryAction::CooldownSuppressed
        );
        std::thread::sleep(Duration::from_millis(130));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportExpiredDeadline)
                .action,
            RecoveryAction::CooldownSuppressed
        );
    }

    #[test]
    fn transport_deadline_storm_within_same_window_does_not_reconnect() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 60,
            keyframe_burst_threshold: 2,
            decoder_reset_burst_threshold: 1,
            keyframe_min_interval_ms: 60,
            escalation_window_ms: 220,
            keyframe_upgrade_min_delay_ms: 10,
        });

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportExpiredDeadline)
                .action,
            RecoveryAction::RequestKeyframe
        );
        for _ in 0..4 {
            assert_eq!(
                controller
                    .on_reason(VideoEscalationReason::TransportExpiredDeadline)
                    .action,
                RecoveryAction::CooldownSuppressed
            );
        }
    }

    #[test]
    fn severe_transport_deadline_requires_repeat_before_reconnect() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 60_000,
            keyframe_burst_threshold: 2,
            decoder_reset_burst_threshold: 2,
            keyframe_min_interval_ms: 60_000,
            escalation_window_ms: 120_000,
            keyframe_upgrade_min_delay_ms: 500,
        });

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportSevereDeadline)
                .action,
            RecoveryAction::CooldownSuppressed
        );
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportSevereDeadline)
                .action,
            RecoveryAction::RequestReconnectCandidate
        );
    }

    #[test]
    fn adapter_idle_after_severe_deadline_escalates_to_reconnect_candidate() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 200,
            keyframe_burst_threshold: 2,
            decoder_reset_burst_threshold: 2,
            ..VideoEscalationConfig::default()
        });

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportSevereDeadline)
                .action,
            RecoveryAction::CooldownSuppressed
        );
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::AdapterIdleTimeout)
                .action,
            RecoveryAction::RequestReconnectCandidate
        );
    }

    #[test]
    fn persistent_wait_keyframe_escalates_to_decoder_reset() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 200,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 2,
            keyframe_upgrade_min_delay_ms: 150,
            ..VideoEscalationConfig::default()
        });

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::WaitKeyframe)
                .action,
            RecoveryAction::RequestKeyframe
        );
        std::thread::sleep(Duration::from_millis(210));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::WaitKeyframe)
                .action,
            RecoveryAction::CooldownSuppressed
        );
        std::thread::sleep(Duration::from_millis(210));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::WaitKeyframe)
                .action,
            RecoveryAction::RequestDecoderReset
        );
    }

    #[test]
    fn transport_sample_loss_requests_keyframe_immediately() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 250,
            keyframe_burst_threshold: 3,
            decoder_reset_burst_threshold: 2,
            ..VideoEscalationConfig::default()
        });

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportSampleLoss)
                .action,
            RecoveryAction::RequestKeyframe
        );
    }

    #[test]
    fn repeated_transport_sample_loss_after_keyframe_escalates_to_decoder_reset() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 200,
            keyframe_burst_threshold: 3,
            decoder_reset_burst_threshold: 2,
            keyframe_upgrade_min_delay_ms: 0,
            ..VideoEscalationConfig::default()
        });

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportSampleLoss)
                .action,
            RecoveryAction::RequestKeyframe
        );
        std::thread::sleep(Duration::from_millis(130));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportSampleLoss)
                .action,
            RecoveryAction::RequestDecoderReset
        );
    }

    #[test]
    fn thin_stream_requests_keyframe_then_decoder_reset_quickly() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 250,
            keyframe_burst_threshold: 3,
            decoder_reset_burst_threshold: 2,
            keyframe_upgrade_min_delay_ms: 0,
            ..VideoEscalationConfig::default()
        });

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::AdapterThinStream)
                .action,
            RecoveryAction::RequestKeyframe
        );
        std::thread::sleep(Duration::from_millis(130));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::AdapterThinStream)
                .action,
            RecoveryAction::RequestDecoderReset
        );
    }

    #[test]
    fn await_recovery_keyframe_is_throttled_within_same_epoch() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 250,
            keyframe_burst_threshold: 3,
            decoder_reset_burst_threshold: 2,
            keyframe_upgrade_min_delay_ms: 0,
            ..VideoEscalationConfig::default()
        });

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
                .action,
            RecoveryAction::RequestKeyframe
        );
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
                .action,
            RecoveryAction::CooldownSuppressed
        );
        std::thread::sleep(Duration::from_millis(130));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
                .action,
            RecoveryAction::CooldownSuppressed
        );
        std::thread::sleep(Duration::from_millis(130));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
                .action,
            RecoveryAction::CooldownSuppressed
        );
    }

    #[test]
    fn persistent_await_recovery_keyframe_escalates_to_decoder_reset_then_reconnect() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 120,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 1,
            keyframe_min_interval_ms: 120,
            escalation_window_ms: 220,
            keyframe_upgrade_min_delay_ms: 0,
        });

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
                .action,
            RecoveryAction::RequestKeyframe
        );
        std::thread::sleep(Duration::from_millis(240));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
                .action,
            RecoveryAction::RequestDecoderReset
        );
        std::thread::sleep(Duration::from_millis(480));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
                .action,
            RecoveryAction::RequestReconnectCandidate
        );
    }

    #[test]
    fn keyframe_epoch_resets_on_reason_change() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 180,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 3,
            keyframe_min_interval_ms: 180,
            escalation_window_ms: 700,
            keyframe_upgrade_min_delay_ms: 160,
        });

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
                .action,
            RecoveryAction::RequestKeyframe
        );
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
                .action,
            RecoveryAction::CooldownSuppressed
        );
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::WaitKeyframe)
                .action,
            RecoveryAction::RequestKeyframe
        );
    }

    #[test]
    fn keyframe_epoch_can_be_reset_explicitly_after_recovery() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 200,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 2,
            keyframe_min_interval_ms: 200,
            escalation_window_ms: 900,
            keyframe_upgrade_min_delay_ms: 200,
        });

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
                .action,
            RecoveryAction::RequestKeyframe
        );
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
                .action,
            RecoveryAction::CooldownSuppressed
        );
        controller.reset_keyframe_epoch();
        std::thread::sleep(Duration::from_millis(220));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
                .action,
            RecoveryAction::RequestKeyframe
        );
    }

    #[test]
    fn idle_timeout_requests_keyframe_then_decoder_reset_quickly() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 250,
            keyframe_burst_threshold: 3,
            decoder_reset_burst_threshold: 2,
            keyframe_upgrade_min_delay_ms: 0,
            ..VideoEscalationConfig::default()
        });

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::AdapterIdleTimeout)
                .action,
            RecoveryAction::RequestKeyframe
        );
        std::thread::sleep(Duration::from_millis(130));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::AdapterIdleTimeout)
                .action,
            RecoveryAction::RequestDecoderReset
        );
    }

    #[test]
    fn cooldown_window_prevents_keyframe_storm() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 220,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 3,
            keyframe_min_interval_ms: 220,
            escalation_window_ms: 800,
            keyframe_upgrade_min_delay_ms: 220,
        });

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportExpiredDeadline)
                .action,
            RecoveryAction::RequestKeyframe
        );
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportExpiredDeadline)
                .action,
            RecoveryAction::CooldownSuppressed
        );
        std::thread::sleep(Duration::from_millis(230));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportExpiredDeadline)
                .action,
            RecoveryAction::CooldownSuppressed
        );
    }

    #[test]
    fn repeated_reason_outside_keyframe_interval_can_upgrade_to_decoder_reset() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 180,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 2,
            keyframe_min_interval_ms: 180,
            escalation_window_ms: 700,
            keyframe_upgrade_min_delay_ms: 120,
        });

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::AdapterThinStream)
                .action,
            RecoveryAction::RequestKeyframe
        );
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::AdapterThinStream)
                .action,
            RecoveryAction::RequestDecoderReset
        );
    }

    #[test]
    fn reconnect_budget_is_single_shot_per_recovery_epoch() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 200,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 1,
            keyframe_min_interval_ms: 200,
            escalation_window_ms: 600,
            keyframe_upgrade_min_delay_ms: 0,
        });
        controller.begin_recovery_epoch(10);
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportSevereDeadline)
                .action,
            RecoveryAction::CooldownSuppressed
        );
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportSevereDeadline)
                .action,
            RecoveryAction::RequestReconnectCandidate
        );
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportSevereDeadline)
                .action,
            RecoveryAction::CooldownSuppressed
        );
        controller.begin_recovery_epoch(11);
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportSevereDeadline)
                .action,
            RecoveryAction::CooldownSuppressed
        );
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportSevereDeadline)
                .action,
            RecoveryAction::RequestReconnectCandidate
        );
    }

    #[test]
    fn keyframe_budget_resets_after_new_recovery_epoch() {
        let mut controller = VideoEscalationController::new(VideoEscalationConfig {
            cooldown_ms: 120,
            keyframe_burst_threshold: 1,
            decoder_reset_burst_threshold: 2,
            keyframe_min_interval_ms: 120,
            escalation_window_ms: 400,
            keyframe_upgrade_min_delay_ms: 100,
        });
        controller.begin_recovery_epoch(3);
        controller.register_action_applied(RecoveryAction::RequestKeyframe);
        controller.register_action_applied(RecoveryAction::RequestKeyframe);
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
                .action,
            RecoveryAction::CooldownSuppressed
        );
        controller.begin_recovery_epoch(4);
        std::thread::sleep(Duration::from_millis(130));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
                .action,
            RecoveryAction::RequestKeyframe
        );
    }

    #[test]
    fn action_contract_defines_owner_budget_and_epoch_rules() {
        let keyframe = VideoEscalationController::action_contract(RecoveryAction::RequestKeyframe);
        assert!(keyframe.budget_consumed_on_proposal);
        assert_eq!(keyframe.budget_kind, Some(RecoveryBudgetKind::Keyframe));
        assert!(!keyframe.advances_recovery_epoch_on_success);

        let reset = VideoEscalationController::action_contract(RecoveryAction::RequestDecoderReset);
        assert!(reset.budget_consumed_on_proposal);
        assert_eq!(reset.budget_kind, Some(RecoveryBudgetKind::DecoderReset));
        assert!(reset.advances_recovery_epoch_on_success);

        let reconnect =
            VideoEscalationController::action_contract(RecoveryAction::RequestReconnectCandidate);
        assert!(reconnect.budget_consumed_on_proposal);
        assert_eq!(reconnect.budget_kind, Some(RecoveryBudgetKind::Reconnect));
        assert!(reconnect.advances_recovery_epoch_on_success);

        let suppressed =
            VideoEscalationController::action_contract(RecoveryAction::CooldownSuppressed);
        assert!(!suppressed.budget_consumed_on_proposal);
        assert!(suppressed.budget_kind.is_none());
        assert!(!suppressed.advances_recovery_epoch_on_success);
    }
}
