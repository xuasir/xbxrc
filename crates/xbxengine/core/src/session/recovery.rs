use xbxengine_protocol::{XbxEngineReconnectReasonDto, XbxEngineTransportStateDto};

pub const FIRST_FRAME_GRACE_MS: f64 = 8_000.0;
pub const KEYFRAME_REQUEST_STALL_MS: f64 = 1_500.0;
pub const DECODER_RESET_AFTER_KEYFRAME_WAIT_MS: f64 = 500.0;
pub const DECODER_RESET_REQUEST_COOLDOWN_MS: f64 = 1_500.0;
pub const RECONNECT_STALL_MS: f64 = 4_000.0;
pub const STALL_RECOVERY_COOLDOWN_MS: f64 = 6_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XbxEngineRecoveryPreset {
    CloudConservative,
    CloudAggressive,
    LanLowLatency,
}

impl Default for XbxEngineRecoveryPreset {
    fn default() -> Self {
        Self::CloudConservative
    }
}

impl XbxEngineRecoveryPreset {
    pub fn from_label(label: &str) -> Option<Self> {
        let normalized = label.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "cloudconservative" | "cloud-conservative" | "cloud_conservative" => {
                Some(Self::CloudConservative)
            }
            "cloudaggressive" | "cloud-aggressive" | "cloud_aggressive" => {
                Some(Self::CloudAggressive)
            }
            "lanlowlatency" | "lan-low-latency" | "lan_low_latency" | "lan" => {
                Some(Self::LanLowLatency)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XbxEngineRecoveryRuntimeConfig {
    pub first_frame_grace_ms: u64,
    pub keyframe_request_stall_ms: u64,
    pub decoder_reset_after_keyframe_wait_ms: u64,
    pub decoder_reset_request_cooldown_ms: u64,
    pub reconnect_stall_ms: u64,
    pub stall_recovery_cooldown_ms: u64,
}

impl Default for XbxEngineRecoveryRuntimeConfig {
    fn default() -> Self {
        Self::from_preset(XbxEngineRecoveryPreset::default())
    }
}

impl XbxEngineRecoveryRuntimeConfig {
    pub fn from_preset(preset: XbxEngineRecoveryPreset) -> Self {
        match preset {
            XbxEngineRecoveryPreset::CloudConservative => Self {
                first_frame_grace_ms: FIRST_FRAME_GRACE_MS as u64,
                keyframe_request_stall_ms: KEYFRAME_REQUEST_STALL_MS as u64,
                decoder_reset_after_keyframe_wait_ms: DECODER_RESET_AFTER_KEYFRAME_WAIT_MS as u64,
                decoder_reset_request_cooldown_ms: DECODER_RESET_REQUEST_COOLDOWN_MS as u64,
                reconnect_stall_ms: RECONNECT_STALL_MS as u64,
                stall_recovery_cooldown_ms: STALL_RECOVERY_COOLDOWN_MS as u64,
            },
            XbxEngineRecoveryPreset::CloudAggressive => Self {
                first_frame_grace_ms: 6_000,
                keyframe_request_stall_ms: 1_000,
                decoder_reset_after_keyframe_wait_ms: 350,
                decoder_reset_request_cooldown_ms: 1_000,
                reconnect_stall_ms: 2_800,
                stall_recovery_cooldown_ms: 4_000,
            },
            XbxEngineRecoveryPreset::LanLowLatency => Self {
                first_frame_grace_ms: 2_500,
                keyframe_request_stall_ms: 450,
                decoder_reset_after_keyframe_wait_ms: 150,
                decoder_reset_request_cooldown_ms: 450,
                reconnect_stall_ms: 1_400,
                stall_recovery_cooldown_ms: 2_000,
            },
        }
    }

    pub fn with_override(self, override_config: XbxEngineRecoveryRuntimeConfigOverride) -> Self {
        Self {
            first_frame_grace_ms: override_config
                .first_frame_grace_ms
                .unwrap_or(self.first_frame_grace_ms),
            keyframe_request_stall_ms: override_config
                .keyframe_request_stall_ms
                .unwrap_or(self.keyframe_request_stall_ms),
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
    pub keyframe_requested_for_current_stall: bool,
    pub decoder_reset_requested_for_current_stall: bool,
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
            keyframe_requested_for_current_stall: false,
            decoder_reset_requested_for_current_stall: false,
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
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct XbxEngineMediaSignal {
    pub latest_frame_rendered_at_ms: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct XbxEngineDecodeRenderSignal {
    // Phase-1 先占位，后续接 decoder/render stall 信号。
    pub decoder_stalled: Option<bool>,
    pub render_stalled: Option<bool>,
    pub allow_decoder_reset: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct XbxEngineRecoverySignals {
    pub transport: XbxEngineTransportSignal,
    pub media: XbxEngineMediaSignal,
    pub decode_render: XbxEngineDecodeRenderSignal,
}

impl XbxEngineRuntimeHealth {
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
            }
            XbxEngineTransportStateDto::Connecting => {
                self.connected_at_ms = None;
                self.decoder_reset_requested_for_current_stall = false;
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
    }

    pub fn restore_reconnect_marker(&mut self, reconnect_started_at_ms: f64) {
        self.last_reconnect_started_at_ms = Some(reconnect_started_at_ms);
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
            },
            media: XbxEngineMediaSignal {
                latest_frame_rendered_at_ms: self.last_frame_rendered_at_ms,
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

        let connected_at_ms = signals.transport.connected_at_ms.unwrap_or(now_ms);
        let activity_at_ms = signals
            .media
            .latest_frame_rendered_at_ms
            .or(signals.transport.latest_video_packet_arrival_at_ms)
            .unwrap_or(connected_at_ms);
        let stalled_for_ms = now_ms - activity_at_ms;

        if signals.media.latest_frame_rendered_at_ms.is_none()
            && signals
                .transport
                .latest_video_packet_arrival_at_ms
                .is_none()
            && now_ms - connected_at_ms < first_frame_grace_ms
        {
            return None;
        }

        let can_try_decoder_reset = signals.decode_render.allow_decoder_reset
            && signals.decode_render.decoder_stalled == Some(true)
            && signals.decode_render.render_stalled != Some(true);
        let should_request_keyframe = (stalled_for_ms >= keyframe_request_stall_ms
            || can_try_decoder_reset)
            && !self.keyframe_requested_for_current_stall
            && self
                .last_keyframe_request_at_ms
                .map(|last| now_ms - last >= keyframe_request_stall_ms)
                .unwrap_or(true);
        if should_request_keyframe {
            return Some(XbxEngineRecoveryAction::RequestVideoKeyframe);
        }
        let should_request_decoder_reset = can_try_decoder_reset
            && self.keyframe_requested_for_current_stall
            && !self.decoder_reset_requested_for_current_stall
            && self
                .last_keyframe_request_at_ms
                .map(|last| now_ms - last >= decoder_reset_after_keyframe_wait_ms)
                .unwrap_or(true)
            && self
                .last_decoder_reset_request_at_ms
                .map(|last| now_ms - last >= decoder_reset_request_cooldown_ms)
                .unwrap_or(true);
        if should_request_decoder_reset {
            return Some(XbxEngineRecoveryAction::RequestDecoderReset);
        }

        if stalled_for_ms < reconnect_stall_ms {
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
}

#[cfg(test)]
mod tests {
    use super::{
        XbxEngineDecodeRenderSignal, XbxEngineMediaSignal, XbxEngineRecoveryAction,
        XbxEngineRecoveryPreset, XbxEngineRecoveryRuntimeConfig,
        XbxEngineRecoveryRuntimeConfigOverride, XbxEngineRecoverySignals, XbxEngineRuntimeHealth,
        XbxEngineTransportSignal, DECODER_RESET_AFTER_KEYFRAME_WAIT_MS, KEYFRAME_REQUEST_STALL_MS,
        RECONNECT_STALL_MS,
    };

    #[test]
    fn recovery_signals_request_keyframe_before_reconnect() {
        let health = XbxEngineRuntimeHealth {
            connected_at_ms: Some(1_000.0),
            ..Default::default()
        };
        let action = health.next_recovery_action_with_signals(
            1_000.0 + KEYFRAME_REQUEST_STALL_MS + 10.0,
            true,
            XbxEngineRecoverySignals {
                transport: XbxEngineTransportSignal {
                    transport_connected: true,
                    connected_at_ms: Some(1_000.0),
                    latest_video_packet_arrival_at_ms: Some(1_000.0),
                },
                media: XbxEngineMediaSignal {
                    latest_frame_rendered_at_ms: Some(1_000.0),
                },
                decode_render: XbxEngineDecodeRenderSignal::default(),
            },
        );
        assert_eq!(action, Some(XbxEngineRecoveryAction::RequestVideoKeyframe));
    }

    #[test]
    fn recovery_signals_request_reconnect_after_extended_stall() {
        let health = XbxEngineRuntimeHealth {
            connected_at_ms: Some(1_000.0),
            keyframe_requested_for_current_stall: true,
            ..Default::default()
        };
        let action = health.next_recovery_action_with_signals(
            1_000.0 + RECONNECT_STALL_MS + 10.0,
            true,
            XbxEngineRecoverySignals {
                transport: XbxEngineTransportSignal {
                    transport_connected: true,
                    connected_at_ms: Some(1_000.0),
                    latest_video_packet_arrival_at_ms: Some(1_000.0),
                },
                media: XbxEngineMediaSignal {
                    latest_frame_rendered_at_ms: Some(1_000.0),
                },
                decode_render: XbxEngineDecodeRenderSignal::default(),
            },
        );
        assert!(matches!(
            action,
            Some(XbxEngineRecoveryAction::RequestReconnect(_))
        ));
    }

    #[test]
    fn recovery_signals_request_decoder_reset_after_keyframe_on_decode_stall() {
        let now_ms = 10_000.0;
        let request_keyframe = XbxEngineRuntimeHealth {
            connected_at_ms: Some(1_000.0),
            ..Default::default()
        }
        .next_recovery_action_with_signals(
            now_ms,
            true,
            XbxEngineRecoverySignals {
                transport: XbxEngineTransportSignal {
                    transport_connected: true,
                    connected_at_ms: Some(1_000.0),
                    latest_video_packet_arrival_at_ms: Some(now_ms - 50.0),
                },
                media: XbxEngineMediaSignal {
                    latest_frame_rendered_at_ms: Some(now_ms - 3_000.0),
                },
                decode_render: XbxEngineDecodeRenderSignal {
                    decoder_stalled: Some(true),
                    render_stalled: Some(false),
                    allow_decoder_reset: true,
                },
            },
        );
        assert_eq!(
            request_keyframe,
            Some(XbxEngineRecoveryAction::RequestVideoKeyframe)
        );

        let request_decoder_reset = XbxEngineRuntimeHealth {
            connected_at_ms: Some(1_000.0),
            last_keyframe_request_at_ms: Some(now_ms - DECODER_RESET_AFTER_KEYFRAME_WAIT_MS - 10.0),
            keyframe_requested_for_current_stall: true,
            ..Default::default()
        }
        .next_recovery_action_with_signals(
            now_ms,
            true,
            XbxEngineRecoverySignals {
                transport: XbxEngineTransportSignal {
                    transport_connected: true,
                    connected_at_ms: Some(1_000.0),
                    latest_video_packet_arrival_at_ms: Some(now_ms - 50.0),
                },
                media: XbxEngineMediaSignal {
                    latest_frame_rendered_at_ms: Some(now_ms - 3_000.0),
                },
                decode_render: XbxEngineDecodeRenderSignal {
                    decoder_stalled: Some(true),
                    render_stalled: Some(false),
                    allow_decoder_reset: true,
                },
            },
        );
        assert_eq!(
            request_decoder_reset,
            Some(XbxEngineRecoveryAction::RequestDecoderReset)
        );
    }

    #[test]
    fn recovery_config_override_changes_keyframe_trigger_threshold() {
        let health = XbxEngineRuntimeHealth {
            connected_at_ms: Some(1_000.0),
            ..Default::default()
        };
        let recovery_config = XbxEngineRecoveryRuntimeConfig {
            keyframe_request_stall_ms: 900,
            ..Default::default()
        };
        let action = health.next_recovery_action_with_signals_and_config(
            1_950.0,
            true,
            XbxEngineRecoverySignals {
                transport: XbxEngineTransportSignal {
                    transport_connected: true,
                    connected_at_ms: Some(1_000.0),
                    latest_video_packet_arrival_at_ms: Some(1_000.0),
                },
                media: XbxEngineMediaSignal {
                    latest_frame_rendered_at_ms: Some(1_000.0),
                },
                decode_render: XbxEngineDecodeRenderSignal::default(),
            },
            &recovery_config,
        );
        assert_eq!(action, Some(XbxEngineRecoveryAction::RequestVideoKeyframe));
    }

    #[test]
    fn recovery_presets_keep_expected_threshold_order() {
        let conservative =
            XbxEngineRecoveryRuntimeConfig::from_preset(XbxEngineRecoveryPreset::CloudConservative);
        let aggressive =
            XbxEngineRecoveryRuntimeConfig::from_preset(XbxEngineRecoveryPreset::CloudAggressive);
        let lan =
            XbxEngineRecoveryRuntimeConfig::from_preset(XbxEngineRecoveryPreset::LanLowLatency);

        assert!(conservative.keyframe_request_stall_ms > aggressive.keyframe_request_stall_ms);
        assert!(aggressive.keyframe_request_stall_ms > lan.keyframe_request_stall_ms);
        assert!(conservative.reconnect_stall_ms > aggressive.reconnect_stall_ms);
        assert!(aggressive.reconnect_stall_ms > lan.reconnect_stall_ms);
    }

    #[test]
    fn recovery_override_applies_partial_fields_only() {
        let base =
            XbxEngineRecoveryRuntimeConfig::from_preset(XbxEngineRecoveryPreset::CloudAggressive);
        let override_config = XbxEngineRecoveryRuntimeConfigOverride {
            reconnect_stall_ms: Some(5_000),
            ..Default::default()
        };
        let merged = base.with_override(override_config);
        assert_eq!(merged.reconnect_stall_ms, 5_000);
        assert_eq!(
            merged.keyframe_request_stall_ms,
            base.keyframe_request_stall_ms
        );
    }
}
