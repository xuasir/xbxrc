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

#[test]
fn ice_connectivity_probe_updates_structured_stats() {
    let stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let bus = ObservationBus::new(stats.clone());

    bus.publish(ObservationEvent::IceConnectivityProbe {
        candidate_pair_count: 4,
        nominated_pair_count: 0,
        succeeded_pair_count: 0,
        in_progress_pair_count: 4,
        failed_pair_count: 0,
        max_requests_sent: 35,
        max_responses_received: 0,
        responses_received_total: 0,
        has_selected_or_nominated_pair: false,
        direct_checks_without_response: true,
        local_candidate_type_summary: "host=1 srflx=1 prflx=0 relay=0 unknown=0".to_string(),
        remote_candidate_type_summary: "host=2 srflx=1 prflx=0 relay=0 unknown=0".to_string(),
        address_family_summary: "ipv4=2 ipv6=1 mixed=1 unknown=0".to_string(),
        observed_at_ms: 12_345.0,
    });

    let locked = stats.lock().expect("stats lock");
    let probe = locked
        .latest_ice_connectivity_probe
        .as_ref()
        .expect("ice probe");
    assert_eq!(probe.candidate_pair_count, 4);
    assert_eq!(probe.max_requests_sent, 35);
    assert_eq!(probe.responses_received_total, 0);
    assert!(probe.direct_checks_without_response);
    assert_eq!(probe.observed_at_ms, 12_345.0);
    assert_eq!(
        locked.latest_observation_label.as_deref(),
        Some("iceConnectivityProbe")
    );
}
