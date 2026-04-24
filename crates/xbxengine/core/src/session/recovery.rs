use xbxengine_protocol::{XbxEngineReconnectReasonDto, XbxEngineTransportStateDto};

/**
 * 这是 session/runtime 级 watchdog：
 * - 负责“整场会话是否长期卡死、是否需要整路 reconnect”这类粗粒度恢复
 * - 不负责 RTC 视频链里的 signal/diagnosis/policy/executor 四层恢复
 *
 * 也就是说：
 * - `transport/rtc/recovery_*` 处理媒体链内部恢复
 * - 本模块只处理 runtime 级别的 session health / reconnect 判定
 *
 * 保留这层是为了让“会话级恢复”和“媒体级恢复”分权，避免再次揉成一团。
 */

pub const FIRST_FRAME_GRACE_MS: f64 = 8_000.0;
pub const KEYFRAME_REQUEST_STALL_MS: f64 = 1_500.0;
pub const KEYFRAME_LOSS_BURST_THRESHOLD: u8 = 2;
pub const DECODER_RESET_AFTER_KEYFRAME_WAIT_MS: f64 = 500.0;
pub const DECODER_RESET_REQUEST_COOLDOWN_MS: f64 = 1_500.0;
pub const RECONNECT_STALL_MS: f64 = 4_000.0;
pub const STALL_RECOVERY_COOLDOWN_MS: f64 = 6_000.0;
pub const STALL_SIGNAL_STABILITY_MS: f64 = 250.0;
pub const AUDIO_ALIVE_VIDEO_ONLY_KEYFRAME_STALL_MS: f64 = 800.0;
pub const AUDIO_ALIVE_VIDEO_ONLY_DECODER_RESET_WAIT_MS: f64 = 300.0;
pub const TWCC_ALIVE_RECONNECT_STALL_MS: f64 = 8_000.0;
pub const TWCC_RECENT_FEEDBACK_GRACE_MS: f64 = 500.0;
pub const NACK_RECENT_GRACE_MS: f64 = 120.0;
pub const VIDEO_PACKET_RECENT_GRACE_MS: f64 = 250.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XbxEngineRecoveryRuntimeConfig {
    pub first_frame_grace_ms: u64,
    pub keyframe_request_stall_ms: u64,
    pub keyframe_loss_burst_threshold: u8,
    pub decoder_reset_after_keyframe_wait_ms: u64,
    pub decoder_reset_request_cooldown_ms: u64,
    pub reconnect_stall_ms: u64,
    pub stall_recovery_cooldown_ms: u64,
}

impl Default for XbxEngineRecoveryRuntimeConfig {
    fn default() -> Self {
        Self {
            first_frame_grace_ms: FIRST_FRAME_GRACE_MS as u64,
            keyframe_request_stall_ms: KEYFRAME_REQUEST_STALL_MS as u64,
            keyframe_loss_burst_threshold: KEYFRAME_LOSS_BURST_THRESHOLD,
            decoder_reset_after_keyframe_wait_ms: DECODER_RESET_AFTER_KEYFRAME_WAIT_MS as u64,
            decoder_reset_request_cooldown_ms: DECODER_RESET_REQUEST_COOLDOWN_MS as u64,
            reconnect_stall_ms: RECONNECT_STALL_MS as u64,
            stall_recovery_cooldown_ms: STALL_RECOVERY_COOLDOWN_MS as u64,
        }
    }
}

impl XbxEngineRecoveryRuntimeConfig {
    pub fn with_override(self, override_config: XbxEngineRecoveryRuntimeConfigOverride) -> Self {
        Self {
            first_frame_grace_ms: override_config
                .first_frame_grace_ms
                .unwrap_or(self.first_frame_grace_ms),
            keyframe_request_stall_ms: override_config
                .keyframe_request_stall_ms
                .unwrap_or(self.keyframe_request_stall_ms),
            keyframe_loss_burst_threshold: override_config
                .keyframe_loss_burst_threshold
                .unwrap_or(self.keyframe_loss_burst_threshold),
            decoder_reset_after_keyframe_wait_ms: override_config
                .decoder_reset_after_keyframe_wait_ms
                .unwrap_or(self.decoder_reset_after_keyframe_wait_ms),
            decoder_reset_request_cooldown_ms: override_config
                .decoder_reset_request_cooldown_ms
                .unwrap_or(self.decoder_reset_request_cooldown_ms),
            reconnect_stall_ms: override_config
                .reconnect_stall_ms
                .unwrap_or(self.reconnect_stall_ms),
            stall_recovery_cooldown_ms: override_config
                .stall_recovery_cooldown_ms
                .unwrap_or(self.stall_recovery_cooldown_ms),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct XbxEngineRecoveryRuntimeConfigOverride {
    pub first_frame_grace_ms: Option<u64>,
    pub keyframe_request_stall_ms: Option<u64>,
    pub keyframe_loss_burst_threshold: Option<u8>,
    pub decoder_reset_after_keyframe_wait_ms: Option<u64>,
    pub decoder_reset_request_cooldown_ms: Option<u64>,
    pub reconnect_stall_ms: Option<u64>,
    pub stall_recovery_cooldown_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct XbxEngineRuntimeHealth {
    pub observed_transport_state: XbxEngineTransportStateDto,
    pub connected_at_ms: Option<f64>,
    pub last_frame_seq: u64,
    pub last_frame_rendered_at_ms: Option<f64>,
    pub inbound_video_packet_count_total: u64,
    pub last_video_packet_arrival_at_ms: Option<f64>,
    pub video_size: Option<(u32, u32)>,
    pub last_keyframe_request_at_ms: Option<f64>,
    pub last_decoder_reset_request_at_ms: Option<f64>,
    pub last_reconnect_started_at_ms: Option<f64>,
    pub stall_candidate_started_at_ms: Option<f64>,
    pub keyframe_requested_for_current_stall: bool,
    pub decoder_reset_requested_for_current_stall: bool,
    pub last_native_presenter_reset_at_ms: Option<f64>,
    pub last_native_presenter_reset_display_tick_epoch: Option<u64>,
    pub last_renderer_submit_count_total: u64,
    pub last_renderer_submit_advanced_at_ms: Option<f64>,
}

impl Default for XbxEngineRuntimeHealth {
    fn default() -> Self {
        Self {
            observed_transport_state: XbxEngineTransportStateDto::New,
            connected_at_ms: None,
            last_frame_seq: 0,
            last_frame_rendered_at_ms: None,
            inbound_video_packet_count_total: 0,
            last_video_packet_arrival_at_ms: None,
            video_size: None,
            last_keyframe_request_at_ms: None,
            last_decoder_reset_request_at_ms: None,
            last_reconnect_started_at_ms: None,
            stall_candidate_started_at_ms: None,
            keyframe_requested_for_current_stall: false,
            decoder_reset_requested_for_current_stall: false,
            last_native_presenter_reset_at_ms: None,
            last_native_presenter_reset_display_tick_epoch: None,
            last_renderer_submit_count_total: 0,
            last_renderer_submit_advanced_at_ms: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum XbxEngineRecoveryAction {
    RequestVideoKeyframe,
    RequestDecoderReset,
    RequestReconnect(XbxEngineReconnectReasonDto),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct XbxEngineTransportSignal {
    pub transport_connected: bool,
    pub connected_at_ms: Option<f64>,
    pub latest_video_packet_arrival_at_ms: Option<f64>,
    pub latest_twcc_feedback_at_ms: Option<f64>,
    pub latest_nack_sent_at_ms: Option<f64>,
    pub latest_nack_recovered_at_ms: Option<f64>,
    pub latest_nack_expired_at_ms: Option<f64>,
    pub latest_nack_expired_frame_is_keyframe: bool,
    pub audio_stream_alive: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct XbxEngineMediaSignal {
    pub latest_frame_decoded_at_ms: Option<f64>,
    pub latest_frame_presented_at_ms: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct XbxEngineDecodeRenderSignal {
    // Phase-1 先占位，后续接 decoder/render stall 信号。
    pub decoder_stalled: Option<bool>,
    pub render_stalled: Option<bool>,
    pub allow_decoder_reset: bool,
    pub local_media_backpressure_active: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct XbxEngineRecoverySignals {
    pub transport: XbxEngineTransportSignal,
    pub media: XbxEngineMediaSignal,
    pub decode_render: XbxEngineDecodeRenderSignal,
}

impl XbxEngineRuntimeHealth {
    pub fn reset_video_epoch(&mut self) {
        // 媒体会话重建后，帧序号和最近一次视频活动都必须重新开始统计，
        // 但 transport 连接态本身不在这里动，避免把已连接状态误清掉。
        self.last_frame_seq = 0;
        self.last_frame_rendered_at_ms = None;
        self.inbound_video_packet_count_total = 0;
        self.last_video_packet_arrival_at_ms = None;
        self.last_keyframe_request_at_ms = None;
        self.last_decoder_reset_request_at_ms = None;
        self.stall_candidate_started_at_ms = None;
        self.keyframe_requested_for_current_stall = false;
        self.decoder_reset_requested_for_current_stall = false;
    }

    pub fn sync_transport_state(
        &mut self,
        next_state: &XbxEngineTransportStateDto,
        now_ms: f64,
    ) -> bool {
        if &self.observed_transport_state == next_state {
            return false;
        }

        self.observed_transport_state = next_state.clone();
        match next_state {
            XbxEngineTransportStateDto::Connected => {
                self.connected_at_ms = Some(now_ms);
                self.keyframe_requested_for_current_stall = false;
                self.decoder_reset_requested_for_current_stall = false;
                self.stall_candidate_started_at_ms = None;
            }
            XbxEngineTransportStateDto::Connecting => {
                self.connected_at_ms = None;
                self.decoder_reset_requested_for_current_stall = false;
                self.stall_candidate_started_at_ms = None;
            }
            XbxEngineTransportStateDto::Disconnected
            | XbxEngineTransportStateDto::Failed
            | XbxEngineTransportStateDto::Closed
            | XbxEngineTransportStateDto::New => {
                self.connected_at_ms = None;
                self.last_frame_rendered_at_ms = None;
                self.last_video_packet_arrival_at_ms = None;
                self.inbound_video_packet_count_total = 0;
                self.keyframe_requested_for_current_stall = false;
                self.decoder_reset_requested_for_current_stall = false;
                self.stall_candidate_started_at_ms = None;
            }
        }
        true
    }

    pub fn record_video_frame(
        &mut self,
        width: u32,
        height: u32,
        frame_seq: u64,
        rendered_at_ms: f64,
    ) -> Option<(u32, u32)> {
        if frame_seq <= self.last_frame_seq {
            return None;
        }

        let previous_video_size = self.video_size;
        self.last_frame_seq = frame_seq;
        self.last_frame_rendered_at_ms = Some(rendered_at_ms);
        self.video_size = Some((width, height));
        self.keyframe_requested_for_current_stall = false;
        self.decoder_reset_requested_for_current_stall = false;
        self.stall_candidate_started_at_ms = None;
        if previous_video_size != Some((width, height)) {
            return Some((width, height));
        }
        None
    }

    pub fn record_video_packet_activity(
        &mut self,
        inbound_video_packet_count_total: u64,
        arrived_at_ms: f64,
    ) {
        if inbound_video_packet_count_total <= self.inbound_video_packet_count_total {
            return;
        }
        self.inbound_video_packet_count_total = inbound_video_packet_count_total;
        self.last_video_packet_arrival_at_ms = Some(arrived_at_ms);
        self.keyframe_requested_for_current_stall = false;
        self.decoder_reset_requested_for_current_stall = false;
        self.stall_candidate_started_at_ms = None;
    }

    pub fn mark_keyframe_requested(&mut self, now_ms: f64) {
        self.last_keyframe_request_at_ms = Some(now_ms);
        self.keyframe_requested_for_current_stall = true;
    }

    pub fn mark_decoder_reset_requested(&mut self, now_ms: f64) {
        self.last_decoder_reset_request_at_ms = Some(now_ms);
        self.decoder_reset_requested_for_current_stall = true;
    }

    pub fn mark_reconnect_started(&mut self, now_ms: f64) {
        self.last_reconnect_started_at_ms = Some(now_ms);
        self.keyframe_requested_for_current_stall = false;
        self.decoder_reset_requested_for_current_stall = false;
        self.stall_candidate_started_at_ms = None;
    }

    pub fn restore_reconnect_marker(&mut self, reconnect_started_at_ms: f64) {
        self.last_reconnect_started_at_ms = Some(reconnect_started_at_ms);
    }

    pub fn update_stall_candidate(
        &mut self,
        now_ms: f64,
        should_track_stall: bool,
        stable_window_ms: f64,
    ) -> bool {
        if !should_track_stall {
            self.stall_candidate_started_at_ms = None;
            return false;
        }
        let started_at = self.stall_candidate_started_at_ms.get_or_insert(now_ms);
        now_ms - *started_at >= stable_window_ms
    }

    pub fn next_recovery_action(
        &self,
        now_ms: f64,
        runtime_state_is_running: bool,
        transport_state: &XbxEngineTransportStateDto,
    ) -> Option<XbxEngineRecoveryAction> {
        let signals = XbxEngineRecoverySignals {
            transport: XbxEngineTransportSignal {
                transport_connected: transport_state == &XbxEngineTransportStateDto::Connected,
                connected_at_ms: self.connected_at_ms,
                latest_video_packet_arrival_at_ms: self.last_video_packet_arrival_at_ms,
                latest_twcc_feedback_at_ms: None,
                latest_nack_sent_at_ms: None,
                latest_nack_recovered_at_ms: None,
                latest_nack_expired_at_ms: None,
                latest_nack_expired_frame_is_keyframe: false,
                audio_stream_alive: false,
            },
            media: XbxEngineMediaSignal {
                // 兼容旧入口：该入口没有宿主 present telemetry，避免把 rendered 时钟伪装成 present。
                latest_frame_decoded_at_ms: self.last_frame_rendered_at_ms,
                latest_frame_presented_at_ms: None,
            },
            decode_render: XbxEngineDecodeRenderSignal::default(),
        };
        self.next_recovery_action_with_signals_and_config(
            now_ms,
            runtime_state_is_running,
            signals,
            &XbxEngineRecoveryRuntimeConfig::default(),
        )
    }

    pub fn next_recovery_action_with_signals(
        &self,
        now_ms: f64,
        runtime_state_is_running: bool,
        signals: XbxEngineRecoverySignals,
    ) -> Option<XbxEngineRecoveryAction> {
        self.next_recovery_action_with_signals_and_config(
            now_ms,
            runtime_state_is_running,
            signals,
            &XbxEngineRecoveryRuntimeConfig::default(),
        )
    }

    pub fn next_recovery_action_with_signals_and_config(
        &self,
        now_ms: f64,
        runtime_state_is_running: bool,
        signals: XbxEngineRecoverySignals,
        recovery_config: &XbxEngineRecoveryRuntimeConfig,
    ) -> Option<XbxEngineRecoveryAction> {
        if !runtime_state_is_running || !signals.transport.transport_connected {
            return None;
        }
        let first_frame_grace_ms = recovery_config.first_frame_grace_ms as f64;
        let keyframe_request_stall_ms = recovery_config.keyframe_request_stall_ms as f64;
        let decoder_reset_after_keyframe_wait_ms =
            recovery_config.decoder_reset_after_keyframe_wait_ms as f64;
        let decoder_reset_request_cooldown_ms =
            recovery_config.decoder_reset_request_cooldown_ms as f64;
        let reconnect_stall_ms = recovery_config.reconnect_stall_ms as f64;
        let stall_recovery_cooldown_ms = recovery_config.stall_recovery_cooldown_ms as f64;
        let recent_media_activity_grace_ms = keyframe_request_stall_ms.min(500.0);
        let effective_keyframe_request_stall_ms = if signals.transport.audio_stream_alive {
            keyframe_request_stall_ms.min(AUDIO_ALIVE_VIDEO_ONLY_KEYFRAME_STALL_MS)
        } else {
            keyframe_request_stall_ms
        };
        let effective_decoder_reset_after_keyframe_wait_ms = if signals.transport.audio_stream_alive
        {
            decoder_reset_after_keyframe_wait_ms.min(AUDIO_ALIVE_VIDEO_ONLY_DECODER_RESET_WAIT_MS)
        } else {
            decoder_reset_after_keyframe_wait_ms
        };
        let recent_twcc_feedback = signals
            .transport
            .latest_twcc_feedback_at_ms
            .map(|at_ms| now_ms - at_ms < TWCC_RECENT_FEEDBACK_GRACE_MS)
            .unwrap_or(false);
        let recent_nack_sent = signals
            .transport
            .latest_nack_sent_at_ms
            .map(|at_ms| now_ms - at_ms < NACK_RECENT_GRACE_MS)
            .unwrap_or(false);
        let recent_nack_recovered = signals
            .transport
            .latest_nack_recovered_at_ms
            .map(|at_ms| now_ms - at_ms < NACK_RECENT_GRACE_MS)
            .unwrap_or(false);
        let recent_nack_expired = signals
            .transport
            .latest_nack_expired_at_ms
            .map(|at_ms| now_ms - at_ms < NACK_RECENT_GRACE_MS)
            .unwrap_or(false);
        let recent_video_packets = signals
            .transport
            .latest_video_packet_arrival_at_ms
            .map(|at_ms| now_ms - at_ms < VIDEO_PACKET_RECENT_GRACE_MS)
            .unwrap_or(false);
        // TWCC 仍然活跃时，允许视频链稍微晚一点才升级到整路重连。
        // 音频存活时只做视频侧恢复，不把它当成重连兜底的延长条件。
        let effective_reconnect_stall_ms = if recent_twcc_feedback {
            reconnect_stall_ms
                .max(keyframe_request_stall_ms * 2.0)
                .max(TWCC_ALIVE_RECONNECT_STALL_MS)
        } else {
            reconnect_stall_ms
        };

        let connected_at_ms = signals.transport.connected_at_ms.unwrap_or(now_ms);
        // 把“媒体推进停滞”和“传输仍有新包”显式拆开：
        // - packet 持续到达不代表 decode/render 还在推进
        // - pipeline stall 场景里如果继续用 packet activity 覆盖 stalled_for，
        //   会把已经冻结数秒的画面误判成“系统仍然活跃”
        let latest_media_activity_at_ms = signals
            .media
            .latest_frame_presented_at_ms
            .or(signals.media.latest_frame_decoded_at_ms);
        let media_stalled_for_ms = latest_media_activity_at_ms
            .map(|at_ms| now_ms - at_ms)
            .unwrap_or_else(|| {
                signals
                    .transport
                    .latest_video_packet_arrival_at_ms
                    .map(|at_ms| now_ms - at_ms)
                    .unwrap_or(now_ms - connected_at_ms)
            });

        if signals.media.latest_frame_presented_at_ms.is_none()
            && signals
                .transport
                .latest_video_packet_arrival_at_ms
                .is_none()
            && now_ms - connected_at_ms < first_frame_grace_ms
        {
            return None;
        }

        // 最近仍在持续 decode/present 时，优先相信新鲜活动，避免短抖动误触发恢复。
        let has_fresh_media_activity = signals
            .media
            .latest_frame_presented_at_ms
            .map(|at_ms| now_ms - at_ms < recent_media_activity_grace_ms)
            .unwrap_or(false)
            || signals
                .media
                .latest_frame_decoded_at_ms
                .map(|at_ms| now_ms - at_ms < recent_media_activity_grace_ms)
                .unwrap_or(false);
        if has_fresh_media_activity {
            return None;
        }
        if recent_nack_recovered {
            return None;
        }
        let transport_hard_recovery_evidence = Self::has_transport_hard_recovery_evidence(
            now_ms,
            &signals.transport,
            effective_keyframe_request_stall_ms,
            recent_twcc_feedback,
            recent_nack_expired,
        );
        let local_media_backpressure =
            signals.decode_render.local_media_backpressure_active && recent_video_packets;
        if local_media_backpressure && !transport_hard_recovery_evidence {
            return None;
        }

        // 进入游戏时常见“音频先恢复、视频短暂断流”的单路 stall，
        // 这里允许更早进入 keyframe/decoder reset，而不是被动等到 reconnect。
        let sustained_video_only_stall = signals.transport.audio_stream_alive
            && signals
                .transport
                .latest_video_packet_arrival_at_ms
                .map(|at_ms| now_ms - at_ms >= effective_keyframe_request_stall_ms)
                .unwrap_or(true)
            && signals
                .media
                .latest_frame_decoded_at_ms
                .map(|at_ms| now_ms - at_ms >= effective_keyframe_request_stall_ms)
                .unwrap_or(true)
            && signals
                .media
                .latest_frame_presented_at_ms
                .map(|at_ms| now_ms - at_ms >= effective_keyframe_request_stall_ms)
                .unwrap_or(true);
        let can_try_decoder_reset = signals.decode_render.allow_decoder_reset
            && signals.decode_render.decoder_stalled == Some(true)
            && signals.decode_render.render_stalled != Some(true);
        if recent_nack_sent && !recent_nack_expired {
            return None;
        }
        let should_request_keyframe = ((media_stalled_for_ms
            >= effective_keyframe_request_stall_ms
            && transport_hard_recovery_evidence)
            || can_try_decoder_reset
            || (recent_nack_expired && signals.transport.latest_nack_expired_frame_is_keyframe))
            && !self.keyframe_requested_for_current_stall
            && self
                .last_keyframe_request_at_ms
                .map(|last| now_ms - last >= effective_keyframe_request_stall_ms)
                .unwrap_or(true);
        if should_request_keyframe {
            return Some(XbxEngineRecoveryAction::RequestVideoKeyframe);
        }
        let should_request_decoder_reset = (can_try_decoder_reset
            || sustained_video_only_stall
            || media_stalled_for_ms >= effective_keyframe_request_stall_ms)
            && self.keyframe_requested_for_current_stall
            && !self.decoder_reset_requested_for_current_stall
            && media_stalled_for_ms < effective_reconnect_stall_ms
            && self
                .last_keyframe_request_at_ms
                .map(|last| now_ms - last >= effective_decoder_reset_after_keyframe_wait_ms)
                .unwrap_or(true)
            && self
                .last_decoder_reset_request_at_ms
                .map(|last| now_ms - last >= decoder_reset_request_cooldown_ms)
                .unwrap_or(true);
        if should_request_decoder_reset {
            return Some(XbxEngineRecoveryAction::RequestDecoderReset);
        }

        if signals.transport.audio_stream_alive || !transport_hard_recovery_evidence {
            return None;
        }
        if media_stalled_for_ms < effective_reconnect_stall_ms {
            return None;
        }
        if self
            .last_reconnect_started_at_ms
            .map(|last| now_ms - last < stall_recovery_cooldown_ms)
            .unwrap_or(false)
        {
            return None;
        }

        Some(XbxEngineRecoveryAction::RequestReconnect(
            XbxEngineReconnectReasonDto::MediaStalled,
        ))
    }

    fn has_transport_hard_recovery_evidence(
        now_ms: f64,
        transport: &XbxEngineTransportSignal,
        keyframe_request_stall_ms: f64,
        recent_twcc_feedback: bool,
        recent_nack_expired: bool,
    ) -> bool {
        if recent_nack_expired {
            return true;
        }
        if transport
            .latest_video_packet_arrival_at_ms
            .map(|at_ms| now_ms - at_ms >= keyframe_request_stall_ms)
            .unwrap_or(true)
        {
            return true;
        }
        !recent_twcc_feedback
    }
}

#[cfg(test)]
#[path = "recovery.test.rs"]
mod tests;
