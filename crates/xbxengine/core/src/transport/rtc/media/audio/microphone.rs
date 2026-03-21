use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc as std_mpsc, Arc,
    },
    thread,
    time::Duration,
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, SampleFormat, SampleRate, Stream, SupportedStreamConfig,
};
use opus::{Application as OpusApplication, Channels as OpusChannels, Encoder as OpusEncoder};
use tokio::{
    runtime::Handle,
    sync::{mpsc, watch},
    task::JoinHandle as TokioJoinHandle,
};

use crate::transport::rtc::media::packet_types::{RtcAudioRtpPacket, RtcRtpPacketMeta};
use crate::XbxEngineRuntimeError;

const TARGET_SAMPLE_RATE_HZ: u32 = 48_000;
const OPUS_FRAME_DURATION_MS: u64 = 20;
const OPUS_FRAME_SAMPLES: usize =
    (TARGET_SAMPLE_RATE_HZ as usize * OPUS_FRAME_DURATION_MS as usize) / 1_000;
const OPUS_MAX_PACKET_BYTES: usize = 4_000;
const PCM_CHANNEL_BUFFER_CAPACITY: usize = 64;

pub(crate) struct XbxMicrophoneSession {
    capture_stop: Arc<AtomicBool>,
    capture_thread: Option<thread::JoinHandle<()>>,
    packet_writer_task: TokioJoinHandle<()>,
    stop_signal: watch::Sender<bool>,
}

impl XbxMicrophoneSession {
    pub(crate) fn start(
        runtime: &Handle,
        tx: mpsc::Sender<RtcAudioRtpPacket>,
    ) -> Result<Self, XbxEngineRuntimeError> {
        let (pcm_sender, pcm_receiver) = mpsc::channel::<Vec<i16>>(PCM_CHANNEL_BUFFER_CAPACITY);
        let capture_stop = Arc::new(AtomicBool::new(false));
        let (source_sample_rate_hz, capture_thread) =
            spawn_microphone_capture_thread(capture_stop.clone(), pcm_sender)?;
        let (stop_signal, stop_receiver) = watch::channel(false);

        let packet_writer_task = spawn_packet_writer_task(
            runtime,
            tx,
            pcm_receiver,
            source_sample_rate_hz,
            stop_receiver.clone(),
        );

        crate::xbx_log_info!(
            "[xbxengine][rtc][mic] capture started sourceRate={}Hz targetRate={}Hz",
            source_sample_rate_hz,
            TARGET_SAMPLE_RATE_HZ
        );

        Ok(Self {
            capture_stop,
            capture_thread: Some(capture_thread),
            packet_writer_task,
            stop_signal,
        })
    }

    pub(crate) fn stop(mut self) -> Result<(), XbxEngineRuntimeError> {
        let _ = self.stop_signal.send(true);
        self.capture_stop.store(true, Ordering::Relaxed);
        if let Some(capture_thread) = self.capture_thread.take() {
            if let Err(error) = capture_thread.join() {
                crate::xbx_log_error!(
                    "[xbxengine][rtc][mic] capture thread join failed: {error:?}"
                );
            }
        }
        self.packet_writer_task.abort();

        crate::xbx_log_info!("[xbxengine][rtc][mic] capture stopped");
        Ok(())
    }
}

fn spawn_microphone_capture_thread(
    stop_signal: Arc<AtomicBool>,
    pcm_sender: mpsc::Sender<Vec<i16>>,
) -> Result<(u32, thread::JoinHandle<()>), XbxEngineRuntimeError> {
    let (startup_sender, startup_receiver) = std_mpsc::sync_channel::<Result<u32, String>>(1);
    let capture_thread = thread::spawn(move || {
        let startup_result =
            initialize_microphone_capture(&pcm_sender).map(|(stream, sample_rate)| {
                let _ = startup_sender.send(Ok(sample_rate));
                (stream, sample_rate)
            });
        let (input_stream, sample_rate_hz) = match startup_result {
            Ok(result) => result,
            Err(error) => {
                let _ = startup_sender.send(Err(error.to_string()));
                return;
            }
        };

        crate::xbx_log_info!(
            "[xbxengine][webrtc-rs][mic] capture thread running at {}Hz",
            sample_rate_hz
        );
        while !stop_signal.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(20));
        }
        drop(input_stream);
    });

    let source_sample_rate_hz = startup_receiver
        .recv()
        .map_err(|_| XbxEngineRuntimeError::new("microphoneCaptureStartupChannelClosed"))?
        .map_err(XbxEngineRuntimeError::new)?;

    Ok((source_sample_rate_hz, capture_thread))
}

fn initialize_microphone_capture(
    pcm_sender: &mpsc::Sender<Vec<i16>>,
) -> Result<(Stream, u32), XbxEngineRuntimeError> {
    let host = cpal::default_host();
    let input_device = host
        .default_input_device()
        .ok_or_else(|| XbxEngineRuntimeError::new("microphoneDeviceUnavailable"))?;
    let supported_config = select_preferred_input_config(&input_device)?;
    let sample_format = supported_config.sample_format();
    let sample_rate_hz = supported_config.sample_rate().0;
    let stream_config = supported_config.config();
    let channel_count = stream_config.channels as usize;

    let input_stream = match sample_format {
        SampleFormat::F32 => {
            let pcm_sender = pcm_sender.clone();
            input_device
                .build_input_stream(
                    &stream_config,
                    move |samples: &[f32], _| {
                        enqueue_pcm_chunk_f32(&pcm_sender, samples, channel_count);
                    },
                    move |error| {
                        crate::xbx_log_error!("[xbxengine][webrtc-rs][mic] capture error: {error}");
                    },
                    None,
                )
                .map_err(|error| {
                    XbxEngineRuntimeError::new(format!("createMicrophoneInputStreamFailed:{error}"))
                })?
        }
        SampleFormat::I16 => {
            let pcm_sender = pcm_sender.clone();
            input_device
                .build_input_stream(
                    &stream_config,
                    move |samples: &[i16], _| {
                        enqueue_pcm_chunk_i16(&pcm_sender, samples, channel_count);
                    },
                    move |error| {
                        crate::xbx_log_error!("[xbxengine][webrtc-rs][mic] capture error: {error}");
                    },
                    None,
                )
                .map_err(|error| {
                    XbxEngineRuntimeError::new(format!("createMicrophoneInputStreamFailed:{error}"))
                })?
        }
        SampleFormat::U16 => {
            let pcm_sender = pcm_sender.clone();
            input_device
                .build_input_stream(
                    &stream_config,
                    move |samples: &[u16], _| {
                        enqueue_pcm_chunk_u16(&pcm_sender, samples, channel_count);
                    },
                    move |error| {
                        crate::xbx_log_error!("[xbxengine][webrtc-rs][mic] capture error: {error}");
                    },
                    None,
                )
                .map_err(|error| {
                    XbxEngineRuntimeError::new(format!("createMicrophoneInputStreamFailed:{error}"))
                })?
        }
        _ => {
            return Err(XbxEngineRuntimeError::new(format!(
                "microphoneSampleFormatUnsupported:{sample_format:?}"
            )))
        }
    };

    input_stream.play().map_err(|error| {
        XbxEngineRuntimeError::new(format!("startMicrophoneInputFailed:{error}"))
    })?;

    Ok((input_stream, sample_rate_hz))
}

fn select_preferred_input_config(
    input_device: &Device,
) -> Result<SupportedStreamConfig, XbxEngineRuntimeError> {
    let default_config = input_device.default_input_config().map_err(|error| {
        XbxEngineRuntimeError::new(format!("microphoneConfigUnavailable:{error}"))
    })?;

    if default_config.sample_rate().0 == TARGET_SAMPLE_RATE_HZ {
        return Ok(default_config);
    }

    // 优先尝试 48k，避免额外重采样；拿不到时回退默认设备配置。
    if let Ok(config_ranges) = input_device.supported_input_configs() {
        let mut preferred = None;
        for config in config_ranges {
            if config.min_sample_rate().0 <= TARGET_SAMPLE_RATE_HZ
                && config.max_sample_rate().0 >= TARGET_SAMPLE_RATE_HZ
            {
                let candidate = config.with_sample_rate(SampleRate(TARGET_SAMPLE_RATE_HZ));
                let candidate_channels = candidate.channels();
                preferred = Some(candidate);
                if candidate_channels == 1 {
                    break;
                }
            }
        }
        if let Some(config) = preferred {
            return Ok(config);
        }
    }

    Ok(default_config)
}

fn spawn_packet_writer_task(
    runtime: &Handle,
    tx: mpsc::Sender<RtcAudioRtpPacket>,
    mut pcm_receiver: mpsc::Receiver<Vec<i16>>,
    source_sample_rate_hz: u32,
    mut stop_receiver: watch::Receiver<bool>,
) -> TokioJoinHandle<()> {
    runtime.spawn(async move {
        let mut opus_encoder = match OpusEncoder::new(
            TARGET_SAMPLE_RATE_HZ,
            OpusChannels::Mono,
            OpusApplication::Voip,
        ) {
            Ok(encoder) => encoder,
            Err(error) => {
                crate::xbx_log_error!("[xbxengine][rtc][mic] opus encoder init failed: {error}");
                return;
            }
        };

        let mut pcm_buffer = VecDeque::<i16>::with_capacity(OPUS_FRAME_SAMPLES * 4);
        let mut encoded_packet = vec![0u8; OPUS_MAX_PACKET_BYTES];

        let mut rtp_timestamp = 0u32;
        let mut sequence_number = 0u16;

        loop {
            tokio::select! {
                _ = stop_receiver.changed() => {
                    break;
                }
                next_pcm = pcm_receiver.recv() => {
                    let Some(captured_pcm) = next_pcm else {
                        break;
                    };
                    let normalized_pcm = resample_to_target_rate(&captured_pcm, source_sample_rate_hz);
                    for sample in normalized_pcm {
                        pcm_buffer.push_back(sample);
                    }

                    while pcm_buffer.len() >= OPUS_FRAME_SAMPLES {
                        let mut opus_frame = [0i16; OPUS_FRAME_SAMPLES];
                        for sample in &mut opus_frame {
                            *sample = pcm_buffer.pop_front().unwrap_or(0);
                        }

                        let encoded_size = match opus_encoder.encode(&opus_frame, &mut encoded_packet) {
                            Ok(size) => size,
                            Err(error) => {
                                crate::xbx_log_error!("[xbxengine][rtc][mic] opus encode failed: {error}");
                                return;
                            }
                        };

                        let rtp_packet = RtcAudioRtpPacket {
                            payload: encoded_packet[..encoded_size].to_vec(),
                            meta: RtcRtpPacketMeta {
                                ssrc: 0,
                                payload_type: 111,
                                sequence_number,
                                timestamp: rtp_timestamp,
                                marker: false,
                            }
                        };
                        
                        if let Err(error) = tx.try_send(rtp_packet) {
                            crate::xbx_log_error!("[xbxengine][rtc][mic] write sample failed: {error}");
                            return;
                        }

                        sequence_number = sequence_number.wrapping_add(1);
                        rtp_timestamp = rtp_timestamp.wrapping_add(OPUS_FRAME_SAMPLES as u32);
                    }
                }
            }
        }
    })
}

fn enqueue_pcm_chunk_f32(sender: &mpsc::Sender<Vec<i16>>, samples: &[f32], channels: usize) {
    if channels == 0 {
        return;
    }
    let mut mono = Vec::with_capacity(samples.len().saturating_div(channels));
    for frame in samples.chunks(channels) {
        let sum = frame.iter().copied().sum::<f32>();
        let average = sum / frame.len() as f32;
        let normalized = average.clamp(-1.0, 1.0);
        mono.push((normalized * i16::MAX as f32) as i16);
    }
    let _ = sender.try_send(mono);
}

fn enqueue_pcm_chunk_i16(sender: &mpsc::Sender<Vec<i16>>, samples: &[i16], channels: usize) {
    if channels == 0 {
        return;
    }
    let mut mono = Vec::with_capacity(samples.len().saturating_div(channels));
    for frame in samples.chunks(channels) {
        let sum = frame.iter().map(|value| *value as i32).sum::<i32>();
        mono.push((sum / frame.len() as i32) as i16);
    }
    let _ = sender.try_send(mono);
}

fn enqueue_pcm_chunk_u16(sender: &mpsc::Sender<Vec<i16>>, samples: &[u16], channels: usize) {
    if channels == 0 {
        return;
    }
    let mut mono = Vec::with_capacity(samples.len().saturating_div(channels));
    for frame in samples.chunks(channels) {
        let sum = frame
            .iter()
            .map(|value| *value as i32 - 32_768)
            .sum::<i32>();
        mono.push((sum / frame.len() as i32) as i16);
    }
    let _ = sender.try_send(mono);
}

fn resample_to_target_rate(samples: &[i16], source_sample_rate_hz: u32) -> Vec<i16> {
    if source_sample_rate_hz == TARGET_SAMPLE_RATE_HZ || samples.is_empty() {
        return samples.to_vec();
    }

    // 简单最近邻重采样：优先保证链路可用，后续如需可替换为更高质量重采样器。
    let output_len = ((samples.len() as u64 * TARGET_SAMPLE_RATE_HZ as u64)
        .saturating_add(source_sample_rate_hz as u64 - 1)
        / source_sample_rate_hz as u64) as usize;
    let mut output = Vec::with_capacity(output_len);
    for output_index in 0..output_len {
        let source_index = ((output_index as u64 * source_sample_rate_hz as u64)
            / TARGET_SAMPLE_RATE_HZ as u64) as usize;
        output.push(samples[source_index.min(samples.len() - 1)]);
    }
    output
}
