use std::sync::{Arc, Mutex};

use super::observation_bus::{ObservationBus, ObservationEvent};
use crate::XbxEngineMediaRuntimeStats;
use crate::XbxEngineVideoTimelineChainSnapshot;
use crate::XbxEngineVideoTimelineObservation;

#[test]
fn video_timeline_observed_updates_latest_timeline_field() {
    let stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let bus = ObservationBus::new(stats.clone());

    bus.publish(ObservationEvent::VideoTimelineObserved {
        observation: XbxEngineVideoTimelineObservation {
            observation_id: 9,
            source_event: "gap-repair-in-flight".to_string(),
            gap: None,
            frame: None,
            chain: XbxEngineVideoTimelineChainSnapshot {
                state: "repairing".to_string(),
                reason: Some("gapRepairInFlight".to_string()),
                chain_break_evidence: None,
                observed_at_ms: 1.0,
            },
            observed_at_ms: 1.0,
        },
    });

    let locked = stats.lock().expect("stats lock");
    let obs = locked
        .latest_video_timeline_observation
        .as_ref()
        .expect("timeline observation");
    assert_eq!(obs.observation_id, 9);
    assert_eq!(obs.chain.state, "repairing");
}

#[test]
fn inbound_packet_loss_estimate_increments_total() {
    let stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let bus = ObservationBus::new(stats.clone());

    bus.publish(ObservationEvent::InboundVideoPacketLossEstimate { packet_count: 3 });
    bus.publish(ObservationEvent::InboundVideoPacketLossEstimate { packet_count: 2 });

    let locked = stats.lock().expect("stats lock");
    assert_eq!(locked.inbound_video_packet_loss_estimate_total, 5);
}
