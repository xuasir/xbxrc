use crate::{
    media::video::ingress::scheduler::IngressDecision, runtime_stats_sink::RuntimeStatsSink,
    transport::rtc::stream::adapter_types::TransportObservation,
    XbxEngineVideoFrameDropObservation,
};
use std::collections::VecDeque;

pub(super) struct MediaSupervisorObservationState {
    frame_count: u64,
    frame_drop_observation_id: u64,
    recent_receive_frame_times_ms: VecDeque<f64>,
    last_transport_escalation_hint: Option<(String, f64)>,
}

impl MediaSupervisorObservationState {
    pub(super) fn new() -> Self {
        Self {
            frame_count: 0,
            frame_drop_observation_id: 0,
            recent_receive_frame_times_ms: VecDeque::new(),
            last_transport_escalation_hint: None,
        }
    }

    pub(super) fn record_frame_arrival(
        &mut self,
        runtime_stats: &RuntimeStatsSink,
        now_ms: f64,
    ) -> u64 {
        self.frame_count = self.frame_count.saturating_add(1);
        self.recent_receive_frame_times_ms.push_back(now_ms);
        trim_recent_times(&mut self.recent_receive_frame_times_ms, now_ms);
        runtime_stats.record_frame_arrival(
            now_ms,
            self.frame_count,
            calculate_recent_fps(&self.recent_receive_frame_times_ms),
        );
        self.frame_count
    }

    pub(super) fn record_stream_dimensions(
        &self,
        runtime_stats: &RuntimeStatsSink,
        width: u32,
        height: u32,
    ) {
        runtime_stats.record_stream_dimensions(width, height);
    }

    pub(super) fn record_ingress_observation(
        &mut self,
        runtime_stats: &RuntimeStatsSink,
        decision: &IngressDecision,
        reconfigure_reason: Option<&str>,
        observed_at_ms: f64,
        width: u32,
        height: u32,
        is_keyframe: bool,
        queue_depth: usize,
    ) {
        if !matches!(
            decision,
            IngressDecision::DropLate
                | IngressDecision::DropBacklog
                | IngressDecision::WaitKeyframe
                | IngressDecision::Reconfigure
        ) {
            return;
        }

        self.frame_drop_observation_id = self.frame_drop_observation_id.saturating_add(1);
        runtime_stats.record_video_frame_drop(XbxEngineVideoFrameDropObservation {
            observation_id: self.frame_drop_observation_id,
            reason: map_ingress_drop_reason(decision, reconfigure_reason),
            observed_at_ms,
            width,
            height,
            is_keyframe,
            queue_depth,
        });
    }

    pub(super) fn total_frame_count(&self) -> u64 {
        self.frame_count
    }

    pub(super) fn should_log_transport_hint(&self, label: &str, now_ms: f64) -> bool {
        match self.last_transport_escalation_hint.as_ref() {
            Some((last_label, last_at_ms)) => {
                last_label != label || now_ms - *last_at_ms >= 1_000.0
            }
            None => true,
        }
    }

    pub(super) fn record_transport_hint(&mut self, label: String, now_ms: f64) {
        self.last_transport_escalation_hint = Some((label, now_ms));
    }
}

fn trim_recent_times(times: &mut VecDeque<f64>, now_ms: f64) {
    while let Some(front) = times.front().copied() {
        if now_ms - front <= 1_000.0 {
            break;
        }
        times.pop_front();
    }
}

fn calculate_recent_fps(times: &VecDeque<f64>) -> f64 {
    let len = times.len();
    if len < 2 {
        return 0.0;
    }
    let first = times.front().copied().unwrap_or_default();
    let last = times.back().copied().unwrap_or(first);
    let window_ms = (last - first).max(1.0);
    ((len.saturating_sub(1)) as f64 * 1_000.0 / window_ms).max(0.0)
}

fn map_ingress_drop_reason(decision: &IngressDecision, reconfigure_reason: Option<&str>) -> String {
    match decision {
        IngressDecision::Submit => "submit".to_string(),
        IngressDecision::DropLate => "dropLate".to_string(),
        IngressDecision::DropBacklog => "dropBacklog".to_string(),
        IngressDecision::WaitKeyframe => "waitKeyframe".to_string(),
        IngressDecision::Reconfigure => {
            if let Some(reason) = reconfigure_reason {
                format!("reconfigure:{reason}")
            } else {
                "reconfigure".to_string()
            }
        }
    }
}

pub(super) fn map_transport_observation_to_hint_label(
    observation: &TransportObservation,
    severe_deadline_packet_threshold: usize,
) -> &'static str {
    match observation {
        TransportObservation::Admission(
            crate::transport::rtc::stream::adapter_types::TransportAdmissionObservation::AwaitRecoveryKeyframe,
        ) => "transportAwaitRecoveryKeyframe",
        TransportObservation::Loss(
            crate::transport::rtc::stream::adapter_types::TransportLossObservation::PacketLossDetected,
        ) => "transportSampleLoss",
        TransportObservation::Loss(
            crate::transport::rtc::stream::adapter_types::TransportLossObservation::RecoveryKeyframeRequested,
        ) => "transportSampleLossBurst",
        TransportObservation::Loss(
            crate::transport::rtc::stream::adapter_types::TransportLossObservation::AwaitRecoveryKeyframe,
        ) => "transportAwaitRecoveryKeyframe",
        TransportObservation::StreamIdleTimeout => "adapterIdleTimeout",
        TransportObservation::StreamThinStall => "adapterThinStream",
        TransportObservation::NackRecoveredLate => "transportRecoveredLate",
        TransportObservation::NackDeadlineExpired { missing_packets } => {
            if usize::from(*missing_packets) >= severe_deadline_packet_threshold {
                "transportSevereDeadline"
            } else {
                "transportExpiredDeadline"
            }
        }
    }
}

pub(super) fn transport_observation_severity(observation: &TransportObservation) -> u8 {
    match observation {
        TransportObservation::NackDeadlineExpired { missing_packets } if *missing_packets >= 64 => {
            2
        }
        TransportObservation::StreamIdleTimeout
        | TransportObservation::StreamThinStall
        | TransportObservation::NackDeadlineExpired { .. } => 1,
        TransportObservation::Admission(_)
        | TransportObservation::Loss(_)
        | TransportObservation::NackRecoveredLate => 0,
    }
}
