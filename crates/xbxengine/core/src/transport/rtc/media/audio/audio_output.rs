use std::{
    collections::VecDeque,
    sync::mpsc as std_mpsc,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, SampleFormat, SampleRate, Stream, SupportedStreamConfig,
};
use opus::{Channels as OpusChannels, Decoder as OpusDecoder};
use tokio::{runtime::Handle, sync::watch, task::JoinHandle as TokioJoinHandle};

use crate::{
    runtime_stats_sink::RuntimeStatsSink, transport::rtc::media::packet_types::RtcAudioRtpPacket,
    XbxEngineMediaRuntimeStats, XbxEngineRuntimeError, XbxEngineVideoTrackStatus,
};

const OPUS_SAMPLE_RATE_HZ: u32 = 48_000;
const OPUS_OUTPUT_CHANNELS: usize = 2;
const MAX_OPUS_FRAME_SAMPLES_PER_CHANNEL: usize = 5_760;
const MAX_BUFFERED_AUDIO_FRAMES: usize = OPUS_SAMPLE_RATE_HZ as usize;

pub(crate) struct XbxRemoteAudioPlaybackSession {
    output_stop_sender: std_mpsc::Sender<()>,
    output_thread: thread::JoinHandle<()>,
    decode_task: TokioJoinHandle<()>,
    stop_signal: watch::Sender<bool>,
}

impl XbxRemoteAudioPlaybackSession {
    pub(crate) fn start(
        runtime: &Handle,
        rx: tokio::sync::mpsc::Receiver<RtcAudioRtpPacket>,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        volume_bits: Arc<AtomicU32>,
    ) -> Result<Self, XbxEngineRuntimeError> {
        let shared_state = Arc::new(Mutex::new(AudioPlaybackSharedState::default()));
        let (output_stop_sender, output_stop_receiver) = std_mpsc::channel::<()>();
        let (startup_sender, startup_receiver) = std_mpsc::sync_channel::<Result<(), String>>(1);
        let output_thread = spawn_audio_output_thread(
            shared_state.clone(),
            volume_bits,
            startup_sender,
            output_stop_receiver,
        );
        startup_receiver
            .recv()
            .map_err(|_| XbxEngineRuntimeError::new("remoteAudioOutputStartupChannelClosed"))?
            .map_err(XbxEngineRuntimeError::new)?;

        let (stop_signal, stop_receiver) = watch::channel(false);
        let decode_task =
            spawn_audio_decode_task(runtime, rx, runtime_stats, shared_state, stop_receiver);

        Ok(Self {
            output_stop_sender,
            output_thread,
            decode_task,
            stop_signal,
        })
    }

    pub(crate) fn stop(self) {
        let _ = self.stop_signal.send(true);
        let _ = self.output_stop_sender.send(());
        self.decode_task.abort();
        let _ = self.output_thread.join();
    }
}

#[derive(Default)]
struct AudioPlaybackSharedState {
    frames: VecDeque<[f32; OPUS_OUTPUT_CHANNELS]>,
    source_cursor_frames: f64,
}

impl AudioPlaybackSharedState {
    fn enqueue_interleaved_stereo(&mut self, samples: &[f32]) {
        for chunk in samples.chunks(OPUS_OUTPUT_CHANNELS) {
            let left = chunk.first().copied().unwrap_or(0.0);
            let right = chunk.get(1).copied().unwrap_or(left);
            self.frames.push_back([left, right]);
        }
        self.trim_overflow();
    }

    fn fill_output_f32(
        &mut self,
        output: &mut [f32],
        output_sample_rate_hz: u32,
        output_channels: usize,
        volume: f32,
    ) {
        output.fill(0.0);
        if output_channels == 0 {
            return;
        }

        // 先用最近邻重采样保证链路可播，后续再按需要替换更高质量实现。
        let output_frame_count = output.len() / output_channels;
        let source_step = OPUS_SAMPLE_RATE_HZ as f64 / output_sample_rate_hz.max(1) as f64;
        let gain = volume.max(0.0);

        for frame_index in 0..output_frame_count {
            let source_index = self.source_cursor_frames.floor() as usize;
            let [left, right] = self.frames.get(source_index).copied().unwrap_or([0.0, 0.0]);
            write_output_frame(
                &mut output[frame_index * output_channels..(frame_index + 1) * output_channels],
                left * gain,
                right * gain,
            );
            self.source_cursor_frames += source_step;
        }

        self.discard_consumed_frames();
    }

    fn trim_overflow(&mut self) {
        let overflow_frames = self.frames.len().saturating_sub(MAX_BUFFERED_AUDIO_FRAMES);
        for _ in 0..overflow_frames {
            self.frames.pop_front();
        }
        if overflow_frames > 0 {
            self.source_cursor_frames =
                (self.source_cursor_frames - overflow_frames as f64).max(0.0);
        }
        if self.frames.is_empty() {
            self.source_cursor_frames = 0.0;
        } else {
            self.source_cursor_frames = self
                .source_cursor_frames
                .min((self.frames.len().saturating_sub(1)) as f64);
        }
    }

    fn discard_consumed_frames(&mut self) {
        let consumed_frames = self.source_cursor_frames.floor() as usize;
        if consumed_frames == 0 {
            return;
        }

        if consumed_frames >= self.frames.len() {
            self.frames.clear();
            self.source_cursor_frames = 0.0;
            return;
        }

        for _ in 0..consumed_frames {
            self.frames.pop_front();
        }
        self.source_cursor_frames -= consumed_frames as f64;
    }
}

fn spawn_audio_decode_task(
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
                crate::xbx_log_error!(
                    "[xbxengine][webrtc-rs][audio] opus decoder init failed: {error}"
                );
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

fn build_output_stream(
    shared_state: Arc<Mutex<AudioPlaybackSharedState>>,
    volume_bits: Arc<AtomicU32>,
) -> Result<Stream, XbxEngineRuntimeError> {
    let host = cpal::default_host();
    let output_device = host
        .default_output_device()
        .ok_or_else(|| XbxEngineRuntimeError::new("audioOutputDeviceUnavailable"))?;
    let output_config = select_preferred_output_config(&output_device)?;
    let stream_config = output_config.config();
    let output_channels = stream_config.channels as usize;
    let output_sample_rate_hz = stream_config.sample_rate.0;

    crate::xbx_log_info!(
        "[xbxengine][webrtc-rs][audio] output config rate={}Hz channels={} format={:?}",
        output_sample_rate_hz,
        output_channels,
        output_config.sample_format()
    );

    match output_config.sample_format() {
        SampleFormat::F32 => {
            let shared_state = shared_state.clone();
            let volume_bits = volume_bits.clone();
            output_device
                .build_output_stream(
                    &stream_config,
                    move |data: &mut [f32], _| {
                        let volume = f32::from_bits(volume_bits.load(Ordering::Relaxed));
                        if let Ok(mut state) = shared_state.lock() {
                            state.fill_output_f32(
                                data,
                                output_sample_rate_hz,
                                output_channels,
                                volume,
                            );
                        } else {
                            data.fill(0.0);
                        }
                    },
                    move |error| {
                        crate::xbx_log_error!(
                            "[xbxengine][webrtc-rs][audio] output stream error: {error}"
                        );
                    },
                    None,
                )
                .map_err(|error| {
                    XbxEngineRuntimeError::new(format!(
                        "createRemoteAudioOutputStreamFailed:{error}"
                    ))
                })
        }
        SampleFormat::I16 => {
            let shared_state = shared_state.clone();
            let volume_bits = volume_bits.clone();
            let mut scratch = Vec::<f32>::new();
            output_device
                .build_output_stream(
                    &stream_config,
                    move |data: &mut [i16], _| {
                        scratch.resize(data.len(), 0.0);
                        let volume = f32::from_bits(volume_bits.load(Ordering::Relaxed));
                        if let Ok(mut state) = shared_state.lock() {
                            state.fill_output_f32(
                                &mut scratch,
                                output_sample_rate_hz,
                                output_channels,
                                volume,
                            );
                        } else {
                            scratch.fill(0.0);
                        }
                        for (index, sample) in data.iter_mut().enumerate() {
                            *sample = float_to_i16(scratch[index]);
                        }
                    },
                    move |error| {
                        crate::xbx_log_error!(
                            "[xbxengine][webrtc-rs][audio] output stream error: {error}"
                        );
                    },
                    None,
                )
                .map_err(|error| {
                    XbxEngineRuntimeError::new(format!(
                        "createRemoteAudioOutputStreamFailed:{error}"
                    ))
                })
        }
        SampleFormat::U16 => {
            let shared_state = shared_state.clone();
            let volume_bits = volume_bits.clone();
            let mut scratch = Vec::<f32>::new();
            output_device
                .build_output_stream(
                    &stream_config,
                    move |data: &mut [u16], _| {
                        scratch.resize(data.len(), 0.0);
                        let volume = f32::from_bits(volume_bits.load(Ordering::Relaxed));
                        if let Ok(mut state) = shared_state.lock() {
                            state.fill_output_f32(
                                &mut scratch,
                                output_sample_rate_hz,
                                output_channels,
                                volume,
                            );
                        } else {
                            scratch.fill(0.0);
                        }
                        for (index, sample) in data.iter_mut().enumerate() {
                            *sample = float_to_u16(scratch[index]);
                        }
                    },
                    move |error| {
                        crate::xbx_log_error!(
                            "[xbxengine][webrtc-rs][audio] output stream error: {error}"
                        );
                    },
                    None,
                )
                .map_err(|error| {
                    XbxEngineRuntimeError::new(format!(
                        "createRemoteAudioOutputStreamFailed:{error}"
                    ))
                })
        }
        sample_format => Err(XbxEngineRuntimeError::new(format!(
            "audioOutputSampleFormatUnsupported:{sample_format:?}"
        ))),
    }
}

fn spawn_audio_output_thread(
    shared_state: Arc<Mutex<AudioPlaybackSharedState>>,
    volume_bits: Arc<AtomicU32>,
    startup_sender: std_mpsc::SyncSender<Result<(), String>>,
    output_stop_receiver: std_mpsc::Receiver<()>,
) -> thread::JoinHandle<()> {
    thread::spawn(
        move || match build_output_stream(shared_state, volume_bits) {
            Ok(output_stream) => {
                if let Err(error) = output_stream.play() {
                    let _ =
                        startup_sender.send(Err(format!("startRemoteAudioOutputFailed:{error}")));
                    return;
                }
                let _ = startup_sender.send(Ok(()));
                let _ = output_stop_receiver.recv();
                drop(output_stream);
            }
            Err(error) => {
                let _ = startup_sender.send(Err(error.to_string()));
            }
        },
    )
}

fn select_preferred_output_config(
    output_device: &Device,
) -> Result<SupportedStreamConfig, XbxEngineRuntimeError> {
    let default_config = output_device.default_output_config().map_err(|error| {
        XbxEngineRuntimeError::new(format!("audioOutputConfigUnavailable:{error}"))
    })?;
    if default_config.sample_rate().0 == OPUS_SAMPLE_RATE_HZ && default_config.channels() == 2 {
        return Ok(default_config);
    }

    if let Ok(config_ranges) = output_device.supported_output_configs() {
        let mut fallback_48k = None;
        for config in config_ranges {
            if config.min_sample_rate().0 <= OPUS_SAMPLE_RATE_HZ
                && config.max_sample_rate().0 >= OPUS_SAMPLE_RATE_HZ
            {
                let candidate = config.with_sample_rate(SampleRate(OPUS_SAMPLE_RATE_HZ));
                if candidate.channels() == 2 {
                    return Ok(candidate);
                }
                if fallback_48k.is_none() {
                    fallback_48k = Some(candidate);
                }
            }
        }
        if let Some(candidate) = fallback_48k {
            return Ok(candidate);
        }
    }

    Ok(default_config)
}

fn write_output_frame(frame: &mut [f32], left: f32, right: f32) {
    match frame.len() {
        0 => {}
        1 => {
            frame[0] = (left + right) * 0.5;
        }
        _ => {
            frame[0] = left;
            frame[1] = right;
            for sample in &mut frame[2..] {
                *sample = 0.0;
            }
        }
    }
}

fn float_to_i16(value: f32) -> i16 {
    (value.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

fn float_to_u16(value: f32) -> u16 {
    let normalized = value.clamp(-1.0, 1.0) * 0.5 + 0.5;
    (normalized * u16::MAX as f32).round() as u16
}

fn now_ms_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::{AudioPlaybackSharedState, OPUS_SAMPLE_RATE_HZ};

    #[test]
    fn playback_buffer_trims_overflow_and_keeps_cursor_bounded() {
        let mut state = AudioPlaybackSharedState::default();
        let mut samples = Vec::new();
        for _ in 0..(OPUS_SAMPLE_RATE_HZ as usize + 16) {
            samples.extend_from_slice(&[0.25, -0.25]);
        }

        state.enqueue_interleaved_stereo(&samples);

        assert_eq!(state.frames.len(), OPUS_SAMPLE_RATE_HZ as usize);
        assert!(state.source_cursor_frames <= (state.frames.len().saturating_sub(1)) as f64);
    }

    #[test]
    fn playback_buffer_resamples_and_consumes_source_frames() {
        let mut state = AudioPlaybackSharedState::default();
        state.enqueue_interleaved_stereo(&[
            0.1, 0.2, //
            0.3, 0.4, //
            0.5, 0.6, //
            0.7, 0.8,
        ]);
        let mut output = vec![0.0; 4];

        state.fill_output_f32(&mut output, 24_000, 2, 1.0);

        assert_eq!(output, vec![0.1, 0.2, 0.5, 0.6]);
        assert!(state.frames.is_empty());
        assert_eq!(state.source_cursor_frames, 0.0);
    }
}
