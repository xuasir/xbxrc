use std::sync::Arc;
use std::time::Duration;
use std::{collections::VecDeque, env};

use openh264::{decoder::Decoder, formats::YUVSource};
use rtp::{codecs::h264::H264Packet, packet::Packet};
use webrtc_media::io::sample_builder::SampleBuilder;

use crate::{webrtc_rs_render::WebRtcRsRenderFrame, XbxEngineRuntimeError};

const VIDEO_JITTER_BUFFER_MIN_MS_ENV: &str = "XBXENGINE_VIDEO_JITTER_BUFFER_MIN_MS";
const VIDEO_JITTER_BUFFER_MAX_MS_ENV: &str = "XBXENGINE_VIDEO_JITTER_BUFFER_MAX_MS";
const VIDEO_JITTER_BUFFER_MAX_LATE_PACKETS_ENV: &str = "XBXENGINE_VIDEO_JITTER_BUFFER_MAX_LATE_PACKETS";
const DEFAULT_VIDEO_JITTER_BUFFER_MIN_MS: u64 = 20;
const DEFAULT_VIDEO_JITTER_BUFFER_MAX_MS: u64 = 30;
const DEFAULT_VIDEO_JITTER_BUFFER_MAX_LATE_PACKETS: u16 = 512;

#[derive(Clone, Copy, Debug)]
struct VideoJitterBufferConfig {
    min_delay_ms: u64,
    max_delay_ms: u64,
    max_late_packets: u16,
}

impl VideoJitterBufferConfig {
    fn from_env() -> Self {
        let min_delay_ms = parse_env_u64(VIDEO_JITTER_BUFFER_MIN_MS_ENV)
            .unwrap_or(DEFAULT_VIDEO_JITTER_BUFFER_MIN_MS);
        let max_delay_ms = parse_env_u64(VIDEO_JITTER_BUFFER_MAX_MS_ENV)
            .unwrap_or(DEFAULT_VIDEO_JITTER_BUFFER_MAX_MS);
        let max_late_packets = parse_env_u16(VIDEO_JITTER_BUFFER_MAX_LATE_PACKETS_ENV)
            .unwrap_or(DEFAULT_VIDEO_JITTER_BUFFER_MAX_LATE_PACKETS);
        let bounded_min = min_delay_ms.min(max_delay_ms.max(1));
        let bounded_max = max_delay_ms.max(bounded_min).max(1);
        let bounded_late_packets = max_late_packets.max(1);
        Self {
            min_delay_ms: bounded_min,
            max_delay_ms: bounded_max,
            max_late_packets: bounded_late_packets,
        }
    }
}

#[derive(Debug)]
struct QueuedDecodedFrame {
    queued_at_ms: f64,
    frame: WebRtcRsRenderFrame,
}

pub(crate) struct WebRtcRsVideoDecodeState {
    sample_builder: SampleBuilder<H264Packet>,
    decoder: Decoder,
    latest_decoded_seq: u64,
    first_video_packet_logged: bool,
    jitter_buffer_min_delay_ms: f64,
    jitter_buffer_max_delay_ms: f64,
    decoded_frame_queue: VecDeque<QueuedDecodedFrame>,
}

impl WebRtcRsVideoDecodeState {
    pub(crate) fn new() -> Result<Self, XbxEngineRuntimeError> {
        let jitter_buffer = VideoJitterBufferConfig::from_env();
        Ok(Self {
            sample_builder: SampleBuilder::new(
                jitter_buffer.max_late_packets,
                H264Packet::default(),
                90_000,
            )
            .with_max_time_delay(Duration::from_millis(jitter_buffer.max_delay_ms)),
            decoder: Decoder::new().map_err(map_openh264_error("createOpenH264DecoderFailed"))?,
            latest_decoded_seq: 0,
            first_video_packet_logged: false,
            jitter_buffer_min_delay_ms: jitter_buffer.min_delay_ms as f64,
            jitter_buffer_max_delay_ms: jitter_buffer.max_delay_ms as f64,
            decoded_frame_queue: VecDeque::new(),
        })
    }

    pub(crate) fn push_packet(&mut self, packet: Packet) {
        if !self.first_video_packet_logged {
            self.first_video_packet_logged = true;
            eprintln!(
                "[xbxengine][webrtc-rs] first video frame received seq={} ts={} marker={} payload={}",
                packet.header.sequence_number,
                packet.header.timestamp,
                packet.header.marker,
                packet.payload.len()
            );
        }

        self.sample_builder.push(packet);
    }

    pub(crate) fn pop_decoded_frame(&mut self, now_ms: f64) -> Option<WebRtcRsRenderFrame> {
        // 先把当前可解码 sample 全部转成帧并入队，再按抖动缓冲窗口出队。
        while let Some(sample) = self.sample_builder.pop() {
            let yuv = match self.decoder.decode(sample.data.as_ref()) {
                Ok(Some(yuv)) => yuv,
                _ => continue,
            };

            let (width, height) = yuv.dimensions();
            let mut rgba_bytes = vec![0; width * height * 4];
            yuv.write_rgba8(&mut rgba_bytes);

            self.latest_decoded_seq = self.latest_decoded_seq.saturating_add(1);
            self.decoded_frame_queue.push_back(QueuedDecodedFrame {
                queued_at_ms: now_ms,
                frame: WebRtcRsRenderFrame {
                    width: width as u32,
                    height: height as u32,
                    frame_seq: self.latest_decoded_seq,
                    rendered_at_ms: now_ms,
                    rgba_bytes: Arc::from(rgba_bytes),
                },
            });
        }

        let queued = self.decoded_frame_queue.front()?;
        let queue_delay_ms = (now_ms - queued.queued_at_ms).max(0.0);
        let should_release = queue_delay_ms >= self.jitter_buffer_max_delay_ms
            || (queue_delay_ms >= self.jitter_buffer_min_delay_ms
                && self.decoded_frame_queue.len() > 1);
        if !should_release {
            return None;
        }
        self.decoded_frame_queue.pop_front().map(|item| item.frame)
    }
}

fn parse_env_u64(key: &str) -> Option<u64> {
    env::var(key).ok()?.trim().parse::<u64>().ok()
}

fn parse_env_u16(key: &str) -> Option<u16> {
    env::var(key).ok()?.trim().parse::<u16>().ok()
}

fn map_openh264_error(
    code: impl Into<String>,
) -> impl FnOnce(openh264::Error) -> XbxEngineRuntimeError {
    let code = code.into();
    move |error| XbxEngineRuntimeError::new(format!("{code}:{error}"))
}
