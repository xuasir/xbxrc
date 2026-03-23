use super::{build_sample_builder, now_ms_f64, UINT16SIZE_HALF};
use crate::media::video::h264::inspection::H264AccessUnitInspection;
use bytes::Bytes;

use crate::media::video::types::{AssembledVideoFrame, FrameValue, VideoCodec};
use crate::transport::rtc::stream::adapter_types::{
    FrameSource, TransportAdmissionObservation, TransportLossObservation, TransportObservation,
    TransportObservationSource,
};
use crate::transport::rtc::stream::video_source::{
    RtcVideoFrameSource, RtcVideoTransportObservationSource,
};

use crate::XbxEngineVideoRtxReinjectObservation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RecoveryKeyframeAction {
    Submit,
    DropAndRequestKeyframe,
    TriggerWaitKeyframe,
    WaitKeyframe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InspectionAdmission {
    Accept,
    AwaitRecoveryKeyframe,
}

pub(super) fn resolve_inspection_admission(
    inspection: &H264AccessUnitInspection,
) -> InspectionAdmission {
    if !inspection.slice_headers_valid {
        return InspectionAdmission::AwaitRecoveryKeyframe;
    }

    InspectionAdmission::Accept
}

pub(super) fn resolve_recovery_keyframe_action(
    waiting_for_recovery_keyframe: bool,
    sample_loss_burst_count: u8,
    media_dropped_packets: u16,
    is_keyframe: bool,
) -> (bool, RecoveryKeyframeAction) {
    // 带丢包的 keyframe/reference 不能继续喂给解码器，否则很容易把本地参考链喂脏，
    // 在 macOS 上会直接放大成 VideoToolbox 连续 bad-data 回调。
    if is_keyframe && media_dropped_packets > 0 {
        return (true, RecoveryKeyframeAction::TriggerWaitKeyframe);
    }

    if is_keyframe {
        return (false, RecoveryKeyframeAction::Submit);
    }

    if media_dropped_packets > 0 {
        if sample_loss_burst_count >= 2 {
            return (true, RecoveryKeyframeAction::TriggerWaitKeyframe);
        }
        return (false, RecoveryKeyframeAction::DropAndRequestKeyframe);
    }

    if waiting_for_recovery_keyframe {
        return (true, RecoveryKeyframeAction::WaitKeyframe);
    }

    (false, RecoveryKeyframeAction::Submit)
}

pub(super) fn detect_forward_gap(
    last_highest_rtp_sequence: Option<u16>,
    sequence: u16,
) -> (Option<u16>, Option<(u16, u16)>) {
    let Some(last_highest) = last_highest_rtp_sequence else {
        return (Some(sequence), None);
    };
    let diff = sequence.wrapping_sub(last_highest);
    if diff == 0 {
        return (Some(last_highest), None);
    }
    if diff < UINT16SIZE_HALF {
        if diff > 1 {
            return (
                Some(sequence),
                Some((last_highest.wrapping_add(1), sequence)),
            );
        }
        return (Some(sequence), None);
    }

    (Some(last_highest), None)
}

impl RtcVideoFrameSource {
    fn should_trigger_thin_stream_stall(&self, now: std::time::Instant) -> bool {
        self.assembling_frame_start.is_some_and(|started_at| {
            now.duration_since(started_at) >= self.assembly_stall_timeout
                && self.current_assembly_packet_count > 0
                && self.current_assembly_packet_count <= self.thin_stream_packet_threshold
        })
    }

    fn should_prioritize_reinject_drain(&self) -> bool {
        self.runtime_stats
            .read(|stats| stats.latest_video_rtx_reinject_observation.clone())
            .flatten()
            .is_some_and(|observation| {
                observation.stage == "queued" && observation.matched_head_gap
            })
    }
}

impl RtcVideoFrameSource {
    pub(super) async fn recv_frame_inner(&mut self) -> Option<AssembledVideoFrame> {
        loop {
            self.maybe_run_nack_maintenance().await;
            if let Some(sample) = self.sample_builder.pop() {
                self.last_packet_time = std::time::Instant::now();
                self.assembling_frame_start = None;
                self.current_assembly_packet_count = 0;
                let payload = sample.data.to_vec();
                self.assembled_frame_count = self.assembled_frame_count.saturating_add(1);
                let inspection = match self.h264_inspector.inspect_access_unit(&payload) {
                    Ok(inspection) => inspection,
                    Err(error) => {
                        crate::xbx_log_error!(
                            "[RtcVideoFrameSource] h264 inspection failed: {error}"
                        );
                        self.waiting_for_recovery_keyframe = true;
                        self.queue_transport_observation(TransportObservation::Admission(
                            TransportAdmissionObservation::AwaitRecoveryKeyframe,
                        ));
                        continue;
                    }
                };
                match resolve_inspection_admission(&inspection) {
                    InspectionAdmission::Accept => {}
                    InspectionAdmission::AwaitRecoveryKeyframe => {
                        crate::xbx_log_warn!(
                            "[RtcVideoFrameSource] h264 inspection rejected sample ts={} bootstrap={:?} slice_headers_valid={}",
                            sample.packet_timestamp,
                            inspection.bootstrap_reject_reason,
                            inspection.slice_headers_valid
                        );
                        self.waiting_for_recovery_keyframe = true;
                        self.queue_transport_observation(TransportObservation::Admission(
                            TransportAdmissionObservation::AwaitRecoveryKeyframe,
                        ));
                        continue;
                    }
                }
                let is_keyframe = inspection.is_idr;
                let media_dropped_packets = sample
                    .prev_dropped_packets
                    .saturating_sub(sample.prev_padding_packets);
                if media_dropped_packets > 0 {
                    self.sample_loss_burst_count = self.sample_loss_burst_count.saturating_add(1);
                    self.clean_samples_since_loss = 0;
                } else if is_keyframe {
                    self.sample_loss_burst_count = 0;
                    self.clean_samples_since_loss = 0;
                } else if self.sample_loss_burst_count > 0 {
                    self.clean_samples_since_loss = self.clean_samples_since_loss.saturating_add(1);
                    if self.clean_samples_since_loss >= 4 {
                        self.sample_loss_burst_count = 0;
                        self.clean_samples_since_loss = 0;
                    }
                }
                let (next_waiting_for_recovery_keyframe, recovery_action) =
                    resolve_recovery_keyframe_action(
                        self.waiting_for_recovery_keyframe,
                        self.sample_loss_burst_count,
                        media_dropped_packets,
                        is_keyframe,
                    );
                self.waiting_for_recovery_keyframe = next_waiting_for_recovery_keyframe;

                if media_dropped_packets > 0 {
                    self.runtime_stats
                        .add_inbound_video_packet_loss_estimate(media_dropped_packets);
                    crate::xbx_log_warn!(
                        "[RtcVideoFrameSource] media loss detected before sample ts={} dropped_packets={} is_keyframe={}",
                        sample.packet_timestamp,
                        media_dropped_packets,
                        is_keyframe
                    );
                }

                let sample_loss_frame_importance = if is_keyframe {
                    "keyframe"
                } else if self.sample_loss_burst_count >= 2 {
                    "reference"
                } else {
                    "delta"
                };

                match recovery_action {
                    RecoveryKeyframeAction::Submit => {}
                    RecoveryKeyframeAction::DropAndRequestKeyframe => {
                        let nack_started = self
                            .observe_sample_loss_and_nack(
                                sample.packet_timestamp,
                                media_dropped_packets,
                                is_keyframe,
                                sample_loss_frame_importance,
                            )
                            .await;
                        if !nack_started {
                            self.queue_transport_observation(TransportObservation::Loss(
                                TransportLossObservation::PacketLossDetected,
                            ));
                        }
                        continue;
                    }
                    RecoveryKeyframeAction::TriggerWaitKeyframe => {
                        self.queue_transport_observation(TransportObservation::Loss(
                            TransportLossObservation::RecoveryKeyframeRequested,
                        ));
                        continue;
                    }
                    RecoveryKeyframeAction::WaitKeyframe => {
                        self.queue_transport_observation(TransportObservation::Loss(
                            TransportLossObservation::AwaitRecoveryKeyframe,
                        ));
                        continue;
                    }
                }

                let config_changed = inspection.config_changed;
                if let Some(width) = inspection.width {
                    self.current_width = width;
                }
                if let Some(height) = inspection.height {
                    self.current_height = height;
                }

                let frame_value = FrameValue::new(is_keyframe, config_changed, payload.len());
                self.last_submitted_frame_value = frame_value;
                let assembled_at = std::time::Instant::now();
                self.transport_deadline_tracker
                    .record_frame_arrival(now_ms_f64());
                if self.assembled_frame_count == 1 || self.assembled_frame_count.is_power_of_two() {
                    crate::xbx_log_info!(
                        "[RtcVideoFrameSource] assembled frame count={} ts={} len={} keyframe={} bootstrap={}",
                        self.assembled_frame_count,
                        sample.packet_timestamp,
                        payload.len(),
                        is_keyframe,
                        inspection.bootstrap_ready
                    );
                }

                crate::xbx_log_debug!(
                    "[Ingress] NALU Assb OK: size={}B, res={}x{}, is_kf={}, bootstrap={}",
                    payload.len(),
                    self.current_width,
                    self.current_height,
                    is_keyframe,
                    inspection.bootstrap_ready
                );

                return Some(AssembledVideoFrame {
                    codec: VideoCodec::H264,
                    is_keyframe,
                    config_changed,
                    value: frame_value,
                    width: self.current_width,
                    height: self.current_height,
                    rtp_timestamp: sample.packet_timestamp,
                    assembled_at,
                    h264: inspection,
                    payload: Bytes::from(payload),
                });
            }

            let now = std::time::Instant::now();
            let idle_timeout = now.duration_since(self.last_packet_time) > self.idle_timeout;
            let thin_stream_stall = self.should_trigger_thin_stream_stall(now);

            if idle_timeout || thin_stream_stall {
                self.sample_builder =
                    build_sample_builder(self.max_late_packets, self.jitter_buffer_max_delay);
                self.assembling_frame_start = None;
                self.current_assembly_packet_count = 0;
                self.last_packet_time = now;

                if self
                    .last_idle_hint_time
                    .map_or(true, |t| now.duration_since(t) >= self.idle_hint_cooldown)
                {
                    self.last_idle_hint_time = Some(now);
                    self.queue_transport_observation(if thin_stream_stall {
                        TransportObservation::StreamThinStall
                    } else {
                        TransportObservation::StreamIdleTimeout
                    });
                }
                continue;
            }

            // 当 RTX 已经命中首洞并排进 reinject queue 时，优先给主 reader 一个很短的直接出队窗口。
            // 否则外层固定 50ms timeout 很容易一直打断普通读路径，导致 queued 包迟迟走不到 deliveredPrimary。
            let read_timeout = if self.should_prioritize_reinject_drain() {
                std::time::Duration::from_millis(8)
            } else {
                std::time::Duration::from_millis(50)
            };
            if let Some(observation) = self
                .runtime_stats
                .read(|stats| stats.latest_video_rtx_reinject_observation.clone())
                .flatten()
            {
                if observation.stage == "queued" && observation.pending_queue_len > 0 {
                    self.reinject_read_poll_count = self.reinject_read_poll_count.saturating_add(1);
                    if self.reinject_read_poll_count == 1
                        || self.reinject_read_poll_count.is_power_of_two()
                    {
                        crate::xbx_log_warn!(
                            "[RtcVideoFrameSource] reinjectReadPoll pending={} gap={:?} nack={:?}..{:?} timeout_ms={} count={}",
                            observation.pending_queue_len,
                            observation.matched_gap_sequence,
                            observation.matched_nack_first_sequence,
                            observation.matched_nack_last_sequence,
                            read_timeout.as_millis(),
                            self.reinject_read_poll_count
                        );
                    }
                }
            }
            match tokio::time::timeout(read_timeout, self.rx.recv()).await {
                Ok(Some(rtp_video_packet)) => {
                    self.received_packet_count = self.received_packet_count.saturating_add(1);
                    let rtp = rtp_video_packet.to_rtp_packet();
                    self.last_packet_time = std::time::Instant::now();
                    if self.assembling_frame_start.is_none() {
                        self.assembling_frame_start = Some(self.last_packet_time);
                        self.current_assembly_packet_count = 0;
                    }
                    self.current_assembly_packet_count =
                        self.current_assembly_packet_count.saturating_add(1);
                    let seq = rtp.header.sequence_number;
                    let now_ms = now_ms_f64();
                    let latest_reinject_observation = self
                        .runtime_stats
                        .read(|stats| stats.latest_video_rtx_reinject_observation.clone())
                        .flatten();
                    if let Some(observation) = latest_reinject_observation.clone() {
                        if observation.stage == "deliveredPrimary"
                            && observation.sequence_number == seq
                        {
                            self.runtime_stats.record_video_rtx_reinject(
                                XbxEngineVideoRtxReinjectObservation {
                                    stage: "adapterRead".to_string(),
                                    primary_ssrc: observation.primary_ssrc,
                                    repair_ssrc: observation.repair_ssrc,
                                    sequence_number: observation.sequence_number,
                                    rtp_timestamp: observation.rtp_timestamp,
                                    pending_queue_len: observation.pending_queue_len,
                                    native_sequence_number: observation.native_sequence_number,
                                    matched_head_gap: observation.matched_head_gap,
                                    matched_nack_range: observation.matched_nack_range,
                                    matched_pending_gap: observation.matched_pending_gap,
                                    matched_gap_sequence: observation.matched_gap_sequence,
                                    matched_nack_first_sequence: observation
                                        .matched_nack_first_sequence,
                                    matched_nack_last_sequence: observation
                                        .matched_nack_last_sequence,
                                    observed_at_ms: now_ms,
                                },
                            );
                        }
                    }
                    let (next_highest_sequence, forward_gap) =
                        detect_forward_gap(self.last_highest_rtp_sequence, seq);
                    self.last_highest_rtp_sequence = next_highest_sequence;
                    if let Some((expected_sequence, received_sequence)) = forward_gap {
                        self.observe_forward_gap_and_nack(expected_sequence, received_sequence)
                            .await;
                    }
                    self.nack_window.add(seq);
                    self.push_recent_rtp_packet(seq, rtp.header.timestamp);
                    if let Some(observation) = latest_reinject_observation.clone() {
                        if observation.stage == "adapterRead" && observation.sequence_number == seq
                        {
                            self.runtime_stats.record_video_rtx_reinject(
                                XbxEngineVideoRtxReinjectObservation {
                                    stage: "sampleBuilderPush".to_string(),
                                    primary_ssrc: observation.primary_ssrc,
                                    repair_ssrc: observation.repair_ssrc,
                                    sequence_number: observation.sequence_number,
                                    rtp_timestamp: observation.rtp_timestamp,
                                    pending_queue_len: observation.pending_queue_len,
                                    native_sequence_number: observation.native_sequence_number,
                                    matched_head_gap: observation.matched_head_gap,
                                    matched_nack_range: observation.matched_nack_range,
                                    matched_pending_gap: observation.matched_pending_gap,
                                    matched_gap_sequence: observation.matched_gap_sequence,
                                    matched_nack_first_sequence: observation
                                        .matched_nack_first_sequence,
                                    matched_nack_last_sequence: observation
                                        .matched_nack_last_sequence,
                                    observed_at_ms: now_ms,
                                },
                            );
                        }
                    }
                    if let Some(resolved) = self.nack_scheduler.resolve_sequence(seq, now_ms) {
                        if let Some(observation) = latest_reinject_observation.clone() {
                            if observation.sequence_number == seq {
                                self.runtime_stats.record_video_rtx_reinject(
                                    XbxEngineVideoRtxReinjectObservation {
                                        stage: "adapterResolved".to_string(),
                                        primary_ssrc: observation.primary_ssrc,
                                        repair_ssrc: observation.repair_ssrc,
                                        sequence_number: observation.sequence_number,
                                        rtp_timestamp: observation.rtp_timestamp,
                                        pending_queue_len: observation.pending_queue_len,
                                        native_sequence_number: observation.native_sequence_number,
                                        matched_head_gap: observation.matched_head_gap,
                                        matched_nack_range: observation.matched_nack_range,
                                        matched_pending_gap: observation.matched_pending_gap,
                                        matched_gap_sequence: observation.matched_gap_sequence,
                                        matched_nack_first_sequence: observation
                                            .matched_nack_first_sequence,
                                        matched_nack_last_sequence: observation
                                            .matched_nack_last_sequence,
                                        observed_at_ms: now_ms,
                                    },
                                );
                            }
                        }
                        self.record_nack_recovered(resolved, now_ms);
                    } else if let Some(observation) = latest_reinject_observation {
                        if observation.stage == "adapterRead" && observation.sequence_number == seq
                        {
                            self.runtime_stats.record_video_rtx_reinject(
                                XbxEngineVideoRtxReinjectObservation {
                                    stage: "adapterResolveMiss".to_string(),
                                    primary_ssrc: observation.primary_ssrc,
                                    repair_ssrc: observation.repair_ssrc,
                                    sequence_number: observation.sequence_number,
                                    rtp_timestamp: observation.rtp_timestamp,
                                    pending_queue_len: observation.pending_queue_len,
                                    native_sequence_number: observation.native_sequence_number,
                                    matched_head_gap: observation.matched_head_gap,
                                    matched_nack_range: observation.matched_nack_range,
                                    matched_pending_gap: observation.matched_pending_gap,
                                    matched_gap_sequence: observation.matched_gap_sequence,
                                    matched_nack_first_sequence: observation
                                        .matched_nack_first_sequence,
                                    matched_nack_last_sequence: observation
                                        .matched_nack_last_sequence,
                                    observed_at_ms: now_ms,
                                },
                            );
                        }
                    }
                    if seq % 100 == 0 {
                        crate::xbx_log_info!(
                            "[RtcVideoFrameSource] RTP packet received: seq={}, ts={}",
                            seq,
                            rtp.header.timestamp
                        );
                    }
                    if self.received_packet_count == 1
                        || self.received_packet_count.is_power_of_two()
                    {
                        crate::xbx_log_info!(
                            "[RtcVideoFrameSource] packet received count={} seq={} ts={}",
                            self.received_packet_count,
                            seq,
                            rtp.header.timestamp
                        );
                    }
                    self.sample_builder.push(rtp);
                }
                Ok(None) => {
                    crate::xbx_log_error!("[RtcVideoFrameSource] rx closed");
                    return None;
                }
                Err(_) => {}
            }
        }
    }
}

impl FrameSource for RtcVideoFrameSource {
    fn recv_frame<'a>(
        &'a mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<AssembledVideoFrame>> + Send + 'a>>
    {
        Box::pin(async move { self.recv_frame_inner().await })
    }
}

impl TransportObservationSource for RtcVideoTransportObservationSource {
    fn recv_transport_observation<'a>(
        &'a mut self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Option<TransportObservation>> + Send + 'a>,
    > {
        Box::pin(async move { self.rx.recv().await })
    }
}
