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
                    CommandResultStatus::Deferred { .. } => {}
                    CommandResultStatus::Failed { .. } => {
                        self.failed_action_count = self.failed_action_count.saturating_add(1);
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RecoveryProjection;
    use crate::transport::rtc::facts::{
        CommandResultFact, CommandResultStatus, TransportCommand, TransportFact,
    };

    #[test]
    fn deferred_command_result_does_not_increment_failed_action_count() {
        let mut projection = RecoveryProjection::default();
        projection.apply_fact(&TransportFact::CommandResult(CommandResultFact {
            command: TransportCommand::RequestReconnectCandidate {
                reason: "recovering-stream".to_string(),
                observation_id: 7,
            },
            status: CommandResultStatus::Deferred {
                reason: "pendingReason=existing".to_string(),
            },
            observed_at_ms: 10.0,
        }));
        assert_eq!(projection.successful_action_count, 0);
        assert_eq!(projection.failed_action_count, 0);
        assert_eq!(projection.last_observed_at_ms, Some(10.0));
    }
}
