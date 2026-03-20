mod media_supervisor;
mod observation;
mod recovery_driver;
mod recovery_scheduler;
mod session;
mod session_scheduler;

use std::sync::{Arc, Mutex};

use tokio::runtime::Runtime;
use xbxengine_protocol::{
    XbxEngineDisplayStateDto, XbxEngineIceCandidateDto, XbxEngineInputEventDto,
};

use crate::{
    media::video::render::renderer::XbxRenderState,
    runtime_stats_sink::RuntimeStatsSink,
    transport::webrtc::control::{
        XbxDataChannelMediaControl, XbxMediaControlContext, XbxMediaControlPort,
    },
    transport::webrtc::data_channel::{
        queue_keyboard_pointer_input, set_keyboard_pointer_enabled, XbxDataChannelState,
    },
    transport::webrtc::transport::XbxTransportState,
    XbxEngineMediaNegotiationRequest, XbxEngineMediaRuntimeStats,
    XbxEnginePendingRuntimeRecoveryAction, XbxEngineRenderFrame, XbxEngineRuntimeConfig,
    XbxEngineRuntimeError,
};
use media_supervisor::{spawn_media_supervisor, MediaSupervisorContext};

/**
 * active `webrtc-rs` 媒体栈的组合根：
 * - backend 不再直接持有一堆分散状态
 * - transport / data-channel / video 细节统一从这里编排
 * - control 动作口单独注入，后续补 recovery/render owner 时不回写 stack 主体
 */
pub(crate) trait XbxMediaStackPort: Send {
    fn sync_runtime_config(&mut self, runtime_config: XbxEngineRuntimeConfig);
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
    fn add_remote_ice_candidates(
        &self,
        remote_candidates: &[XbxEngineIceCandidateDto],
    ) -> Result<(), XbxEngineRuntimeError>;
    fn apply_display_state(
        &mut self,
        state: XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn local_candidates_snapshot(&self) -> Vec<XbxEngineIceCandidateDto>;
    fn local_ice_gathering_complete(&self) -> bool;
    fn snapshot_runtime_stats(&self) -> XbxEngineMediaRuntimeStats;
    fn take_pending_runtime_recovery_action(
        &mut self,
    ) -> Option<XbxEnginePendingRuntimeRecoveryAction>;
    fn take_latest_render_frame(&mut self) -> Option<XbxEngineRenderFrame>;
    fn set_audio_volume(&mut self, value: f32);
    fn set_microphone_capturing(&mut self, capturing: bool) -> Result<(), XbxEngineRuntimeError>;
    fn set_keyboard_pointer_enabled(&mut self, enabled: bool) -> Result<(), XbxEngineRuntimeError>;
    fn push_keyboard_pointer_input(
        &mut self,
        event: XbxEngineInputEventDto,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn request_video_keyframe(&mut self) -> Result<(), XbxEngineRuntimeError>;
    fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError>;
    fn update_host_video_timing(
        &mut self,
        host_display_interval_ms: Option<f64>,
        host_frame_age_budget_ms: Option<f64>,
    );
    fn stop(&mut self);
}

pub(crate) struct XbxActiveMediaStack {
    runtime: Arc<Runtime>,
    transport: XbxTransportState,
    control: Arc<Mutex<Box<dyn XbxMediaControlPort>>>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    pending_runtime_recovery_action: Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
    data_channel_state: Arc<Mutex<XbxDataChannelState>>,
    render_state: Arc<Mutex<XbxRenderState>>,
    runtime_config: XbxEngineRuntimeConfig,
}

impl XbxActiveMediaStack {
    pub(crate) fn new(runtime_config: XbxEngineRuntimeConfig) -> Self {
        Self::with_control(
            Arc::new(Mutex::new(Box::<XbxDataChannelMediaControl>::default())),
            runtime_config,
        )
    }

    pub(crate) fn with_control(
        control: Arc<Mutex<Box<dyn XbxMediaControlPort>>>,
        runtime_config: XbxEngineRuntimeConfig,
    ) -> Self {
        let (tx, rx) =
            tokio::sync::mpsc::channel::<Box<dyn crate::transport::adapter::FrameSource>>(1);
        let mut transport = XbxTransportState::default();
        transport.frame_source_tx = Arc::new(Mutex::new(Some(tx)));

        let render_state = Arc::new(Mutex::new(XbxRenderState::default()));
        let supervisor_render_state = render_state.clone();

        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("build webrtc-rs runtime"),
        );
        let data_channel_state = Arc::new(Mutex::new(XbxDataChannelState::default()));

        let handle = runtime.handle().clone();
        let runtime_stats_for_supervisor =
            Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default())); // 这里先预创建，后面传入
        let runtime_stats_for_spawn = runtime_stats_for_supervisor.clone(); // clone 给 spawn 使用
        let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
        let pending_runtime_recovery_action_for_spawn = pending_runtime_recovery_action.clone();
        let data_channel_state_for_supervisor = data_channel_state.clone();

        spawn_media_supervisor(
            handle.clone(),
            rx,
            MediaSupervisorContext {
                runtime_stats: runtime_stats_for_spawn,
                pending_runtime_recovery_action: pending_runtime_recovery_action_for_spawn,
                data_channel_state: data_channel_state_for_supervisor,
                render_state: supervisor_render_state,
                runtime_config: runtime_config.clone(),
            },
        );

        Self {
            runtime,
            transport,
            control,
            runtime_stats: runtime_stats_for_supervisor,
            pending_runtime_recovery_action,
            data_channel_state,
            render_state,
            runtime_config,
        }
    }
}

impl XbxMediaStackPort for XbxActiveMediaStack {
    fn sync_runtime_config(&mut self, runtime_config: XbxEngineRuntimeConfig) {
        self.runtime_config = runtime_config;
    }

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
        self.transport.create_offer(
            &self.runtime.handle(),
            &self.runtime_config.webrtc.negotiation,
        )
    }

    fn apply_remote_description(
        &self,
        answer_sdp: &str,
        remote_candidates: &[XbxEngineIceCandidateDto],
    ) -> Result<(), XbxEngineRuntimeError> {
        self.transport.apply_remote_description(
            &self.runtime.handle(),
            answer_sdp,
            remote_candidates,
        )
    }

    fn add_remote_ice_candidates(
        &self,
        remote_candidates: &[XbxEngineIceCandidateDto],
    ) -> Result<(), XbxEngineRuntimeError> {
        self.transport
            .add_remote_ice_candidates(&self.runtime.handle(), remote_candidates)
    }

    fn local_candidates_snapshot(&self) -> Vec<XbxEngineIceCandidateDto> {
        self.transport.local_candidates_snapshot()
    }

    fn local_ice_gathering_complete(&self) -> bool {
        self.transport.local_ice_gathering_complete()
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
            if let Some(frame) = render_state.peek_latest_frame() {
                // 通过 latest-slot 回填最新呈现帧，保证 runtime tick 能观测到帧推进。
                let render_signal = render_state.render_signal_snapshot(now_ms);
                stats.latest_video_frame = Some(crate::XbxEngineVideoFrameStats {
                    width: frame.width,
                    height: frame.height,
                    frame_seq: frame.frame_seq,
                    fps: render_signal.fps,
                    rendered_at_ms: frame.rendered_at_ms,
                });
                stats.video_present_fps = render_signal.fps;
                stats.latest_video_present_time_ms = render_signal.latest_present_time_ms;
                stats.video_renderer_stalled = render_signal.renderer_stalled;
                stats.video_present_submit_count_total = render_signal.present_submit_count_total;
                stats.video_present_overwrite_count_total =
                    render_signal.present_overwrite_count_total;
                // 当前 decode 热路径尚未完全接回；已 present 可视为 decode-ok 的保守下界。
                if stats.latest_video_decode_ok_time_ms.is_none() {
                    stats.latest_video_decode_ok_time_ms = render_signal.latest_present_time_ms;
                }
            } else {
                let render_signal = render_state.render_signal_snapshot(now_ms);
                stats.latest_video_present_time_ms = render_signal.latest_present_time_ms;
                stats.video_renderer_stalled = render_signal.renderer_stalled;
                stats.video_present_submit_count_total = render_signal.present_submit_count_total;
                stats.video_present_overwrite_count_total =
                    render_signal.present_overwrite_count_total;
                if stats.latest_video_decode_ok_time_ms.is_none() {
                    stats.latest_video_decode_ok_time_ms = render_signal.latest_present_time_ms;
                }
            }
        }
        stats
    }

    fn take_pending_runtime_recovery_action(
        &mut self,
    ) -> Option<XbxEnginePendingRuntimeRecoveryAction> {
        self.pending_runtime_recovery_action
            .lock()
            .ok()
            .and_then(|mut action| action.take())
    }

    fn take_latest_render_frame(&mut self) -> Option<XbxEngineRenderFrame> {
        self.render_state
            .lock()
            .ok()
            .and_then(|mut render_state| render_state.take_latest_frame())
    }

    fn set_audio_volume(&mut self, value: f32) {
        self.transport.set_audio_volume(value);
    }

    fn set_microphone_capturing(&mut self, capturing: bool) -> Result<(), XbxEngineRuntimeError> {
        self.transport
            .set_microphone_capturing(&self.runtime.handle(), capturing)
    }

    fn set_keyboard_pointer_enabled(&mut self, enabled: bool) -> Result<(), XbxEngineRuntimeError> {
        set_keyboard_pointer_enabled(&self.data_channel_state, enabled);
        Ok(())
    }

    fn push_keyboard_pointer_input(
        &mut self,
        event: XbxEngineInputEventDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        queue_keyboard_pointer_input(&self.data_channel_state, event);
        Ok(())
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

    fn update_host_video_timing(
        &mut self,
        host_display_interval_ms: Option<f64>,
        host_frame_age_budget_ms: Option<f64>,
    ) {
        RuntimeStatsSink::new(self.runtime_stats.clone())
            .record_host_video_timing(host_display_interval_ms, host_frame_age_budget_ms);
    }

    fn stop(&mut self) {
        self.transport.stop_peer_connection(&self.runtime.handle());
        if let Ok(mut pending_action) = self.pending_runtime_recovery_action.lock() {
            *pending_action = None;
        }
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
