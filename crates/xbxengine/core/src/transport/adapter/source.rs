use crate::media::video::h264::inspection::H264AccessUnitInspection;
use crate::transport::webrtc::recovery::recovery_signal::VideoRecoverySignal;

use super::{
    build_sample_builder, now_ms_f64, resolve_playout_delay, Bytes, EncodedFrame, FrameSource,
    FrameSourceEvent, FrameValue, VideoCodec, WebrtcVideoAdapter, UINT16SIZE_HALF,
};

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
    Recover(VideoRecoverySignal),
}

pub(super) fn resolve_inspection_admission(
    inspection: &H264AccessUnitInspection,
) -> InspectionAdmission {
    if !inspection.slice_headers_valid {
        return InspectionAdmission::Recover(VideoRecoverySignal::TransportAwaitRecoveryKeyframe);
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

impl WebrtcVideoAdapter {
    fn should_trigger_thin_stream_stall(&self, now: std::time::Instant) -> bool {
        self.assembling_frame_start.is_some_and(|started_at| {
            now.duration_since(started_at) >= self.assembly_stall_timeout
                && self.current_assembly_packet_count > 0
                && self.current_assembly_packet_count <= self.thin_stream_packet_threshold
        })
    }
}

impl FrameSource for WebrtcVideoAdapter {
    fn recv_frame<'a>(
        &'a mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<FrameSourceEvent>> + Send + 'a>>
    {
        Box::pin(async {
            loop {
                self.maybe_run_nack_maintenance().await;
                if let Some(signal) = self.pending_recovery_signal.take() {
                    return Some(FrameSourceEvent::RecoverySignal(signal));
                }
                if let Some(sample) = self.sample_builder.pop() {
                    self.last_packet_time = std::time::Instant::now();
                    self.assembling_frame_start = None;
                    self.current_assembly_packet_count = 0;
                    let payload = sample.data.to_vec();
                    let inspection = match self.h264_inspector.inspect_access_unit(&payload) {
                        Ok(inspection) => inspection,
                        Err(error) => {
                            crate::xbx_log_error!(
                                "[WebrtcVideoAdapter] h264 inspection failed: {error}"
                            );
                            self.waiting_for_recovery_keyframe = true;
                            self.queue_recovery_signal(
                                VideoRecoverySignal::TransportAwaitRecoveryKeyframe,
                            );
                            continue;
                        }
                    };
                    match resolve_inspection_admission(&inspection) {
                        InspectionAdmission::Accept => {}
                        InspectionAdmission::Recover(signal) => {
                            crate::xbx_log_warn!(
                                "[WebrtcVideoAdapter] h264 inspection rejected sample ts={} bootstrap={:?} slice_headers_valid={}",
                                sample.packet_timestamp,
                                inspection.bootstrap_reject_reason,
                                inspection.slice_headers_valid
                            );
                            self.waiting_for_recovery_keyframe = true;
                            self.queue_recovery_signal(signal);
                            continue;
                        }
                    }
                    let is_keyframe = inspection.is_idr;
                    let media_dropped_packets = sample
                        .prev_dropped_packets
                        .saturating_sub(sample.prev_padding_packets);
                    if media_dropped_packets > 0 {
                        self.sample_loss_burst_count =
                            self.sample_loss_burst_count.saturating_add(1);
                        self.clean_samples_since_loss = 0;
                    } else if is_keyframe {
                        self.sample_loss_burst_count = 0;
                        self.clean_samples_since_loss = 0;
                    } else if self.sample_loss_burst_count > 0 {
                        self.clean_samples_since_loss =
                            self.clean_samples_since_loss.saturating_add(1);
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
                        if let Ok(mut stats) = self.runtime_stats.lock() {
                            stats.inbound_video_packet_loss_estimate_total = stats
                                .inbound_video_packet_loss_estimate_total
                                .saturating_add(u64::from(media_dropped_packets));
                        }
                        crate::xbx_log_warn!(
                            "[WebrtcVideoAdapter] media loss detected before sample ts={} dropped_packets={} is_keyframe={}",
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
                                self.queue_recovery_signal(
                                    VideoRecoverySignal::TransportSampleLoss,
                                );
                            }
                            continue;
                        }
                        RecoveryKeyframeAction::TriggerWaitKeyframe => {
                            self.queue_recovery_signal(
                                VideoRecoverySignal::TransportSampleLossBurst,
                            );
                            continue;
                        }
                        RecoveryKeyframeAction::WaitKeyframe => {
                            self.queue_recovery_signal(
                                VideoRecoverySignal::TransportAwaitRecoveryKeyframe,
                            );
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
                    let playout_delay = resolve_playout_delay(
                        frame_value,
                        self.jitter_buffer_min_delay,
                        self.jitter_buffer_max_delay,
                    );
                    let target_playout_at_ms = now_ms_f64() + playout_delay.as_millis() as f64;
                    self.frame_deadline_tracker
                        .record_frame_target(target_playout_at_ms);

                    crate::xbx_log_debug!(
                        "[Ingress] NALU Assb OK: size={}B, res={}x{}, is_kf={}, bootstrap={}",
                        payload.len(),
                        self.current_width,
                        self.current_height,
                        is_keyframe,
                        inspection.bootstrap_ready
                    );

                    return Some(FrameSourceEvent::Frame(EncodedFrame {
                        codec: VideoCodec::H264,
                        is_keyframe,
                        config_changed,
                        value: frame_value,
                        width: self.current_width,
                        height: self.current_height,
                        rtp_timestamp: sample.packet_timestamp,
                        assembled_at: std::time::Instant::now(),
                        target_playout_time: std::time::Instant::now() + playout_delay,
                        h264: inspection,
                        payload: Bytes::from(payload),
                    }));
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
                        return Some(FrameSourceEvent::RecoverySignal(if thin_stream_stall {
                            VideoRecoverySignal::AdapterThinStream
                        } else {
                            VideoRecoverySignal::AdapterIdleTimeout
                        }));
                    }
                    continue;
                }

                let wait_duration = std::time::Duration::from_millis(50);
                match tokio::time::timeout(wait_duration, self.track.read_rtp()).await {
                    Ok(Ok((rtp, _))) => {
                        self.last_packet_time = std::time::Instant::now();
                        if self.assembling_frame_start.is_none() {
                            self.assembling_frame_start = Some(self.last_packet_time);
                            self.current_assembly_packet_count = 0;
                        }
                        self.current_assembly_packet_count =
                            self.current_assembly_packet_count.saturating_add(1);
                        let seq = rtp.header.sequence_number;
                        let now_ms = now_ms_f64();
                        let (next_highest_sequence, forward_gap) =
                            detect_forward_gap(self.last_highest_rtp_sequence, seq);
                        self.last_highest_rtp_sequence = next_highest_sequence;
                        if let Some((expected_sequence, received_sequence)) = forward_gap {
                            self.observe_forward_gap_and_nack(expected_sequence, received_sequence)
                                .await;
                        }
                        self.nack_window.add(seq);
                        self.push_recent_rtp_packet(seq, rtp.header.timestamp);
                        if let Some(resolved) = self.nack_scheduler.resolve_sequence(seq, now_ms) {
                            self.record_nack_recovered(resolved, now_ms);
                        }
                        if seq % 100 == 0 {
                            crate::xbx_log_info!(
                                "[WebrtcVideoAdapter] RTP packet received: seq={}, ts={}",
                                seq,
                                rtp.header.timestamp
                            );
                        }
                        self.sample_builder.push(rtp);
                    }
                    Ok(Err(e)) => {
                        if !e.to_string().contains("io: EOF") {
                            crate::xbx_log_error!("[WebrtcVideoAdapter] track read error: {}", e);
                        }
                        return None;
                    }
                    Err(_) => {}
                }
            }
        })
    }
}
