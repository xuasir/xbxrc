use std::sync::{Arc, Mutex};
use tokio::{runtime::Handle, sync::watch, task::JoinHandle as TokioJoinHandle};

use opus::{Channels as OpusChannels, Decoder as OpusDecoder};

use crate::{
    runtime_stats_sink::RuntimeStatsSink, transport::rtc::stream::packet_types::RtcAudioRtpPacket,
    XbxEngineMediaRuntimeStats, XbxEngineVideoTrackStatus,
};

use super::{
    state::AudioPlaybackSharedState, MAX_OPUS_FRAME_SAMPLES_PER_CHANNEL, OPUS_OUTPUT_CHANNELS,
    OPUS_SAMPLE_RATE_HZ,
};

pub(super) fn spawn_audio_decode_task(
    runtime: &Handle,
    mut rx: tokio::sync::mpsc::Receiver<RtcAudioRtpPacket>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    shared_state: Arc<Mutex<AudioPlaybackSharedState>>,
    mut stop_receiver: watch::Receiver<bool>,
) -> TokioJoinHandle<()> {
    let runtime_stats = RuntimeStatsSink::new(runtime_stats);
    runtime.spawn(async move {
        let mut decoder = match OpusDecoder::new(OPUS_SAMPLE_RATE_HZ, OpusChannels::Stereo) {
            Ok(decoder) => decoder,
            Err(error) => {
                crate::xbx_log_error!("[xbxengine][rtc][audio] opus decoder init failed: {error}");
                return;
            }
        };
        let mut decoded_pcm =
            vec![0.0f32; MAX_OPUS_FRAME_SAMPLES_PER_CHANNEL * OPUS_OUTPUT_CHANNELS];
        let mut total_audio_bytes = 0u64;
        let mut last_audio_sample_bytes = 0u64;
        let mut last_audio_sample_at_ms = now_ms_f64();

        loop {
            tokio::select! {
                _ = stop_receiver.changed() => {
                    break;
                }
                next_packet = rx.recv() => {
                    let Some(rtp) = next_packet else {
                        break;
                    };
                    total_audio_bytes = total_audio_bytes.saturating_add(rtp.payload.len() as u64);
                    update_audio_runtime_stats(
                        &runtime_stats,
                        total_audio_bytes,
                        &mut last_audio_sample_bytes,
                        &mut last_audio_sample_at_ms,
                    );

                    if rtp.payload.is_empty() {
                        continue;
                    }

                    match decoder.decode_float(&rtp.payload, &mut decoded_pcm, false) {
                        Ok(decoded_samples_per_channel) => {
                            let decoded_len =
                                decoded_samples_per_channel.saturating_mul(OPUS_OUTPUT_CHANNELS);
                            if let Ok(mut state) = shared_state.lock() {
                                state.enqueue_interleaved_stereo(&decoded_pcm[..decoded_len]);
                            }
                        }
                        Err(error) => {
                            crate::xbx_log_warn!(
                                "[xbxengine][rtc][audio] opus decode failed: {error}"
                            );
                        }
                    }
                }
            }
        }
    })
}

fn update_audio_runtime_stats(
    runtime_stats: &RuntimeStatsSink,
    total_audio_bytes: u64,
    last_audio_sample_bytes: &mut u64,
    last_audio_sample_at_ms: &mut f64,
) {
    let now_ms = now_ms_f64();
    let elapsed_ms = (now_ms - *last_audio_sample_at_ms).max(0.0);
    runtime_stats.update(|shared| {
        shared.inbound_audio_bytes_total = total_audio_bytes;
        if shared.first_audio_packet_arrival_time_ms.is_none() {
            shared.first_audio_packet_arrival_time_ms = Some(now_ms);
        }
        shared.latest_audio_packet_arrival_time_ms = Some(now_ms);
        if elapsed_ms >= 250.0 {
            let delta_bytes = total_audio_bytes.saturating_sub(*last_audio_sample_bytes);
            let audio_kbps = (delta_bytes * 8) as f64 / elapsed_ms.max(1.0);
            shared.inbound_audio_bitrate_kbps = Some(audio_kbps.max(0.0));
            shared.inbound_bitrate_kbps =
                Some(shared.inbound_video_bitrate_kbps.unwrap_or(0.0) + audio_kbps.max(0.0));
            *last_audio_sample_bytes = total_audio_bytes;
            *last_audio_sample_at_ms = now_ms;
        }
        shared.inbound_bytes_total =
            shared.inbound_video_bytes_total + shared.inbound_audio_bytes_total;
        if shared.latest_video_track_status.is_none()
            && shared.inbound_audio_bytes_total > 0
            && shared.inbound_video_bytes_total == 0
        {
            // 没有视频时也要同步 audioOnly，避免上层长期看不到音频态。
            shared.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
                state: "audioOnly".to_string(),
                video_width: None,
                video_height: None,
                mime_type: None,
                transport_state: shared.transport_state.clone(),
                video_bytes_total: 0,
                video_packet_count_total: 0,
                audio_bytes_total: shared.inbound_audio_bytes_total,
                observed_at_ms: now_ms,
            });
        }
    });
}

fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}
