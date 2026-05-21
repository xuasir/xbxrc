// RFC receiver-local NACK 合同：seq + RTT 时序 + 本地优先级 + OOS 跳过。

use super::*;
use std::sync::{Arc, Mutex};

use crate::media::video::ingress::budget::{FrameBudgetContext, FrameBudgetWindowSource};
use crate::media::video::types::FrameValue;
use crate::transport::rtc::receive::nack_policy::{
    cloud_nack_max_age_ms, cloud_startup_head_hole_deadline_at_ms,
};
use crate::transport::rtc::stream::nack_contract::{NackBatch, PacketRecoveryDisposition};
use crate::transport::rtc::stream::sink::RtcRtcpSendPort;
use bytes::Bytes;
use rtc_rtcp::transport_feedbacks::transport_layer_nack::TransportLayerNack;
#[test]
fn cloud_nack_windows_follow_rtt_without_floor() {
    let now_ms = 1_000.0;
    let base_deadline_at_ms = 1_120.0;

    let adjusted_deadline = cloud_startup_head_hole_deadline_at_ms(
        now_ms,
        base_deadline_at_ms,
        true,
        false,
        Some(90.0),
        None,
    );
    let adjusted_max_age = cloud_nack_max_age_ms(100, true, false, Some(90.0));

    assert_eq!(adjusted_deadline, 1_170.0);
    assert_eq!(adjusted_max_age, 170);
}

#[test]
fn non_cloud_nack_windows_remain_unchanged() {
    let now_ms = 1_000.0;
    let base_deadline_at_ms = 1_120.0;

    let adjusted_deadline = cloud_startup_head_hole_deadline_at_ms(
        now_ms,
        base_deadline_at_ms,
        false,
        false,
        Some(90.0),
        None,
    );
    let adjusted_max_age = cloud_nack_max_age_ms(180, false, false, Some(90.0));

    assert_eq!(adjusted_deadline, base_deadline_at_ms);
    assert_eq!(adjusted_max_age, 180);
}

#[test]
fn repair_value_tier_marks_delta_as_low_value_on_cloud_high_rtt() {
    assert_eq!(
        classify_repair_value_tier(
            FrameBudgetContext::for_transport(
                FrameValue::new(false, false, 8 * 1024),
                false,
                Some(160.0),
                Some(1_030.0),
                Some(1_040.0),
                false,
                FrameBudgetWindowSource::Transport,
            ),
            false,
            false,
        ),
        FrameBudgetLinkValue::Disposable
    );
}

#[test]
fn repair_value_tier_keeps_reference_as_anchor_while_waiting_keyframe() {
    assert_eq!(
        classify_repair_value_tier(
            FrameBudgetContext::for_transport(
                FrameValue::new(false, true, 48 * 1024),
                true,
                Some(140.0),
                Some(1_020.0),
                Some(1_050.0),
                false,
                FrameBudgetWindowSource::Recovery,
            ),
            false,
            false,
        ),
        FrameBudgetLinkValue::Anchor
    );
}

fn make_test_source(
    transport_capability: Arc<dyn crate::transport::rtc::capability::RtcTransportCapability>,
) -> RtcVideoFrameSource {
    let (_tx, rx) = tokio::sync::mpsc::channel(1);
    let (transport_observation_tx, _transport_observation_rx) =
        tokio::sync::mpsc::unbounded_channel();
    let rtcp_port: Arc<dyn RtcRtcpSendPort> = Arc::new(CaptureRtcpPort::default());
    let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
    RtcVideoFrameSource::new(
        rx,
        transport_observation_tx,
        rtcp_port,
        runtime_stats,
        16,
        std::time::Duration::from_millis(10),
        std::time::Duration::from_millis(20),
        std::time::Duration::from_millis(200),
        crate::transport::rtc::receive::test_nack_scheduler_config(),
        transport_capability,
    )
}

#[tokio::test]
async fn cloud_sample_loss_observation_includes_rtt_merged_deadline() {
    use xbxengine_protocol::XbxEngineTargetTypeDto;

    let transport_capability = Arc::new(CaptureTransportCapability {
        port: Arc::new(CaptureRtcpPort::default()),
    });
    let mut source = make_test_source(transport_capability);
    source.current_media_ssrc = Some(0x1234_5678);
    source.local_rtcp_sender_ssrc = 0x1122_3344;
    source.runtime_stats.update(|stats| {
        stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
        stats.video_rtt_ms = Some(90.0);
        stats.transport_policy_profile = Some("cloudGaming".to_string());
    });
    source
        .receive_core_mut()
        .receive_engine
        .observe_rtp_sequence(100, 0.0);
    source
        .receive_core_mut()
        .receive_engine
        .observe_rtp_sequence(103, 1.0);

    let before_ms = crate::transport::rtc::receive::now_ms_f64();
    let started = source
        .observe_sample_loss_and_nack(90_001, 2, false, "supply")
        .await;
    assert!(
        started,
        "cloud sample-loss should register nack like non-cloud path"
    );
    let observation = source
        .runtime_stats
        .read(|stats| stats.latest_video_nack_observation.clone())
        .expect("stats lock");
    let deadline_at_ms = observation
        .and_then(|o| o.deadline_at_ms)
        .expect("cloud sample-loss nack should carry admission deadline");
    assert!(
        deadline_at_ms >= before_ms + 150.0,
        "cloud 路径应合并 RTT margin 与动态 NACK 超时，deadline={deadline_at_ms} before={before_ms}"
    );
}

#[tokio::test]
async fn sample_loss_registers_pending_nack_without_chain_broken_escalation() {
    let transport_capability = Arc::new(CaptureTransportCapability {
        port: Arc::new(CaptureRtcpPort::default()),
    });
    let mut source = make_test_source(transport_capability);
    source.current_media_ssrc = Some(0x4455_6677);
    source.local_rtcp_sender_ssrc = 0x1122_3344;
    source
        .receive_core_mut()
        .receive_engine
        .observe_rtp_sequence(100, 0.0);
    source
        .receive_core_mut()
        .receive_engine
        .observe_rtp_sequence(103, 1.0);

    let started = source
        .observe_sample_loss_and_nack(90_001, 2, false, "supply")
        .await;
    assert!(started);
    assert!(source.receive_core().receive_engine.pending_nack_count() > 0);
}

#[tokio::test]
async fn low_value_sample_loss_skipped_under_burst_oos() {
    let transport_capability = Arc::new(CaptureTransportCapability {
        port: Arc::new(CaptureRtcpPort::default()),
    });
    let mut source = make_test_source(transport_capability);
    source.sample_loss_burst_count = 4;
    let now_ms = crate::transport::rtc::receive::now_ms_f64();
    source.recent_oos_active_until_ms = Some(now_ms + 5_000.0);

    let started = source
        .observe_sample_loss_and_nack(90_002, 1, false, "disposable")
        .await;
    assert!(!started);
}

#[derive(Default)]
struct CaptureRtcpPort {
    payloads: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl RtcRtcpSendPort for CaptureRtcpPort {
    fn send_rtcp(&self, payload: &[u8]) -> Result<(), String> {
        self.payloads
            .lock()
            .expect("payloads lock")
            .push(payload.to_vec());
        Ok(())
    }
}

struct CaptureTransportCapability {
    port: Arc<CaptureRtcpPort>,
}

impl crate::transport::rtc::capability::RtcTransportCapability for CaptureTransportCapability {
    fn video_feedback_state(&self) -> crate::transport::rtc::capability::VideoFeedbackState {
        crate::transport::rtc::capability::VideoFeedbackState::Ready
    }

    fn send_nack_rtcp(
        &self,
        payload: &[u8],
    ) -> Result<(), crate::transport::rtc::capability::TransportCapabilityError> {
        self.port.send_rtcp(payload).map_err(|detail| {
            crate::transport::rtc::capability::TransportCapabilityError::SendFailed { detail }
        })
    }

    fn send_keyframe(
        &self,
        _kind: crate::transport::rtc::capability::KeyframeRequestKind,
    ) -> crate::transport::rtc::capability::KeyframeSendOutcome {
        crate::transport::rtc::capability::KeyframeSendOutcome::Sent
    }

    fn send_remb(
        &self,
        _kbps: u32,
    ) -> Result<(), crate::transport::rtc::capability::TransportCapabilityError> {
        Ok(())
    }

    fn latest_rtt_ms(&self) -> Option<u32> {
        None
    }
}

#[tokio::test]
async fn send_nack_batch_uses_real_media_and_sender_ssrc() {
    let capture = Arc::new(CaptureRtcpPort::default());
    let payloads = capture.payloads.clone();
    let transport_capability: Arc<dyn crate::transport::rtc::capability::RtcTransportCapability> =
        Arc::new(CaptureTransportCapability {
            port: capture.clone(),
        });
    let mut source = make_test_source(transport_capability);
    source.current_media_ssrc = Some(0x4455_6677);
    source.local_rtcp_sender_ssrc = 0x1122_3344;

    let batch = NackBatch {
        sequences: vec![12, 13, 15],
        retry_count: 1,
        source: "sampleLoss",
        frame_rtp_timestamp: Some(0x0102_0304),
        frame_is_keyframe: Some(false),
        frame_importance: "supply",
        deadline_at_ms: None,
        estimated_recovery_arrival_ms: None,
        frame_playout_deadline_at_ms: None,
        nack_disposition: PacketRecoveryDisposition::Attempted,
        frame_unrecoverable_reason: None,
        budget_context: FrameBudgetContext::for_transport(
            FrameValue::new(false, false, 8 * 1024),
            false,
            None,
            None,
            None,
            false,
            FrameBudgetWindowSource::Transport,
        ),
    };

    source
        .send_nack_batch("sent", &batch, 1_000.0)
        .await
        .expect("send should succeed");

    let captured = payloads.lock().expect("payloads lock");
    assert_eq!(captured.len(), 1);
    let mut raw = Bytes::copy_from_slice(&captured[0]);
    let packets = rtc_rtcp::packet::unmarshal(&mut raw).expect("nack payload should parse");
    let nack = packets
        .into_iter()
        .find_map(|packet| {
            packet
                .as_any()
                .downcast_ref::<TransportLayerNack>()
                .cloned()
        })
        .expect("expected transport layer nack");
    assert_eq!(nack.media_ssrc, 0x4455_6677);
    assert_eq!(nack.sender_ssrc, 0x1122_3344);
}
