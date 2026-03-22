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

pub enum VideoEscalationReason {
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

pub struct VideoEscalationController {
    cooldown: Duration,
    burst_window: Duration,
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
    transport_deadline_window_started_at: Option<Instant>,
    transport_deadline_window_count: u8,
    next_observation_id: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoEscalationDecision {
    pub observation_id: u64,
    pub action: RecoveryAction,
}

impl VideoEscalationController {
    pub fn new(
        cooldown: Duration,
        keyframe_burst_threshold: u8,
        decoder_reset_burst_threshold: u8,
    ) -> Self {
        Self {
            cooldown,
            burst_window: cooldown.clamp(Duration::from_millis(200), Duration::from_millis(350)),
            keyframe_burst_threshold: keyframe_burst_threshold.max(1),
            decoder_reset_burst_threshold: decoder_reset_burst_threshold.max(1),
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
            transport_deadline_window_started_at: None,
            transport_deadline_window_count: 0,
            next_observation_id: 0,
        }
    }

    pub fn on_reason(&mut self, reason: VideoEscalationReason) -> VideoEscalationDecision {
        self.next_observation_id = self.next_observation_id.saturating_add(1);
        let now = Instant::now();
        let action = match reason {
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
                    RecoveryAction::RequestReconnectCandidate
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
                            && self
                                .last_keyframe_request_at
                                .map_or(false, |last| last.elapsed() <= self.cooldown);
                    let repeated_idle_timeout_after_keyframe =
                        matches!(reason, VideoEscalationReason::AdapterIdleTimeout)
                            && self
                                .last_keyframe_request_at
                                .map_or(false, |last| last.elapsed() <= self.cooldown);
                    let repeated_wait_recovery_after_keyframe = matches!(
                        reason,
                        VideoEscalationReason::TransportAwaitRecoveryKeyframe
                    ) && self
                        .last_keyframe_request_at
                        .map_or(false, |last| last.elapsed() <= self.cooldown);
                    let repeated_sample_loss_after_keyframe =
                        matches!(reason, VideoEscalationReason::TransportSampleLoss)
                            && self
                                .last_keyframe_request_at
                                .map_or(false, |last| last.elapsed() <= self.cooldown.mul_f32(0.5));
                    let persistent_wait_keyframe =
                        matches!(reason, VideoEscalationReason::WaitKeyframe)
                            && self.wait_keyframe_started_at.map_or(false, |started_at| {
                                now.duration_since(started_at) >= self.cooldown.mul_f32(2.0)
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
                            RecoveryAction::RequestReconnectCandidate
                        } else if self
                            .last_keyframe_request_at
                            .map_or(true, |last| last.elapsed() >= self.cooldown)
                        {
                            self.last_keyframe_request_at = Some(now);
                            self.pending_keyframe_signals = 0;
                            self.reconnect_candidate_signals = 0;
                            RecoveryAction::RequestKeyframe
                        } else {
                            RecoveryAction::CooldownSuppressed
                        }
                    } else if repeated_wait_recovery_after_keyframe
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
                        RecoveryAction::RequestDecoderReset
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
                        RecoveryAction::RequestDecoderReset
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
                        RecoveryAction::RequestDecoderReset
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
                        RecoveryAction::RequestDecoderReset
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
                        RecoveryAction::RequestDecoderReset
                    } else if self.pending_keyframe_signals < self.keyframe_burst_threshold {
                        RecoveryAction::WaitForBurst
                    } else if self
                        .last_keyframe_request_at
                        .map_or(true, |last| last.elapsed() >= self.cooldown)
                    {
                        self.last_keyframe_request_at = Some(now);
                        self.pending_keyframe_signals = 0;
                        self.reconnect_candidate_signals = 0;
                        RecoveryAction::RequestKeyframe
                    } else {
                        RecoveryAction::CooldownSuppressed
                    }
                }
            }
            VideoEscalationReason::Reconfigure | VideoEscalationReason::DecoderBackendFailure => {
                self.wait_keyframe_started_at = None;
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
                    RecoveryAction::RequestDecoderReset
                } else {
                    RecoveryAction::CooldownSuppressed
                }
            }
            VideoEscalationReason::TransportSevereDeadline => {
                self.wait_keyframe_started_at = None;
                // 大洞 deadline 失效通常说明这一段视频已经不可救，
                // 这里直接跳过 keyframe burst，优先推到更高一级恢复。
                self.pending_keyframe_signals = 0;
                self.pending_decoder_reset_signals = 0;
                self.reconnect_candidate_signals =
                    self.reconnect_candidate_signals.saturating_add(1);
                self.last_severe_deadline_at = Some(now);
                if self.reconnect_candidate_signals >= 2 {
                    RecoveryAction::RequestReconnectCandidate
                } else {
                    RecoveryAction::CooldownSuppressed
                }
            }
        };
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
}

#[cfg(test)]
mod tests {
    use super::{RecoveryAction, VideoEscalationController, VideoEscalationReason};
    use std::time::Duration;

    #[test]
    fn waits_for_burst_before_requesting_keyframe() {
        let mut controller = VideoEscalationController::new(Duration::from_millis(250), 2, 2);

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
        let mut controller = VideoEscalationController::new(Duration::from_millis(250), 2, 2);

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::AdapterIdleTimeout)
                .action,
            RecoveryAction::RequestKeyframe
        );
    }

    #[test]
    fn reconfigure_burst_expires_before_requesting_decoder_reset() {
        let mut controller = VideoEscalationController::new(Duration::from_millis(250), 2, 2);

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
        let mut controller = VideoEscalationController::new(Duration::from_millis(250), 2, 2);

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::DecoderBackendFailure)
                .action,
            RecoveryAction::RequestDecoderReset
        );
    }

    #[test]
    fn repeated_transport_deadline_failures_escalate_to_reconnect_candidate() {
        let mut controller = VideoEscalationController::new(Duration::from_millis(40), 2, 1);

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
        std::thread::sleep(Duration::from_millis(45));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportExpiredDeadline)
                .action,
            RecoveryAction::RequestKeyframe
        );
        std::thread::sleep(Duration::from_millis(45));
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportExpiredDeadline)
                .action,
            RecoveryAction::RequestReconnectCandidate
        );
    }

    #[test]
    fn transport_deadline_storm_within_same_window_does_not_reconnect() {
        let mut controller = VideoEscalationController::new(Duration::from_millis(60), 2, 1);

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
        let mut controller = VideoEscalationController::new(Duration::from_secs(60), 2, 2);

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
        let mut controller = VideoEscalationController::new(Duration::from_millis(200), 2, 2);

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
        let mut controller = VideoEscalationController::new(Duration::from_millis(200), 1, 2);

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
            RecoveryAction::RequestKeyframe
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
        let mut controller = VideoEscalationController::new(Duration::from_millis(250), 3, 2);

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportSampleLoss)
                .action,
            RecoveryAction::RequestKeyframe
        );
    }

    #[test]
    fn repeated_transport_sample_loss_after_keyframe_escalates_to_decoder_reset() {
        let mut controller = VideoEscalationController::new(Duration::from_millis(200), 3, 2);

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportSampleLoss)
                .action,
            RecoveryAction::RequestKeyframe
        );
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportSampleLoss)
                .action,
            RecoveryAction::RequestDecoderReset
        );
    }

    #[test]
    fn thin_stream_requests_keyframe_then_decoder_reset_quickly() {
        let mut controller = VideoEscalationController::new(Duration::from_millis(250), 3, 2);

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::AdapterThinStream)
                .action,
            RecoveryAction::RequestKeyframe
        );
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::AdapterThinStream)
                .action,
            RecoveryAction::RequestDecoderReset
        );
    }

    #[test]
    fn await_recovery_keyframe_requests_keyframe_then_decoder_reset_quickly() {
        let mut controller = VideoEscalationController::new(Duration::from_millis(250), 3, 2);

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
            RecoveryAction::RequestDecoderReset
        );
    }

    #[test]
    fn idle_timeout_requests_keyframe_then_decoder_reset_quickly() {
        let mut controller = VideoEscalationController::new(Duration::from_millis(250), 3, 2);

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::AdapterIdleTimeout)
                .action,
            RecoveryAction::RequestKeyframe
        );
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::AdapterIdleTimeout)
                .action,
            RecoveryAction::RequestDecoderReset
        );
    }
}
