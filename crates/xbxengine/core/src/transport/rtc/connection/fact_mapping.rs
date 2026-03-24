use crate::transport::rtc::events::RtcConnectionLifecycleState;
use crate::transport::rtc::facts::{ConnectionLifecycleStateFact, DataChannelLabelFact};

pub(crate) fn map_data_channel_label_fact(label: &str) -> Option<DataChannelLabelFact> {
    match label {
        "control" => Some(DataChannelLabelFact::Control),
        "message" => Some(DataChannelLabelFact::Message),
        "input" => Some(DataChannelLabelFact::Input),
        "chat" => Some(DataChannelLabelFact::Chat),
        _ => None,
    }
}

pub(crate) fn map_connection_lifecycle_state_fact(
    state: RtcConnectionLifecycleState,
) -> ConnectionLifecycleStateFact {
    match state {
        RtcConnectionLifecycleState::New => ConnectionLifecycleStateFact::New,
        RtcConnectionLifecycleState::Connecting => ConnectionLifecycleStateFact::Connecting,
        RtcConnectionLifecycleState::Connected => ConnectionLifecycleStateFact::Connected,
        RtcConnectionLifecycleState::Disconnected => ConnectionLifecycleStateFact::Disconnected,
        RtcConnectionLifecycleState::Recovering => ConnectionLifecycleStateFact::Recovering,
        RtcConnectionLifecycleState::Failed => ConnectionLifecycleStateFact::Failed,
        RtcConnectionLifecycleState::Closed => ConnectionLifecycleStateFact::Closed,
    }
}
