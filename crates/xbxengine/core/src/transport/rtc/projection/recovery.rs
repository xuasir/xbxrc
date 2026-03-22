use crate::transport::rtc::facts::{CommandResultStatus, MediaFact, TransportFact};

#[derive(Clone, Debug, PartialEq, Default)]
pub struct RecoveryProjection {
    pub latest_diagnosis_label: Option<String>,
    pub pending_action: bool,
    pub successful_action_count: u64,
    pub failed_action_count: u64,
    pub last_observed_at_ms: Option<f64>,
}

impl RecoveryProjection {
    pub fn apply_fact(&mut self, fact: &TransportFact) {
        match fact {
            TransportFact::Media(MediaFact::TransportObservationRaised {
                label,
                observed_at_ms,
                ..
            }) => {
                self.latest_diagnosis_label = Some(label.clone());
                self.last_observed_at_ms = Some(*observed_at_ms);
            }
            TransportFact::CommandResult(result) => {
                self.pending_action = false;
                self.last_observed_at_ms = Some(result.observed_at_ms);
                match result.status {
                    CommandResultStatus::Succeeded => {
                        self.successful_action_count =
                            self.successful_action_count.saturating_add(1);
                    }
                    CommandResultStatus::Failed { .. } => {
                        self.failed_action_count = self.failed_action_count.saturating_add(1);
                    }
                }
            }
            _ => {}
        }
    }
}
