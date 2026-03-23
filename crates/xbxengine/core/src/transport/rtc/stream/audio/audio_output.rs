use std::sync::{atomic::AtomicU32, Arc, Mutex};

use tokio::{runtime::Handle, sync::watch};

use crate::XbxEngineMediaRuntimeStats;
use crate::{transport::rtc::stream::packet_types::RtcAudioRtpPacket, XbxEngineRuntimeError};

use super::{decode::spawn_audio_decode_task, output::spawn_audio_output_thread};

pub(crate) struct XbxRemoteAudioPlaybackSession {
    output_stop_sender: std::sync::mpsc::Sender<()>,
    output_thread: std::thread::JoinHandle<()>,
    decode_task: tokio::task::JoinHandle<()>,
    stop_signal: watch::Sender<bool>,
}

impl XbxRemoteAudioPlaybackSession {
    pub(crate) fn start(
        runtime: &Handle,
        rx: tokio::sync::mpsc::Receiver<RtcAudioRtpPacket>,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        volume_bits: Arc<AtomicU32>,
    ) -> Result<Self, XbxEngineRuntimeError> {
        let shared_state = Arc::new(Mutex::new(super::state::AudioPlaybackSharedState::default()));
        let (output_stop_sender, output_stop_receiver) = std::sync::mpsc::channel::<()>();
        let (startup_sender, startup_receiver) =
            std::sync::mpsc::sync_channel::<Result<(), String>>(1);
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
