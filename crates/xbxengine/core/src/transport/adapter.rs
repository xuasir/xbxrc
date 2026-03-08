use bytes::Bytes;
use rtp::codecs::h264::H264Packet;
use std::sync::Arc;

use webrtc::track::track_remote::TrackRemote;
use webrtc_media::io::sample_builder::SampleBuilder;

use crate::media::video::types::{EncodedFrame, VideoCodec};
use crate::transport::h264_resolution::parse_sps_dimensions_from_nal;

pub enum FrameSourceEvent {
    Frame(EncodedFrame),
    RequestKeyframe(String), // The reason for requesting a keyframe
}

pub trait FrameSource: Send {
    fn recv_frame<'a>(
        &'a mut self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Option<FrameSourceEvent>> + Send + 'a>,
    >;
}

pub struct WebrtcVideoAdapter {
    track: Arc<TrackRemote>,
    sample_builder: SampleBuilder<H264Packet>,
    max_late_packets: u16,
    idle_timeout: std::time::Duration,
    last_packet_time: std::time::Instant,
    assembling_frame_start: Option<std::time::Instant>,
    last_keyframe_request_time: Option<std::time::Instant>,
    current_width: u32,
    current_height: u32,
}

impl WebrtcVideoAdapter {
    pub fn new(track: Arc<TrackRemote>, max_late_packets: u16, idle_timeout: std::time::Duration) -> Self {
        Self {
            track,
            sample_builder: SampleBuilder::new(max_late_packets, H264Packet::default(), 90_000),
            max_late_packets,
            idle_timeout,
            last_packet_time: std::time::Instant::now(),
            assembling_frame_start: None,
            last_keyframe_request_time: None,
            current_width: 0,
            current_height: 0,
        }
    }
}

fn parse_idr_and_sps(payload: &[u8]) -> (bool, Option<(u32, u32)>) {
    let mut is_keyframe = false;
    let mut resolution = None;
    let mut i = 0;
    while i + 3 < payload.len() {
        let start_len = if payload[i] == 0 && payload[i + 1] == 0 && payload[i + 2] == 1 {
            3
        } else if i + 4 < payload.len()
            && payload[i] == 0
            && payload[i + 1] == 0
            && payload[i + 2] == 0
            && payload[i + 3] == 1
        {
            4
        } else {
            i += 1;
            continue;
        };

        if i + start_len >= payload.len() {
            break;
        }

        let nal_type = payload[i + start_len] & 0x1f;

        let mut nal_end = payload.len();
        let mut j = i + start_len;
        while j + 3 < payload.len() {
            if (payload[j] == 0 && payload[j + 1] == 0 && payload[j + 2] == 1)
                || (j + 4 < payload.len()
                    && payload[j] == 0
                    && payload[j + 1] == 0
                    && payload[j + 2] == 0
                    && payload[j + 3] == 1)
            {
                nal_end = j;
                break;
            }
            j += 1;
        }

        let nal = &payload[i + start_len..nal_end];
        if nal_type == 5 {
            is_keyframe = true;
        } else if nal_type == 7 {
            resolution = parse_sps_dimensions_from_nal(nal);
        }

        i = nal_end;
    }
    (is_keyframe, resolution)
}

impl FrameSource for WebrtcVideoAdapter {
    fn recv_frame<'a>(
        &'a mut self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Option<FrameSourceEvent>> + Send + 'a>,
    > {
        Box::pin(async {
            loop {
                if let Some(sample) = self.sample_builder.pop() {
                    self.last_packet_time = std::time::Instant::now();
                    self.assembling_frame_start = None;
                    let payload = sample.data.to_vec();
                    // extract the actual sequence numbers from the sample if needed,
                    // but since sample builder popped it, it means it's complete.
                    let (is_keyframe, maybe_res) = parse_idr_and_sps(&payload);

                    let mut config_changed = false;
                    if let Some((w, h)) = maybe_res {
                        if w != self.current_width || h != self.current_height {
                            self.current_width = w;
                            self.current_height = h;
                            config_changed = true;
                        }
                    }

                    let playout_delay = std::time::Duration::from_millis(30);

                    // --- 网络层健康度指标：有效 NALU 大小与分辨率 ---
                    crate::xbx_log_debug!(
                        "[Ingress] NALU Assb OK: size={}B, res={}x{}, is_kf={}",
                        payload.len(),
                        self.current_width,
                        self.current_height,
                        is_keyframe
                    );

                    return Some(FrameSourceEvent::Frame(EncodedFrame {
                        codec: VideoCodec::H264,
                        is_keyframe,
                        config_changed,
                        width: self.current_width,
                        height: self.current_height,
                        rtp_timestamp: sample.packet_timestamp,
                        assembled_at: std::time::Instant::now(),
                        target_playout_time: std::time::Instant::now() + playout_delay,
                        payload: Bytes::from(payload),
                    }));
                }

                let now = std::time::Instant::now();
                // 仅在网络真正停止传包时熔断
                // 注意：不使用帧装配超时（assembly_timeout），因为大 IDR 帧在 15 Mbps
                // 下可能需要 80-200ms 才能完整传输，短超时会反复打断装配形成死循环
                let idle_timeout = now.duration_since(self.last_packet_time) > self.idle_timeout;

                if idle_timeout {
                    let reason = format!("Network idle timeout (>{:?})", self.idle_timeout);
                    self.sample_builder = SampleBuilder::new(self.max_late_packets, H264Packet::default(), 90_000);
                    self.assembling_frame_start = None;
                    self.last_packet_time = now;
                    
                    if self.last_keyframe_request_time.map_or(true, |t| now.duration_since(t) > std::time::Duration::from_millis(500)) {
                        self.last_keyframe_request_time = Some(now);
                        return Some(FrameSourceEvent::RequestKeyframe(reason));
                    }
                    continue;
                }

                // 每次最多等 50ms，避免 pop 检查滞后太久
                let wait_duration = std::time::Duration::from_millis(50);
                match tokio::time::timeout(wait_duration, self.track.read_rtp()).await {
                    Ok(Ok((rtp, _))) => {
                        self.last_packet_time = std::time::Instant::now();
                        if self.assembling_frame_start.is_none() {
                            self.assembling_frame_start = Some(self.last_packet_time);
                        }
                        let seq = rtp.header.sequence_number;
                        if seq % 100 == 0 {
                            crate::xbx_log_info!("[WebrtcVideoAdapter] RTP packet received: seq={}, ts={}", seq, rtp.header.timestamp);
                        }
                        self.sample_builder.push(rtp);
                    }
                    Ok(Err(e)) => {
                        if !e.to_string().contains("io: EOF") {
                            crate::xbx_log_error!("[WebrtcVideoAdapter] track read error: {}", e);
                        }
                        return None;
                    }
                    Err(_) => {
                        // Timeout hit: the start of the next loop will trigger `assembly_timeout` or `idle_timeout` and reset.
                    }
                }
            }
        })
    }
}
