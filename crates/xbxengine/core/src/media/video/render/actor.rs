use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::media::video::types::DecodedFrame;
use crate::media::video::render::renderer::XbxRenderState;

pub enum RendererMsg {
    Frame(DecodedFrame),
    Stop,
}

pub struct RendererActorHandle {
    tx: SyncSender<RendererMsg>,
}

impl RendererActorHandle {
    pub fn new(render_state: Arc<Mutex<XbxRenderState>>) -> Self {
        let (tx, rx) = mpsc::sync_channel(1);

        thread::Builder::new()
            .name("XbxRendererActor".into())
            .spawn(move || {
                run_renderer_loop(rx, render_state);
            })
            .expect("Failed to spawn renderer actor thread");

        Self { tx }
    }

    pub fn submit(&self, frame: DecodedFrame) -> Result<(), TrySendError<RendererMsg>> {
        self.tx.try_send(RendererMsg::Frame(frame))
    }

    pub fn stop(&self) {
        let _ = self.tx.send(RendererMsg::Stop);
    }
}

fn run_renderer_loop(
    rx: Receiver<RendererMsg>,
    render_state: Arc<Mutex<XbxRenderState>>,
) {
    while let Ok(msg) = rx.recv() {
        match msg {
            RendererMsg::Frame(frame) => {
                let mut state = match render_state.lock() {
                    Ok(guard) => guard,
                    Err(_) => continue,
                };
                
                // Set the current real-time ms before presenting so metrics are correct
                let mut render_frame = frame.surface;
                render_frame.rendered_at_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as f64)
                    .unwrap_or(0.0);
                
                if let Err(e) = state.present_frame(render_frame) {
                    crate::xbx_log_error!("[XbxRendererActor] present_frame error: {:?}", e);
                }
            }
            RendererMsg::Stop => {
                break;
            }
        }
    }
}
