use std::sync::{Arc, Mutex};

use tokio::runtime::Runtime;
use xbxengine_protocol::{XbxEngineDisplayStateDto, XbxEngineIceCandidateDto};

use crate::{
    media::video::render::renderer::XbxRenderState,
    transport::webrtc::control::{
        XbxDataChannelMediaControl, XbxMediaControlContext, XbxMediaControlPort,
    },
    transport::webrtc::data_channel::XbxDataChannelState,
    transport::webrtc::transport::XbxTransportState,
    XbxEngineMediaNegotiationRequest, XbxEngineMediaRuntimeStats, XbxEngineRenderFrame,
    XbxEngineRuntimeConfig, XbxEngineRuntimeError,
};

/**
 * active `webrtc-rs` 媒体栈的组合根：
 * - backend 不再直接持有一堆分散状态
 * - transport / data-channel / video 细节统一从这里编排
 * - control 动作口单独注入，后续补 recovery/render owner 时不回写 stack 主体
 */
pub(crate) trait XbxMediaStackPort: Send {
    fn rebuild_peer_connection(
        &mut self,
        request: &XbxEngineMediaNegotiationRequest,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn create_offer(&self) -> Result<String, XbxEngineRuntimeError>;
    fn apply_remote_description(
        &self,
        answer_sdp: &str,
        remote_candidates: &[XbxEngineIceCandidateDto],
    ) -> Result<(), XbxEngineRuntimeError>;
    fn apply_display_state(
        &mut self,
        state: XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn local_candidates_snapshot(&self) -> Vec<XbxEngineIceCandidateDto>;
    fn snapshot_runtime_stats(&self) -> XbxEngineMediaRuntimeStats;
    fn take_latest_render_frame(&mut self) -> Option<XbxEngineRenderFrame>;
    fn set_microphone_capturing(&mut self, capturing: bool) -> Result<(), XbxEngineRuntimeError>;
    fn request_video_keyframe(&mut self) -> Result<(), XbxEngineRuntimeError>;
    fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError>;
    fn stop(&mut self);
}

pub(crate) struct XbxActiveMediaStack {
    runtime: Arc<Runtime>,
    transport: XbxTransportState,
    control: Arc<Mutex<Box<dyn XbxMediaControlPort>>>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    data_channel_state: Arc<Mutex<XbxDataChannelState>>,
    render_state: Arc<Mutex<XbxRenderState>>,
    runtime_config: XbxEngineRuntimeConfig,
}

impl XbxActiveMediaStack {
    pub(crate) fn new(runtime_config: XbxEngineRuntimeConfig) -> Self {
        Self::with_control(Arc::new(Mutex::new(Box::<XbxDataChannelMediaControl>::default())), runtime_config)
    }

    pub(crate) fn with_control(
        control: Arc<Mutex<Box<dyn XbxMediaControlPort>>>,
        runtime_config: XbxEngineRuntimeConfig,
    ) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Box<dyn crate::transport::adapter::FrameSource>>(1);
        let mut transport = XbxTransportState::default();
        transport.frame_source_tx = Arc::new(Mutex::new(Some(tx)));

        let render_state = Arc::new(Mutex::new(XbxRenderState::default()));
        let supervisor_render_state = render_state.clone();

        let control_for_keyframe = control.clone();
        
        let runtime = Arc::new(tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build webrtc-rs runtime"));

        let handle = runtime.handle().clone();
        let runtime_stats_for_supervisor = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default())); // 这里先预创建，后面传入
        let runtime_stats_for_spawn = runtime_stats_for_supervisor.clone(); // clone 给 spawn 使用
        let video_pipeline_config = runtime_config.webrtc.video_pipeline.clone();

        handle.clone().spawn(async move {
            crate::xbx_log_info!("[Supervisor] started waiting for video stream");
            let mut active_handles: Option<(
                Arc<crate::media::video::decode::actor::DecodeActorHandle>,
                Arc<crate::media::video::pacer::actor::PacerActorHandle>,
                Arc<crate::media::video::render::actor::RendererActorHandle>
            )> = None;
            
            while let Some(mut frame_source) = rx.recv().await {
                crate::xbx_log_info!("[Supervisor] mounting new video frame source");
                
                // Clear old handles if any
                if let Some((decode, pacer, renderer)) = active_handles.take() {
                    decode.stop();
                    pacer.stop();
                    renderer.stop();
                }
                
                let renderer_handle = Arc::new(crate::media::video::render::actor::RendererActorHandle::new(supervisor_render_state.clone()));
                let pacer_handle = Arc::new(crate::media::video::pacer::actor::PacerActorHandle::new(renderer_handle.clone(), 16));
                let decode_handle = Arc::new(crate::media::video::decode::actor::DecodeActorHandle::new(
                    pacer_handle.clone(),
                    video_pipeline_config.jitter_buffer_min_delay_ms,
                    video_pipeline_config.jitter_buffer_max_delay_ms,
                ));
                let mut ingress = crate::media::video::ingress::scheduler::VideoIngress::new(10);
                
                active_handles = Some((decode_handle.clone(), pacer_handle.clone(), renderer_handle.clone()));
                let handle_for_inner = handle.clone();
                let control_for_keyframe = control_for_keyframe.clone();
                
                // Read frames in a separate task so we can still rx.recv() new tracks
                let stats_for_frame_loop = runtime_stats_for_spawn.clone();
                tokio::spawn(async move {
                    use crate::media::video::ingress::scheduler::FrameScheduler;
                    use crate::transport::adapter::FrameSourceEvent;
                    use std::time::Instant;
                    let mut frame_count = 0u64;
                    
                    loop {
                        let future = frame_source.recv_frame();
                        if let Some(event) = future.await {
                            match event {
                                FrameSourceEvent::Frame(encoded_frame) => {
                                    // 更新包活动时间，防止 recovery 系统误判为 stall
                                    let now_ms = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_millis() as f64)
                                        .unwrap_or(0.0);
                                    frame_count += 1;
                                    if let Ok(mut stats) = stats_for_frame_loop.lock() {
                                        stats.latest_video_packet_arrival_time_ms = Some(now_ms);
                                        stats.inbound_video_packet_count_total = frame_count;
                                    }
                                    
                                    ingress.submit(encoded_frame, Instant::now());
                                    while let Some(frame) = ingress.pop() {
                                        // 同步分辨率到 runtime_stats（供 diagnostics / recovery 使用）
                                        if frame.width > 0 {
                                            if let Ok(mut stats) = stats_for_frame_loop.lock() {
                                                stats.latest_video_stream_width = Some(frame.width);
                                                stats.latest_video_stream_height = Some(frame.height);
                                            }
                                        }
                                        
                                        let frame_is_key = frame.is_keyframe;
                                        let frame_payload_len = frame.payload.len();
                                        let frame_w = frame.width;
                                        let frame_h = frame.height;
                                        
                                        if decode_handle.submit(frame).is_err() {
                                            crate::xbx_log_warn!("[Supervisor] decode queue full, drop frame");
                                        }

                                        let frame_type = if frame_is_key { "KEYFRAME" } else { "DELTA" };
                                        // keyframe 或每 300 帧打印一次分辨率信息
                                        if frame_is_key || frame_count % 300 == 0 {
                                            let (w, h) = if frame_w > 0 {
                                                (frame_w, frame_h)
                                            } else {
                                                stats_for_frame_loop.lock().ok()
                                                    .and_then(|s| s.latest_video_stream_width.zip(s.latest_video_stream_height))
                                                    .unwrap_or((0, 0))
                                            };
                                            crate::xbx_log_info!(
                                                "[Supervisor] Network test mode: received valid {} frame (size: {}B, res: {}x{}, frame#: {})",
                                                frame_type, frame_payload_len, w, h, frame_count
                                            );
                                        }
                                    }

                                }
                                FrameSourceEvent::RequestKeyframe(reason) => {
                                    crate::xbx_log_warn!("[Supervisor] Adapter requested keyframe: {}", reason);
                                    // Make dummy context for control interface
                                    // In real implementation we should inject properly, but for now just send on control channel
                                    let dummy_transport = XbxTransportState::default();
                                    if let Ok(mut c) = control_for_keyframe.lock() {
                                        let _ = c.request_video_keyframe(XbxMediaControlContext {
                                            runtime: &handle_for_inner,
                                            transport: &dummy_transport,
                                        });
                                    }
                                }
                            }
                        } else {
                            crate::xbx_log_info!("[Supervisor] frame source connection closed");
                            break;
                        }
                    }
                });
            }
        });

        Self {
            runtime,
            transport,
            control,
            runtime_stats: runtime_stats_for_supervisor,
            data_channel_state: Arc::new(Mutex::new(XbxDataChannelState::default())),
            render_state,
            runtime_config,
        }
    }
}

impl XbxMediaStackPort for XbxActiveMediaStack {
    fn rebuild_peer_connection(
        &mut self,
        request: &XbxEngineMediaNegotiationRequest,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.data_channel_state = Arc::new(Mutex::new(XbxDataChannelState::default()));
        // 先切断 decode/render 热路径，当前阶段只保留网络稳定器相关链路。
        {
            let mut render_state = self
                .render_state
                .lock()
                .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRenderStateLockFailed"))?;
            render_state.reset()?;
        }
        self.transport.rebuild_peer_connection(
            &self.runtime.handle(),
            request,
            self.data_channel_state.clone(),
            self.runtime_stats.clone(),
            self.render_state.clone(),
            &self.runtime_config.webrtc,
        )
    }

    fn create_offer(&self) -> Result<String, XbxEngineRuntimeError> {
        self.transport.create_offer(&self.runtime.handle())
    }

    fn apply_remote_description(
        &self,
        answer_sdp: &str,
        remote_candidates: &[XbxEngineIceCandidateDto],
    ) -> Result<(), XbxEngineRuntimeError> {
        self.transport
            .apply_remote_description(&self.runtime.handle(), answer_sdp, remote_candidates)
    }

    fn local_candidates_snapshot(&self) -> Vec<XbxEngineIceCandidateDto> {
        self.transport.local_candidates_snapshot()
    }

    fn apply_display_state(
        &mut self,
        state: XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        let mut render_state = self
            .render_state
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRenderStateLockFailed"))?;
        render_state.apply_display_state(state)
    }

    fn snapshot_runtime_stats(&self) -> XbxEngineMediaRuntimeStats {
        let mut stats = self
            .runtime_stats
            .lock()
            .ok()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        let now_ms = now_ms_f64();
        if let Ok(render_state) = self.render_state.lock() {
            let render_signal = render_state.render_signal_snapshot(now_ms);
            stats.latest_video_present_time_ms = render_signal.latest_present_time_ms;
            stats.video_renderer_stalled = render_signal.renderer_stalled;
            // 当前 decode 热路径尚未完全接回；已 present 可视为 decode-ok 的保守下界。
            if stats.latest_video_decode_ok_time_ms.is_none() {
                stats.latest_video_decode_ok_time_ms = render_signal.latest_present_time_ms;
            }
        }
        stats
    }

    fn take_latest_render_frame(&mut self) -> Option<XbxEngineRenderFrame> {
        self.render_state
            .lock()
            .ok()
            .and_then(|mut render_state| render_state.take_latest_frame())
    }

    fn set_microphone_capturing(&mut self, capturing: bool) -> Result<(), XbxEngineRuntimeError> {
        self.transport
            .set_microphone_capturing(&self.runtime.handle(), capturing)
    }

    fn request_video_keyframe(&mut self) -> Result<(), XbxEngineRuntimeError> {
        let mut control = self.control.lock().unwrap();
        control.request_video_keyframe(XbxMediaControlContext {
            runtime: &self.runtime.handle(),
            transport: &self.transport,
        })
    }

    fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        let mut control = self.control.lock().unwrap();
        control.request_decoder_reset(XbxMediaControlContext {
            runtime: &self.runtime.handle(),
            transport: &self.transport,
        })
    }

    fn stop(&mut self) {
        self.transport.stop_peer_connection(&self.runtime.handle());
        if let Ok(mut render_state) = self.render_state.lock() {
            render_state.stop();
        }
    }
}

fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}
