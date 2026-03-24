use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc, Mutex,
};
use std::thread;

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, SampleFormat, SampleRate, Stream, SupportedStreamConfig,
};

use crate::XbxEngineRuntimeError;

use super::{state::AudioPlaybackSharedState, OPUS_SAMPLE_RATE_HZ};

pub(super) fn spawn_audio_output_thread(
    shared_state: Arc<Mutex<AudioPlaybackSharedState>>,
    volume_bits: Arc<AtomicU32>,
    startup_sender: std::sync::mpsc::SyncSender<Result<(), String>>,
    output_stop_receiver: std::sync::mpsc::Receiver<()>,
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
        "[xbxengine][rtc][audio] output config rate={}Hz channels={} format={:?}",
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
                            "[xbxengine][rtc][audio] output stream error: {error}"
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
                            "[xbxengine][rtc][audio] output stream error: {error}"
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
                            "[xbxengine][rtc][audio] output stream error: {error}"
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

fn float_to_i16(value: f32) -> i16 {
    (value.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

fn float_to_u16(value: f32) -> u16 {
    let normalized = value.clamp(-1.0, 1.0) * 0.5 + 0.5;
    (normalized * u16::MAX as f32).round() as u16
}
