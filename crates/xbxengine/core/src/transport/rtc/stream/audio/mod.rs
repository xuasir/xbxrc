use std::sync::{atomic::AtomicU32, Arc, Mutex};

use tokio::runtime::Handle;

use crate::{XbxEngineMediaRuntimeStats, XbxEngineRuntimeError};

pub(crate) mod audio_output;
mod decode;
mod output;
pub(crate) mod sink;
mod state;

pub(super) const OPUS_SAMPLE_RATE_HZ: u32 = 48_000;
pub(super) const OPUS_OUTPUT_CHANNELS: usize = 2;
pub(super) const MAX_OPUS_FRAME_SAMPLES_PER_CHANNEL: usize = 5_760;
pub(super) const MAX_BUFFERED_AUDIO_LATENCY_MS: u32 = 160;
pub(super) const MAX_BUFFERED_AUDIO_FRAMES: usize =
    (OPUS_SAMPLE_RATE_HZ as usize * MAX_BUFFERED_AUDIO_LATENCY_MS as usize) / 1_000;

pub(crate) use audio_output::XbxRemoteAudioPlaybackSession;
pub(crate) use sink::RtcAudioPlaybackSink;

pub(crate) fn build_audio_playback_components(
    runtime: &Handle,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    volume_bits: Arc<AtomicU32>,
) -> Result<(XbxRemoteAudioPlaybackSession, RtcAudioPlaybackSink), XbxEngineRuntimeError> {
    let (sender, receiver) = tokio::sync::mpsc::channel(1024);
    let session =
        XbxRemoteAudioPlaybackSession::start(runtime, receiver, runtime_stats, volume_bits)?;
    Ok((session, RtcAudioPlaybackSink::new(sender)))
}
