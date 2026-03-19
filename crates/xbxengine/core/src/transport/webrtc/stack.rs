use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::runtime::Runtime;
use xbxengine_protocol::{
    XbxEngineDisplayStateDto, XbxEngineIceCandidateDto, XbxEngineInputEventDto,
};

use crate::{
    media::video::render::renderer::XbxRenderState,
    transport::webrtc::control::{
        XbxDataChannelMediaControl, XbxMediaControlContext, XbxMediaControlPort,
    },
    transport::webrtc::data_channel::{
        queue_keyboard_pointer_input, set_keyboard_pointer_enabled, XbxDataChannelState,
    },
    transport::webrtc::escalation::{VideoEscalationController, VideoEscalationReason},
    transport::webrtc::recovery_coordinator::{RecoveryCoordinator, RecoveryRuntimeState},
    transport::webrtc::recovery_diagnosis::{diagnose_ingress_signal, diagnose_transport_signal},
    transport::webrtc::recovery_executor::apply_recovery_decision,
    transport::webrtc::recovery_signal::VideoIngressSignal,
    transport::webrtc::transport::XbxTransportState,
    XbxEngineMediaNegotiationRequest, XbxEngineMediaRuntimeStats,
    XbxEnginePendingRuntimeRecoveryAction, XbxEngineRenderFrame, XbxEngineRuntimeConfig,
    XbxEngineRuntimeError, XbxEngineVideoFrameDropObservation,
};

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
        let (tx, mut rx) =
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
        let video_pipeline_config = runtime_config.webrtc.video_pipeline.clone();
        let ingress_keyframe_cooldown_ms = runtime_config
            .webrtc
            .recovery
            .keyframe_request_stall_ms
            .max(250);
        let data_channel_state_for_supervisor = data_channel_state.clone();

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

                let stats_for_frame_loop = runtime_stats_for_spawn.clone();
                let renderer_handle = Arc::new(
                    crate::media::video::render::actor::RendererActorHandle::new(
                        supervisor_render_state.clone(),
                        stats_for_frame_loop.clone(),
                    ),
                );
                let pacer_handle = Arc::new(
                    crate::media::video::pacer::actor::PacerActorHandle::new(
                        renderer_handle.clone(),
                        stats_for_frame_loop.clone(),
                        16,
                    ),
                );
                let decode_handle = Arc::new(crate::media::video::decode::actor::DecodeActorHandle::new(
                    pacer_handle.clone(),
                    stats_for_frame_loop.clone(),
                    video_pipeline_config.jitter_buffer_min_delay_ms,
                    video_pipeline_config.jitter_buffer_max_delay_ms,
                ));
                let mut ingress = crate::media::video::ingress::scheduler::VideoIngress::new(
                    usize::from(video_pipeline_config.backlog_drop_threshold_packets.max(1)),
                    std::time::Duration::from_millis(
                        video_pipeline_config.late_frame_drop_threshold_ms,
                    ),
                );

                active_handles = Some((decode_handle.clone(), pacer_handle.clone(), renderer_handle.clone()));
                let data_channel_state_for_keyframe = data_channel_state_for_supervisor.clone();
                let pending_runtime_action_for_frame_loop =
                    pending_runtime_recovery_action_for_spawn.clone();
                let ingress_keyframe_cooldown = std::time::Duration::from_millis(
                    ingress_keyframe_cooldown_ms,
                );
                let startup_escalation_grace = std::time::Duration::from_millis(
                    runtime_config.webrtc.recovery.first_frame_grace_ms,
                );

                // Read frames in a separate task so we can still rx.recv() new tracks
                tokio::spawn(async move {
                    use crate::media::video::ingress::scheduler::FrameScheduler;
                    use crate::media::video::ingress::scheduler::IngressDecision;
                    use crate::transport::adapter::FrameSourceEvent;
                    use std::time::Instant;
                    let mut frame_count = 0u64;
                    let mut frame_drop_observation_id = 0u64;
                    let mut recent_receive_frame_times_ms = VecDeque::<f64>::new();
                    let mut last_transport_escalation_hint: Option<(String, f64)> = None;
                    let stream_started_at = Instant::now();
                    // 统一升级判定，避免 ingress/adapter 各自维护冷却状态。
                    let escalation_controller = VideoEscalationController::new(
                        ingress_keyframe_cooldown,
                        runtime_config
                            .webrtc
                            .recovery
                            .keyframe_loss_burst_threshold
                            .max(1) as u8,
                        runtime_config
                            .webrtc
                            .recovery
                            .keyframe_loss_burst_threshold
                            .saturating_add(1)
                            .max(1),
                    );
                    let mut recovery_coordinator = RecoveryCoordinator::new(
                        escalation_controller,
                        stream_started_at,
                        startup_escalation_grace,
                    );
                    let mut startup_retry_tick =
                        tokio::time::interval(Duration::from_millis(120));

                    loop {
                        tokio::select! {
                            _ = startup_retry_tick.tick() => {
                                if let Some(retry_decision) =
                                    recovery_coordinator.poll_startup_retry(
                                        stats_for_frame_loop.as_ref(),
                                    )
                                {
                                    // startup retry 仍然复用同一 recovery coordinator，
                                    // supervisor loop 只负责转发时钟事件与执行动作。
                                    publish_recovery_runtime_state(
                                        RecoveryCoordinator::runtime_state_for_diagnosis(
                                            stats_for_frame_loop.as_ref(),
                                            "startupLowQuality",
                                            stream_started_at,
                                            startup_escalation_grace,
                                        ),
                                        &stats_for_frame_loop,
                                    );
                                    apply_recovery_decision(
                                        &stats_for_frame_loop,
                                        &pending_runtime_action_for_frame_loop,
                                        &data_channel_state_for_keyframe,
                                        Some(&decode_handle),
                                        retry_decision,
                                        "startupLowQuality",
                                        now_ms_f64(),
                                    )
                                    .await;
                                }
                            }
                            maybe_event = frame_source.recv_frame() => {
                                let Some(event) = maybe_event else {
                                    crate::xbx_log_info!("[Supervisor] frame source connection closed");
                                    break;
                                };
                                match event {
                                FrameSourceEvent::Frame(encoded_frame) => {
                                    // 更新包活动时间，防止 recovery 系统误判为 stall
                                    let now_ms = now_ms_f64();
                                    recent_receive_frame_times_ms.push_back(now_ms);
                                    trim_recent_times(&mut recent_receive_frame_times_ms, now_ms);
                                    frame_count += 1;
                                    if let Ok(mut stats) = stats_for_frame_loop.lock() {
                                        stats.latest_video_packet_arrival_time_ms = Some(now_ms);
                                        stats.inbound_video_packet_count_total = frame_count;
                                        stats.inbound_video_frame_rate_fps =
                                            calculate_recent_fps(&recent_receive_frame_times_ms);
                                    }

                                    let frame_queue_depth_before_submit = ingress.queue_depth();
                                    let frame_meta = (
                                        encoded_frame.width,
                                        encoded_frame.height,
                                        encoded_frame.is_keyframe,
                                    );
                                    let reconfigure_reason =
                                        ingress.describe_reconfigure_reason(&encoded_frame);
                                    let decision = ingress.submit(encoded_frame, Instant::now());
                                    if matches!(
                                        decision,
                                        IngressDecision::DropLate
                                            | IngressDecision::DropBacklog
                                            | IngressDecision::WaitKeyframe
                                            | IngressDecision::Reconfigure
                                    ) {
                                        frame_drop_observation_id =
                                            frame_drop_observation_id.saturating_add(1);
                                        if let Ok(mut stats) = stats_for_frame_loop.lock() {
                                            stats.latest_video_frame_drop =
                                                Some(XbxEngineVideoFrameDropObservation {
                                                    observation_id: frame_drop_observation_id,
                                                    reason: map_ingress_drop_reason(
                                                        &decision,
                                                        reconfigure_reason.as_deref(),
                                                    ),
                                                    observed_at_ms: now_ms,
                                                    width: frame_meta.0,
                                                    height: frame_meta.1,
                                                    is_keyframe: frame_meta.2,
                                                    queue_depth: frame_queue_depth_before_submit,
                                                });
                                        }
                                    }
                                    if matches!(
                                        decision,
                                        IngressDecision::WaitKeyframe | IngressDecision::Reconfigure
                                    ) {
                                        let diagnosis = diagnose_ingress_signal(
                                            VideoIngressSignal::from_decision(&decision),
                                        );
                                        apply_recovery_reason(
                                            &mut recovery_coordinator,
                                            &stats_for_frame_loop,
                                            &pending_runtime_action_for_frame_loop,
                                            &data_channel_state_for_keyframe,
                                            &decode_handle,
                                            diagnosis.label,
                                            diagnosis.reason,
                                            stream_started_at,
                                            startup_escalation_grace,
                                            now_ms,
                                        )
                                        .await;
                                    }
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
                                FrameSourceEvent::RecoverySignal(signal) => {
                                    let diagnosis = diagnose_transport_signal(signal);
                                    let hint_now_ms = now_ms_f64();
                                    let should_log_transport_hint =
                                        match last_transport_escalation_hint.as_ref() {
                                            Some((last_label, last_at_ms)) => {
                                                last_label != diagnosis.label
                                                    || hint_now_ms - *last_at_ms >= 1_000.0
                                            }
                                            None => true,
                                        };
                                    if should_log_transport_hint {
                                        // transport hint 是高频信号，只在标签变化或 1s 周期到达时打印，
                                        // 避免 trace / UI 被同一条 deadline 日志刷爆。
                                        crate::xbx_log_warn!(
                                            "[Supervisor] Transport escalation hint: {}",
                                            diagnosis.label
                                        );
                                        last_transport_escalation_hint = Some((
                                            diagnosis.label.to_string(),
                                            hint_now_ms,
                                        ));
                                    }
                                    apply_recovery_reason(
                                        &mut recovery_coordinator,
                                        &stats_for_frame_loop,
                                        &pending_runtime_action_for_frame_loop,
                                        &data_channel_state_for_keyframe,
                                        &decode_handle,
                                        diagnosis.label,
                                        diagnosis.reason,
                                        stream_started_at,
                                        startup_escalation_grace,
                                        now_ms_f64(),
                                    )
                                    .await;
                                }
                            }
                        }
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
            pending_runtime_recovery_action,
            data_channel_state,
            render_state,
            runtime_config,
        }
    }
}

fn publish_recovery_runtime_state(
    state: RecoveryRuntimeState,
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
) {
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_phase = Some(state.phase.as_str().to_string());
        stats.recovery_policy_profile = Some(state.recovery_policy_profile.to_string());
        stats.recovery_diagnosis = Some(state.diagnosis_label);
        stats.recovery_coupling_mode = Some(state.coupling.mode.as_str().to_string());
        stats.recovery_coupling_summary = Some(state.coupling.summary());
    }
}

async fn apply_recovery_reason(
    recovery_coordinator: &mut RecoveryCoordinator,
    runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    pending_runtime_action: &Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
    data_channel_state: &Arc<Mutex<XbxDataChannelState>>,
    decode_handle: &Arc<crate::media::video::decode::actor::DecodeActorHandle>,
    diagnosis_label: &'static str,
    reason: VideoEscalationReason,
    stream_started_at: std::time::Instant,
    startup_escalation_grace: Duration,
    observed_at_ms: f64,
) {
    let diagnosis_label = RecoveryCoordinator::runtime_state_for_diagnosis(
        runtime_stats.as_ref(),
        diagnosis_label,
        stream_started_at,
        startup_escalation_grace,
    )
    .diagnosis_label;
    let escalation_decision =
        recovery_coordinator.on_reason_with_runtime_stats(reason, runtime_stats.as_ref());
    publish_recovery_runtime_state(
        RecoveryCoordinator::runtime_state_for_diagnosis(
            runtime_stats.as_ref(),
            diagnosis_label.as_str(),
            stream_started_at,
            startup_escalation_grace,
        ),
        runtime_stats,
    );
    apply_recovery_decision(
        runtime_stats,
        pending_runtime_action,
        data_channel_state,
        Some(decode_handle),
        escalation_decision,
        diagnosis_label.as_str(),
        observed_at_ms,
    )
    .await;
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
        if let Ok(mut stats) = self.runtime_stats.lock() {
            stats.host_display_interval_ms = host_display_interval_ms;
            stats.host_frame_age_budget_ms = host_frame_age_budget_ms;
        }
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

fn trim_recent_times(times: &mut VecDeque<f64>, now_ms: f64) {
    while let Some(front) = times.front().copied() {
        if now_ms - front <= 1_000.0 {
            break;
        }
        times.pop_front();
    }
}

fn calculate_recent_fps(times: &VecDeque<f64>) -> f64 {
    let len = times.len();
    if len < 2 {
        return 0.0;
    }
    let first = times.front().copied().unwrap_or_default();
    let last = times.back().copied().unwrap_or(first);
    let window_ms = (last - first).max(1.0);
    ((len.saturating_sub(1)) as f64 * 1_000.0 / window_ms).max(0.0)
}

fn map_ingress_drop_reason(
    decision: &crate::media::video::ingress::scheduler::IngressDecision,
    reconfigure_reason: Option<&str>,
) -> String {
    match decision {
        crate::media::video::ingress::scheduler::IngressDecision::Submit => "submit".to_string(),
        crate::media::video::ingress::scheduler::IngressDecision::DropLate => {
            "dropLate".to_string()
        }
        crate::media::video::ingress::scheduler::IngressDecision::DropBacklog => {
            "dropBacklog".to_string()
        }
        crate::media::video::ingress::scheduler::IngressDecision::WaitKeyframe => {
            "waitKeyframe".to_string()
        }
        crate::media::video::ingress::scheduler::IngressDecision::Reconfigure => {
            if let Some(reason) = reconfigure_reason {
                format!("reconfigure:{reason}")
            } else {
                "reconfigure".to_string()
            }
        }
    }
}

fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}
