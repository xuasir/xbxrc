use crate::transport::rtc::facts::{MediaFact, PeerFact, TransportFact};

#[derive(Clone, Debug, PartialEq, Default)]
pub struct DiagnosticsProjection {
    pub latest_label: Option<String>,
    pub latest_summary: Option<String>,
    pub last_observed_at_ms: Option<f64>,
}

impl DiagnosticsProjection {
    pub fn apply_fact(&mut self, fact: &TransportFact) {
        match fact {
            TransportFact::Peer(PeerFact::ConnectionStateChanged {
                state,
                observed_at_ms,
            }) => {
                self.latest_label = Some("peer.connectionState".to_string());
                self.latest_summary = Some(format!("state={state:?}"));
                self.last_observed_at_ms = Some(*observed_at_ms);
            }
            TransportFact::Media(MediaFact::FrameArrived {
                width,
                height,
                is_keyframe,
                observed_at_ms,
                ..
            }) => {
                self.latest_label = Some("media.frameArrived".to_string());
                self.latest_summary = Some(format!(
                    "resolution={}x{} keyframe={}",
                    width, height, is_keyframe
                ));
                self.last_observed_at_ms = Some(*observed_at_ms);
            }
            TransportFact::CommandResult(result) => {
                self.latest_label = Some("command.result".to_string());
                self.latest_summary = Some(format!("status={:?}", result.status));
                self.last_observed_at_ms = Some(result.observed_at_ms);
            }
            _ => {}
        }
    }
}
