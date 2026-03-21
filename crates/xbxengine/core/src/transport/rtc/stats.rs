use std::sync::{Arc, Mutex};

use xbxengine_protocol::XbxEngineTransportStateDto;

use crate::XbxEngineMediaRuntimeStats;

pub(crate) fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

pub(crate) fn apply_transport_event(
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    state: XbxEngineTransportStateDto,
    label: &str,
    summary: &str,
) {
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_state = state;
        stats.latest_observation_label = Some(label.to_string());
        stats.latest_observation_summary = Some(summary.to_string());
    }
}
