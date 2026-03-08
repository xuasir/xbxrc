use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread;
use std::time::{Duration, Instant};

use crate::media::video::types::DecodedFrame;
use crate::media::video::render::actor::RendererActorHandle;
use std::sync::Arc;

pub enum PacerMsg {
    Frame(DecodedFrame),
    Flush,
    Stop,
}

pub struct PacerActorHandle {
    tx: SyncSender<PacerMsg>,
}

impl PacerActorHandle {
    pub fn new(renderer: Arc<RendererActorHandle>, refresh_interval_ms: u64) -> Self {
        let (tx, rx) = mpsc::sync_channel(2);

        thread::Builder::new()
            .name("XbxPacerActor".into())
            .spawn(move || {
                run_pacer_loop(rx, renderer, refresh_interval_ms);
            })
            .expect("Failed to spawn pacer actor thread");

        Self { tx }
    }

    pub fn submit(&self, frame: DecodedFrame) -> Result<(), TrySendError<PacerMsg>> {
        self.tx.try_send(PacerMsg::Frame(frame))
    }

    pub fn flush(&self) {
        let _ = self.tx.send(PacerMsg::Flush);
    }

    pub fn stop(&self) {
        let _ = self.tx.send(PacerMsg::Stop);
    }
}

fn run_pacer_loop(
    rx: Receiver<PacerMsg>,
    renderer: Arc<RendererActorHandle>,
    refresh_interval_ms: u64,
) {
    let refresh_interval = Duration::from_millis(refresh_interval_ms);
    let catch_up_threshold = Duration::from_millis(500);
    let mut catch_up_mode = false;

    while let Ok(msg) = rx.recv() {
        match msg {
            PacerMsg::Frame(frame) => {
                let now = Instant::now();
                let deadline = frame.pts;

                if catch_up_mode {
                    if now > deadline + catch_up_threshold {
                        continue;
                    } else {
                        catch_up_mode = false;
                    }
                }

                // If massive backlog, enter catch_up_mode
                if now > deadline + catch_up_threshold {
                    catch_up_mode = true;
                    continue;
                }

                // If late, but within 500ms, just submit immediately (don't sleep)
                if now >= deadline {
                    // Late or Perfect time.
                    // (removed strict drop if now > deadline + refresh_interval)
                } else {
                    // Early frame
                    let sleep_time = deadline.duration_since(now);
                    thread::sleep(sleep_time);
                }

                // Render queue size 1 as per RFC (or renderer limits itself).
                // Renderer handles immediate flip.
                if renderer.submit(frame).is_err() {
                    crate::xbx_log_warn!("[XbxPacerActor] renderer queue full, frame dropped!");
                }
            }
            PacerMsg::Flush => {
                catch_up_mode = false;
            }
            PacerMsg::Stop => {
                break;
            }
        }
    }
}
