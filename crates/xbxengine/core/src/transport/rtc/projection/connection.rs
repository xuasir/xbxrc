use crate::transport::rtc::facts::{
    ConnectionLifecycleStateFact, DataChannelLabelFact, PeerFact, TransportFact,
};

#[derive(Clone, Debug, PartialEq)]
pub struct ConnectionProjection {
    pub lifecycle_state: ConnectionLifecycleStateFact,
    pub local_candidate_total: u64,
    pub latest_candidate_kind: Option<String>,
    pub latest_transport_path: Option<String>,
    pub latest_rtt_ms: Option<f64>,
    pub latest_loss_ratio_1s: Option<f64>,
    pub control_channel_open: bool,
    pub message_channel_open: bool,
    pub input_channel_open: bool,
    pub chat_channel_open: bool,
    pub last_observed_at_ms: Option<f64>,
}

impl Default for ConnectionProjection {
    fn default() -> Self {
        Self {
            lifecycle_state: ConnectionLifecycleStateFact::New,
            local_candidate_total: 0,
            latest_candidate_kind: None,
            latest_transport_path: None,
            latest_rtt_ms: None,
            latest_loss_ratio_1s: None,
            control_channel_open: false,
            message_channel_open: false,
            input_channel_open: false,
            chat_channel_open: false,
            last_observed_at_ms: None,
        }
    }
}

impl ConnectionProjection {
    pub fn apply_fact(&mut self, fact: &TransportFact) {
        let TransportFact::Peer(peer_fact) = fact else {
            return;
        };
        match peer_fact {
            PeerFact::ConnectionStateChanged {
                state,
                observed_at_ms,
            } => {
                self.lifecycle_state = *state;
                self.last_observed_at_ms = Some(*observed_at_ms);
            }
            PeerFact::DataChannelOpened {
                label,
                observed_at_ms,
            } => {
                self.set_channel_state(label, true);
                self.last_observed_at_ms = Some(*observed_at_ms);
            }
            PeerFact::DataChannelClosed {
                label,
                observed_at_ms,
            } => {
                self.set_channel_state(label, false);
                self.last_observed_at_ms = Some(*observed_at_ms);
            }
            PeerFact::DataChannelBufferedAmountHigh { observed_at_ms, .. }
            | PeerFact::DataChannelBufferedAmountLow { observed_at_ms, .. } => {
                self.last_observed_at_ms = Some(*observed_at_ms);
            }
            PeerFact::TransportMetricsSampled {
                video_rtt_ms,
                loss_ratio_1s,
                transport_path,
                observed_at_ms,
                ..
            } => {
                self.latest_rtt_ms = *video_rtt_ms;
                self.latest_loss_ratio_1s = Some(*loss_ratio_1s);
                self.latest_transport_path = transport_path.clone();
                self.last_observed_at_ms = Some(*observed_at_ms);
            }
        }
    }

    fn set_channel_state(&mut self, label: &DataChannelLabelFact, open: bool) {
        match label {
            DataChannelLabelFact::Control => self.control_channel_open = open,
            DataChannelLabelFact::Message => self.message_channel_open = open,
            DataChannelLabelFact::Input => self.input_channel_open = open,
            DataChannelLabelFact::Chat => self.chat_channel_open = open,
        }
    }
}
