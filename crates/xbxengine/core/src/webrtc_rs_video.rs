use std::sync::{Arc, Mutex};

use rtp::packet::Packet;

use crate::webrtc_rs_render::WebRtcRsRenderState;
pub(crate) use crate::webrtc_rs_video_decode::WebRtcRsVideoDecodeState;
use crate::XbxEngineMediaRuntimeStats;

pub(crate) fn process_remote_video_packet(
    video_decode_state: &Arc<Mutex<WebRtcRsVideoDecodeState>>,
    render_state: &Arc<Mutex<WebRtcRsRenderState>>,
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    packet: Packet,
    now_ms: f64,
    frame_seq: &mut u64,
) {
    let mut decode_state = match video_decode_state.lock() {
        Ok(state) => state,
        Err(_) => return,
    };
    decode_state.push_packet(packet);

    while let Some(render_frame) = decode_state.pop_decoded_frame(now_ms) {
        let rgba_byte_len = render_frame.rgba_bytes.len();
        let frame_stats = match render_state.lock() {
            Ok(mut render_state) => match render_state.present_frame(render_frame) {
                Ok(frame_stats) => frame_stats,
                Err(error) => {
                    eprintln!("[xbxengine][webrtc-rs] render present failed: {error}");
                    continue;
                }
            },
            Err(_) => continue,
        };

        *frame_seq = frame_stats.frame_seq;

        if let Ok(mut stats) = runtime_stats.lock() {
            stats.latest_video_frame = Some(frame_stats.clone());
        }

        if frame_stats.frame_seq == 1 {
            eprintln!(
                "[xbxengine][webrtc-rs] first decoded video frame width={} height={} rgba_bytes={}",
                frame_stats.width, frame_stats.height, rgba_byte_len
            );
        } else if frame_stats.frame_seq % 30 == 0 {
            eprintln!(
                "[xbxengine][webrtc-rs] decoded video frames frames={} width={} height={} rgba_bytes={}",
                frame_stats.frame_seq,
                frame_stats.width,
                frame_stats.height,
                rgba_byte_len
            );
        }
    }
}
