use std::time::{Duration, Instant};

pub enum VideoEscalationReason {
    WaitKeyframe,
    Reconfigure,
    AdapterIdleTimeout,
    TransportExpiredDeadline,
    TransportSevereDeadline,
    TransportRecoveredLate,
}

pub struct VideoEscalationController {
    cooldown: Duration,
    keyframe_burst_threshold: u8,
    decoder_reset_burst_threshold: u8,
    severe_deadline_reconnect_window: Duration,
    pending_keyframe_signals: u8,
    pending_decoder_reset_signals: u8,
    reconnect_candidate_signals: u8,
    last_keyframe_request_at: Option<Instant>,
    last_decoder_reset_at: Option<Instant>,
    last_severe_deadline_at: Option<Instant>,
    next_observation_id: u64,
}

pub struct VideoEscalationDecision {
    pub observation_id: u64,
    pub action: &'static str,
}

impl VideoEscalationController {
    pub fn new(
        cooldown: Duration,
        keyframe_burst_threshold: u8,
        decoder_reset_burst_threshold: u8,
    ) -> Self {
        Self {
            cooldown,
            keyframe_burst_threshold: keyframe_burst_threshold.max(1),
            decoder_reset_burst_threshold: decoder_reset_burst_threshold.max(1),
            severe_deadline_reconnect_window: cooldown.mul_f32(3.0),
            pending_keyframe_signals: 0,
            pending_decoder_reset_signals: 0,
            reconnect_candidate_signals: 0,
            last_keyframe_request_at: None,
            last_decoder_reset_at: None,
            last_severe_deadline_at: None,
            next_observation_id: 0,
        }
    }

    pub fn on_reason(&mut self, reason: VideoEscalationReason) -> VideoEscalationDecision {
        self.next_observation_id = self.next_observation_id.saturating_add(1);
        let now = Instant::now();
        let action = match reason {
            VideoEscalationReason::WaitKeyframe
            | VideoEscalationReason::AdapterIdleTimeout
            | VideoEscalationReason::TransportRecoveredLate => {
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
                    "requestReconnectCandidate"
                } else {
                    self.pending_keyframe_signals = self.pending_keyframe_signals.saturating_add(1);
                    self.pending_decoder_reset_signals = 0;
                    if self.pending_keyframe_signals < self.keyframe_burst_threshold {
                        "waitForBurst"
                    } else if self
                        .last_keyframe_request_at
                        .map_or(true, |last| last.elapsed() >= self.cooldown)
                    {
                        self.last_keyframe_request_at = Some(now);
                        self.pending_keyframe_signals = 0;
                        self.reconnect_candidate_signals = 0;
                        "requestKeyframe"
                    } else {
                        "cooldownSuppressed"
                    }
                }
            }
            VideoEscalationReason::Reconfigure
            | VideoEscalationReason::TransportExpiredDeadline => {
                self.pending_keyframe_signals = 0;
                self.pending_decoder_reset_signals =
                    self.pending_decoder_reset_signals.saturating_add(1);
                if self.pending_decoder_reset_signals < self.decoder_reset_burst_threshold {
                    "waitForDecoderResetBurst"
                } else if self
                    .last_decoder_reset_at
                    .map_or(true, |last| last.elapsed() >= self.cooldown)
                {
                    self.pending_decoder_reset_signals = 0;
                    self.reconnect_candidate_signals =
                        self.reconnect_candidate_signals.saturating_add(1);
                    self.last_decoder_reset_at = Some(now);
                    self.last_keyframe_request_at = Some(now);
                    "requestDecoderReset"
                } else if self.reconnect_candidate_signals
                    >= self.decoder_reset_burst_threshold.saturating_add(1)
                {
                    "requestReconnectCandidate"
                } else {
                    "cooldownSuppressed"
                }
            }
            VideoEscalationReason::TransportSevereDeadline => {
                // 大洞 deadline 失效通常说明这一段视频已经不可救，
                // 这里直接跳过 keyframe burst，优先推到更高一级恢复。
                self.pending_keyframe_signals = 0;
                self.pending_decoder_reset_signals = 0;
                self.reconnect_candidate_signals =
                    self.reconnect_candidate_signals.saturating_add(1);
                self.last_severe_deadline_at = Some(now);
                if self
                    .last_decoder_reset_at
                    .map_or(true, |last| last.elapsed() >= self.cooldown)
                {
                    self.last_decoder_reset_at = Some(now);
                    self.last_keyframe_request_at = Some(now);
                    "requestDecoderReset"
                } else if self.reconnect_candidate_signals >= 2 {
                    "requestReconnectCandidate"
                } else {
                    "cooldownSuppressed"
                }
            }
        };
        VideoEscalationDecision {
            observation_id: self.next_observation_id,
            action,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{VideoEscalationController, VideoEscalationReason};
    use std::time::Duration;

    #[test]
    fn waits_for_burst_before_requesting_keyframe() {
        let mut controller = VideoEscalationController::new(Duration::from_millis(250), 2, 2);

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::WaitKeyframe)
                .action,
            "waitForBurst"
        );
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::AdapterIdleTimeout)
                .action,
            "requestKeyframe"
        );
    }

    #[test]
    fn repeated_transport_deadline_failures_escalate_to_reconnect_candidate() {
        let mut controller = VideoEscalationController::new(Duration::from_secs(60), 2, 1);

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportExpiredDeadline)
                .action,
            "requestDecoderReset"
        );
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportExpiredDeadline)
                .action,
            "cooldownSuppressed"
        );
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportExpiredDeadline)
                .action,
            "requestReconnectCandidate"
        );
    }

    #[test]
    fn severe_transport_deadline_shortcuts_to_decoder_reset_then_reconnect() {
        let mut controller = VideoEscalationController::new(Duration::from_secs(60), 2, 2);

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportSevereDeadline)
                .action,
            "requestDecoderReset"
        );
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportSevereDeadline)
                .action,
            "requestReconnectCandidate"
        );
    }

    #[test]
    fn adapter_idle_after_severe_deadline_escalates_to_reconnect_candidate() {
        let mut controller = VideoEscalationController::new(Duration::from_millis(200), 2, 2);

        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::TransportSevereDeadline)
                .action,
            "requestDecoderReset"
        );
        assert_eq!(
            controller
                .on_reason(VideoEscalationReason::AdapterIdleTimeout)
                .action,
            "requestReconnectCandidate"
        );
    }
}
