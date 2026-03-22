use crate::transport::rtc::facts::{
    CommandResultStatus, PeerFact, TimerFact, TransportCommand, TransportFact,
};

#[derive(Clone, Debug, PartialEq, Default)]
pub struct BweProjection {
    pub latest_rtt_ms: Option<f64>,
    pub latest_loss_ratio_1s: Option<f64>,
    pub latest_actual_video_bitrate_kbps: Option<f64>,
    pub latest_observed_remb_kbps: Option<u32>,
    pub latest_transport_path: Option<String>,
    pub latest_sample_tick_ms: Option<f64>,
    pub target_remb_kbps: Option<u32>,
    pub last_observed_at_ms: Option<f64>,
}

impl BweProjection {
    pub fn apply_fact(&mut self, fact: &TransportFact) {
        match fact {
            TransportFact::Peer(PeerFact::TransportMetricsSampled {
                video_rtt_ms,
                loss_ratio_1s,
                actual_video_bitrate_kbps,
                observed_remb_kbps,
                transport_path,
                observed_at_ms,
            }) => {
                self.latest_rtt_ms = *video_rtt_ms;
                self.latest_loss_ratio_1s = Some(*loss_ratio_1s);
                self.latest_actual_video_bitrate_kbps = *actual_video_bitrate_kbps;
                self.latest_observed_remb_kbps = *observed_remb_kbps;
                self.latest_transport_path = transport_path.clone();
                self.last_observed_at_ms = Some(*observed_at_ms);
            }
            TransportFact::Timer(TimerFact::MetricsSampleTick { observed_at_ms }) => {
                self.latest_sample_tick_ms = Some(*observed_at_ms);
                self.last_observed_at_ms = Some(*observed_at_ms);
            }
            TransportFact::CommandResult(result) => {
                if matches!(result.status, CommandResultStatus::Succeeded) {
                    if let TransportCommand::SetTargetRembKbps { target_kbps, .. } = result.command
                    {
                        self.target_remb_kbps = Some(target_kbps);
                        self.last_observed_at_ms = Some(result.observed_at_ms);
                    }
                }
            }
            _ => {}
        }
    }
}
