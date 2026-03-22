use crate::transport::rtc::facts::{IngressDecisionFact, MediaFact, TransportFact};

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MediaProjection {
    pub frame_count: u64,
    pub latest_frame_resolution: Option<(u32, u32)>,
    pub latest_frame_keyframe: Option<bool>,
    pub ingress_queue_depth: Option<usize>,
    pub latest_transport_observation_label: Option<String>,
    pub wait_keyframe_count: u64,
    pub reconfigure_count: u64,
    pub last_observed_at_ms: Option<f64>,
}

impl MediaProjection {
    pub fn apply_fact(&mut self, fact: &TransportFact) {
        let TransportFact::Media(media_fact) = fact else {
            return;
        };
        match media_fact {
            MediaFact::FrameArrived {
                width,
                height,
                is_keyframe,
                observed_at_ms,
                ..
            } => {
                self.frame_count = self.frame_count.saturating_add(1);
                self.latest_frame_resolution = Some((*width, *height));
                self.latest_frame_keyframe = Some(*is_keyframe);
                self.last_observed_at_ms = Some(*observed_at_ms);
            }
            MediaFact::TransportObservationRaised {
                label,
                observed_at_ms,
                ..
            } => {
                self.latest_transport_observation_label = Some(label.clone());
                self.last_observed_at_ms = Some(*observed_at_ms);
            }
            MediaFact::IngressDecisionObserved {
                decision,
                queue_depth,
                observed_at_ms,
            } => {
                self.ingress_queue_depth = Some(*queue_depth);
                self.last_observed_at_ms = Some(*observed_at_ms);
                match decision {
                    IngressDecisionFact::WaitKeyframe => {
                        self.wait_keyframe_count = self.wait_keyframe_count.saturating_add(1);
                    }
                    IngressDecisionFact::Reconfigure => {
                        self.reconfigure_count = self.reconfigure_count.saturating_add(1);
                    }
                    IngressDecisionFact::Submit
                    | IngressDecisionFact::DropLate
                    | IngressDecisionFact::DropBacklog => {}
                }
            }
        }
    }
}
