use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread;

use crate::media::video::decode::video_decode::XbxVideoDecodeState;
use crate::media::video::types::{DecodedFrame, EncodedFrame};
use crate::media::video::pacer::actor::PacerActorHandle;
use std::sync::Arc;

pub enum DecodeMsg {
    Frame(EncodedFrame),
    Flush,
    Stop,
}

pub struct DecodeActorHandle {
    tx: SyncSender<DecodeMsg>,
}

impl DecodeActorHandle {
    pub fn new(pacer: Arc<PacerActorHandle>, min_delay_ms: u64, max_delay_ms: u64) -> Self {
        let (tx, rx) = mpsc::sync_channel(2);

        thread::Builder::new()
            .name("XbxDecodeActor".into())
            .spawn(move || {
                run_decode_loop(rx, pacer, min_delay_ms, max_delay_ms);
            })
            .expect("Failed to spawn decode actor thread");

        Self { tx }
    }

    pub fn submit(&self, frame: EncodedFrame) -> Result<(), TrySendError<DecodeMsg>> {
        match self.tx.try_send(DecodeMsg::Frame(frame)) {
            Ok(_) => Ok(()),
            Err(e) => {
                match e {
                    TrySendError::Full(_) => Err(e),
                    TrySendError::Disconnected(_) => {
                        crate::xbx_log_error!("[DecodeActorHandle] Decode thread is disconnected (likely panicked)!");
                        Err(e)
                    }
                }
            }
        }
    }

    pub fn flush(&self) {
        let _ = self.tx.send(DecodeMsg::Flush);
    }

    pub fn stop(&self) {
        let _ = self.tx.send(DecodeMsg::Stop);
    }
}

fn run_decode_loop(rx: Receiver<DecodeMsg>, pacer: Arc<PacerActorHandle>, min_delay_ms: u64, max_delay_ms: u64) {
    // 设置线程局部的 panic hook，确保崩溃信息能被记录到 xbx_log
    std::panic::set_hook(Box::new(|panic_info| {
        let msg = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            *s
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            &s[..]
        } else {
            "Box<Any>"
        };
        crate::xbx_log_error!("[XbxDecodeActor] PANIC occurred: {} at {:?}", msg, panic_info.location());
    }));

    let mut decode_state = match XbxVideoDecodeState::new(min_delay_ms, max_delay_ms) {
        Ok(state) => state,
        Err(e) => {
            crate::xbx_log_error!("Failed to initialize hardware decoder: {:?}", e);
            return;
        }
    };

    while let Ok(msg) = rx.recv() {
        match msg {
            DecodeMsg::Frame(frame) => {
                let target_time = frame.target_playout_time;
                let now_ms = crate::media::video::decode::video_decode::now_ms_f64();
                crate::xbx_log_warn!("[XbxDecodeActor] processing frame ts={} len={}", frame.rtp_timestamp, frame.payload.len());
                decode_state.process_encoded_frame(frame, now_ms);
                while let Some(render_frame) = decode_state.pop_decoded_frame(now_ms) {
                    let decoded_frame = DecodedFrame {
                        width: render_frame.width,
                        height: render_frame.height,
                        pts: target_time, // map pts back
                        surface: render_frame,
                    };
                    
                    // DecodeActor sends decoded frame to pacer queue
                    if pacer.submit(decoded_frame).is_err() {
                        crate::xbx_log_warn!("[XbxDecodeActor] pacer queue full, drop frame");
                    }
                }
            }
            DecodeMsg::Flush => {
                let _ = decode_state.request_decoder_reset();
            }
            DecodeMsg::Stop => {
                break;
            }
        }
    }
}
