use std::sync::{Arc, Mutex};

use bytes::BytesMut;
use rtc::data_channel::RTCDataChannelInit;
use rtc::peer_connection::event::RTCDataChannelEvent;
use rtc::peer_connection::message::RTCMessage;
use rtc::sansio::Protocol;
use rtc_rtcp::transport_feedbacks::transport_layer_cc::TransportLayerCc;
use xbxengine_protocol::{
    build_xbox_stream_control_authorization_payload,
    build_xbox_stream_control_gamepad_changed_payload,
    build_xbox_stream_control_video_keyframe_requested_payload,
    build_xbox_stream_dimensions_changed_payload,
    build_xbox_stream_input_metadata_bootstrap_packet, build_xbox_stream_message_handshake_payload,
    build_xbox_stream_post_handshake_payloads, is_xbox_stream_message_handshake_ack,
    XBOX_STREAM_CHAT_CHANNEL_LABEL, XBOX_STREAM_CONTROL_CHANNEL_LABEL,
    XBOX_STREAM_DATA_CHANNEL_PROFILES, XBOX_STREAM_DEFAULT_VIEWPORT_HEIGHT,
    XBOX_STREAM_DEFAULT_VIEWPORT_WIDTH, XBOX_STREAM_INPUT_CHANNEL_LABEL,
    XBOX_STREAM_MESSAGE_CHANNEL_LABEL,
};

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::connection::builder::ControlledPeerConnection;
use crate::transport::rtc::connection::rumble::parse_rumble_requests;
use crate::transport::rtc::connection::short_text_preview;
use crate::transport::rtc::connection::transport_metrics::{
    build_twcc_observation, TWCC_OBSERVATION_SOURCE_REMOTE_RTCP,
};
use crate::transport::rtc::connection::RtcConnectionService;
use crate::transport::rtc::facts::{DataChannelLabelFact, PeerFact, TransportFact};
use crate::transport::rtc::stats::now_ms_f64;
use crate::transport::rtc::stream::{
    MediaPacketKind, RtcMediaIngressPacket, RtcMediaPacketSource, RtcRtpPacketMeta,
};
use crate::XbxEngineDataChannelMessageCatalogObservation;
use crate::{XbxEngineMediaRuntimeStats, XbxEngineRuntimeError};

const RTC_CONTROL_DELAYED_GAMEPAD_ADDED_MS: f64 = 500.0;
const RTC_CONTROL_DELAYED_PLI_PRIME_MS: f64 = 0.0;
const RTC_INPUT_BUFFERED_AMOUNT_HIGH_THRESHOLD_BYTES: u32 = 1024;
const RTC_INPUT_BUFFERED_AMOUNT_LOW_THRESHOLD_BYTES: u32 = 512;
pub(crate) const MESSAGE_CHANNEL_LABEL: &str = XBOX_STREAM_MESSAGE_CHANNEL_LABEL;
pub(crate) const CONTROL_CHANNEL_LABEL: &str = XBOX_STREAM_CONTROL_CHANNEL_LABEL;
pub(crate) const INPUT_CHANNEL_LABEL: &str = XBOX_STREAM_INPUT_CHANNEL_LABEL;
pub(crate) const CHAT_CHANNEL_LABEL: &str = XBOX_STREAM_CHAT_CHANNEL_LABEL;
const INPUT_METADATA_SEQ: u32 = 0;
const INPUT_METADATA_MAX_TOUCHPOINTS: u8 = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StreamViewportDimensions {
    pub width: u32,
    pub height: u32,
}

impl StreamViewportDimensions {
    fn sanitized(self) -> Self {
        Self {
            width: self.width.max(1),
            height: self.height.max(1),
        }
    }
}

impl Default for StreamViewportDimensions {
    fn default() -> Self {
        Self {
            width: XBOX_STREAM_DEFAULT_VIEWPORT_WIDTH,
            height: XBOX_STREAM_DEFAULT_VIEWPORT_HEIGHT,
        }
    }
}

// phase-1 先只把控制面 channel 拓扑建进 rtc，真正的 ready/handshake 由后续事件循环接管。
pub(crate) fn bootstrap_default_channels(
    peer_connection: &mut ControlledPeerConnection,
    state: &mut crate::transport::rtc::connection::runtime_state::RtcConnectionRuntimeState,
) -> Result<(), crate::XbxEngineRuntimeError> {
    for profile in XBOX_STREAM_DATA_CHANNEL_PROFILES {
        let label = profile.label;
        let channel = peer_connection
            .create_data_channel(
                label,
                Some(RTCDataChannelInit {
                    ordered: profile.ordered,
                    protocol: profile.protocol_name.to_string(),
                    ..Default::default()
                }),
            )
            .map_err(|err| {
                crate::XbxEngineRuntimeError::new(format!(
                    "xbxEngineRtcCreateDataChannelFailed({label}): {err}"
                ))
            })?;
        state
            .data_channel_labels
            .insert(channel.id(), channel.label().to_string());
    }
    Ok(())
}

pub(crate) fn build_message_handshake_payload() -> String {
    build_xbox_stream_message_handshake_payload()
}

pub(crate) fn build_post_handshake_message_payloads(
    viewport: StreamViewportDimensions,
) -> Vec<String> {
    let viewport = viewport.sanitized();
    build_xbox_stream_post_handshake_payloads(viewport.width, viewport.height)
}

pub(crate) fn build_dimensions_changed_message_payload(
    viewport: StreamViewportDimensions,
) -> String {
    let viewport = viewport.sanitized();
    build_xbox_stream_dimensions_changed_payload(viewport.width, viewport.height)
}

pub(crate) fn is_handshake_ack_payload(payload: &str) -> bool {
    is_xbox_stream_message_handshake_ack(payload)
}

pub(crate) fn build_control_decoder_reset_payload() -> String {
    serde_json::json!({
        "message": "decoderReset",
    })
    .to_string()
}

pub(crate) fn build_control_video_keyframe_requested_payload() -> String {
    build_xbox_stream_control_video_keyframe_requested_payload()
}

pub(crate) fn build_control_authorization_payload() -> String {
    build_xbox_stream_control_authorization_payload()
}

pub(crate) fn build_control_gamepad_changed_payload(added: bool) -> String {
    build_xbox_stream_control_gamepad_changed_payload(added)
}

pub(crate) fn build_input_metadata_bootstrap_packet() -> Vec<u8> {
    build_xbox_stream_input_metadata_bootstrap_packet(now_ms_f64(), INPUT_METADATA_MAX_TOUCHPOINTS)
}

fn build_local_text_catalog_observation(
    observation_id: u64,
    channel: String,
    payload: &str,
) -> XbxEngineDataChannelMessageCatalogObservation {
    let parsed = serde_json::from_str::<serde_json::Value>(payload).ok();
    let kind_type = parsed
        .as_ref()
        .and_then(|value| value.get("type"))
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let kind_message = parsed
        .as_ref()
        .and_then(|value| value.get("message"))
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let target = parsed
        .as_ref()
        .and_then(|value| value.get("target"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .or_else(|| Some(channel.clone()));
    let mut keys = parsed
        .as_ref()
        .and_then(|value| value.as_object())
        .map(|object| object.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_else(|| vec!["payload".to_string()]);
    keys.sort();

    XbxEngineDataChannelMessageCatalogObservation {
        observation_id,
        direction: "local".to_string(),
        channel,
        kind_type,
        kind_message,
        target,
        keys,
        payload_len: payload.len(),
        observed_at_ms: now_ms_f64(),
    }
}

pub(crate) fn build_input_metadata_packet(seq: u32, time: f64, max_touchpoints: u8) -> Vec<u8> {
    let mut packet = Vec::with_capacity(15);
    packet.extend_from_slice(&8u16.to_le_bytes());
    packet.extend_from_slice(&seq.to_le_bytes());
    packet.extend_from_slice(&time.to_le_bytes());
    packet.push(max_touchpoints);
    packet
}

impl RtcConnectionService {
    fn next_data_channel_catalog_observation_id(&mut self) -> u64 {
        self.data_channel_catalog_observation_id =
            self.data_channel_catalog_observation_id.saturating_add(1);
        self.data_channel_catalog_observation_id
    }

    pub(super) fn observe_control_replay_if_ready(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Some(actions) = self.control_service.peek_replay_actions_if_ready() else {
            return Ok(());
        };
        if actions.request_decoder_reset {
            self.send_control_payload(
                build_control_decoder_reset_payload(),
                "rtcControlReplayDecoderResetSent",
                "phase1 rtc control replay decoder reset sent",
                runtime_stats,
            )?;
            self.control_service.clear_pending_decoder_reset_request();
            self.sync_control_replay_runtime_stats(runtime_stats);
        }
        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.latest_observation_label = Some("rtcControlReplayConsumed".to_string());
            stats.latest_observation_summary = Some(format!(
                "phase1 rtc control replay consumed decoderReset={}",
                actions.request_decoder_reset
            ));
        });
        Ok(())
    }

    pub(super) fn try_send_message_handshake(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        if !self.control_service.should_send_message_handshake() {
            return Ok(());
        }
        let Some(channel_id) = self.data_channel_id_for_label(MESSAGE_CHANNEL_LABEL) else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcMessageChannelMissing",
            ));
        };
        self.send_text_on_channel_id(
            channel_id,
            build_message_handshake_payload(),
            "rtcMessageHandshakeSent",
            "phase1 rtc message handshake sent",
            runtime_stats,
        )
    }

    pub(super) fn send_post_handshake_messages(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        if !self.control_service.should_send_post_handshake_messages() {
            crate::xbx_log_debug!(
                "[xbxengine][rtc] post-handshake bootstrap skipped handshake_acked={} sent={}",
                self.control_service.state().message_handshake_acked,
                self.control_service.state().post_handshake_messages_sent
            );
            return Ok(());
        }
        let Some(channel_id) = self.data_channel_id_for_label(MESSAGE_CHANNEL_LABEL) else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcMessageChannelMissing",
            ));
        };
        let viewport = StreamViewportDimensions {
            width: self
                .webrtc_runtime_config
                .negotiation
                .target_resolution_width,
            height: self
                .webrtc_runtime_config
                .negotiation
                .target_resolution_height,
        };
        for payload in build_post_handshake_message_payloads(viewport) {
            crate::xbx_log_debug!(
                "[xbxengine][rtc] sending post-handshake payload channel_id={} len={}",
                channel_id,
                payload.len()
            );
            self.send_text_on_channel_id(
                channel_id,
                payload,
                "rtcMessagePostHandshakeSent",
                "phase1 rtc post-handshake message sent",
                runtime_stats,
            )?;
        }
        self.control_service.mark_post_handshake_messages_sent();
        Ok(())
    }

    pub(super) fn try_bootstrap_control_channel(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        if !self.control_service.can_bootstrap_control() {
            let state = self.control_service.state();
            crate::xbx_log_debug!(
                "[xbxengine][rtc] control bootstrap skipped control_open={} control_started={} handshake_acked={} bootstrapped_after_handshake={}",
                state.control_channel_open,
                state.control_started,
                state.message_handshake_acked,
                state.control_bootstrapped_after_handshake
            );
            return Ok(());
        }
        crate::xbx_log_debug!(
            "[xbxengine][rtc] control bootstrap starting control_open={} control_started={} handshake_acked={} bootstrapped_after_handshake={}",
            self.control_service.state().control_channel_open,
            self.control_service.state().control_started,
            self.control_service.state().message_handshake_acked,
            self.control_service.state().control_bootstrapped_after_handshake
        );
        self.send_control_payload(
            build_control_authorization_payload(),
            "rtcControlAuthorizationSent",
            "phase1 rtc control authorization sent",
            runtime_stats,
        )?;
        self.send_control_payload(
            build_control_gamepad_changed_payload(false),
            "rtcControlGamepadRemovedSent",
            "phase1 rtc control gamepad removed sent",
            runtime_stats,
        )?;
        self.control_service.mark_control_bootstrapped();
        crate::xbx_log_debug!(
            "[xbxengine][rtc] control bootstrap completed control_open={} control_started={} handshake_acked={} bootstrapped_after_handshake={}",
            self.control_service.state().control_channel_open,
            self.control_service.state().control_started,
            self.control_service.state().message_handshake_acked,
            self.control_service.state().control_bootstrapped_after_handshake
        );
        let now_ms = now_ms_f64();
        self.delayed_gamepad_added_due_at_ms = Some(now_ms + RTC_CONTROL_DELAYED_GAMEPAD_ADDED_MS);
        self.delayed_pli_prime_due_at_ms = Some(now_ms + RTC_CONTROL_DELAYED_PLI_PRIME_MS);
        Ok(())
    }

    pub(super) fn send_control_payload(
        &mut self,
        payload: String,
        observation_label: &str,
        observation_summary: &str,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Some(channel_id) = self.data_channel_id_for_label(CONTROL_CHANNEL_LABEL) else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcControlChannelMissing",
            ));
        };
        self.send_text_on_channel_id(
            channel_id,
            payload,
            observation_label,
            observation_summary,
            runtime_stats,
        )
    }

    pub(crate) fn control_keyframe_request_ready(&self) -> bool {
        self.control_service.is_control_ready()
    }

    pub(crate) fn request_video_keyframe_control_direct(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        if !self.control_keyframe_request_ready() {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcControlChannelNotReadyForVideoKeyframe",
            ));
        }
        self.send_control_payload(
            build_control_video_keyframe_requested_payload(),
            "rtcControlVideoKeyframeRequested",
            "phase1 rtc control video keyframe requested",
            runtime_stats,
        )
    }

    pub(super) fn send_text_on_channel_id(
        &mut self,
        channel_id: u16,
        payload: String,
        observation_label: &str,
        observation_summary: &str,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let channel_label = self
            .state
            .lock()
            .ok()
            .and_then(|state| state.data_channel_labels.get(&channel_id).cloned())
            .unwrap_or_else(|| format!("id:{channel_id}"));
        let catalog = build_local_text_catalog_observation(
            self.next_data_channel_catalog_observation_id(),
            channel_label,
            &payload,
        );
        let peer_connection = self
            .peer_connection
            .as_mut()
            .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineRtcPeerConnectionUnavailable"))?;
        let mut data_channel = peer_connection.data_channel(channel_id).ok_or_else(|| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcDataChannelUnavailable({channel_id})"))
        })?;
        data_channel.send_text(payload).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcDataChannelSendTextFailed: {err}"))
        })?;
        self.io_runtime.pump(peer_connection)?;
        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.latest_observation_label = Some(observation_label.to_string());
            stats.latest_observation_summary = Some(observation_summary.to_string());
            stats.latest_data_channel_message_catalog_observation = Some(catalog);
        });
        Ok(())
    }

    pub(super) fn send_binary_on_channel_id(
        &mut self,
        channel_id: u16,
        payload: Vec<u8>,
        observation_label: &str,
        observation_summary: &str,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let peer_connection = self
            .peer_connection
            .as_mut()
            .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineRtcPeerConnectionUnavailable"))?;
        let mut data_channel = peer_connection.data_channel(channel_id).ok_or_else(|| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcDataChannelUnavailable({channel_id})"))
        })?;
        data_channel
            .send(BytesMut::from(payload.as_slice()))
            .map_err(|err| {
                XbxEngineRuntimeError::new(format!(
                    "xbxEngineRtcDataChannelSendBinaryFailed: {err}"
                ))
            })?;
        self.io_runtime.pump(peer_connection)?;
        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.latest_observation_label = Some(observation_label.to_string());
            stats.latest_observation_summary = Some(observation_summary.to_string());
        });
        Ok(())
    }

    pub(crate) fn input_stream_ready(&self) -> bool {
        self.state.lock().ok().is_some_and(|state| {
            state.input_channel_open
                && state.input_metadata_bootstrapped
                && !state.input_backpressure_high
        })
    }

    pub(crate) fn send_input_stream_packet(
        &mut self,
        payload: Vec<u8>,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<bool, XbxEngineRuntimeError> {
        if !self.input_stream_ready() {
            return Ok(false);
        }
        let Some(channel_id) = self.data_channel_id_for_label(INPUT_CHANNEL_LABEL) else {
            return Ok(false);
        };
        let packet_len = payload.len();
        let seq = if payload.len() >= 6 {
            let mut seq_bytes = [0u8; 4];
            seq_bytes.copy_from_slice(&payload[2..6]);
            u32::from_le_bytes(seq_bytes)
        } else {
            0
        };
        let summary = format!("phase1 rtc input stream packet sent seq={seq} bytes={packet_len}");
        self.send_binary_on_channel_id(
            channel_id,
            payload,
            "rtcInputStreamPacketSent",
            &summary,
            runtime_stats,
        )?;
        Ok(true)
    }

    pub(super) fn try_bootstrap_input_channel(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Some(channel_id) = self.data_channel_id_for_label(INPUT_CHANNEL_LABEL) else {
            return Err(XbxEngineRuntimeError::new(
                "xbxEngineRtcInputChannelMissing",
            ));
        };
        let handshake_acked = self.control_service.state().message_handshake_acked;
        let should_send = {
            let Ok(mut state) = self.state.lock() else {
                return Ok(());
            };
            if !state.input_channel_open {
                crate::xbx_log_debug!(
                    "[xbxengine][rtc] input bootstrap skipped because input channel is not open"
                );
                false
            } else {
                let should_send_pre_handshake = !state.input_metadata_bootstrapped;
                let should_send_post_handshake =
                    handshake_acked && !state.input_metadata_bootstrapped_after_handshake;
                if should_send_pre_handshake || should_send_post_handshake {
                    crate::xbx_log_debug!(
                        "[xbxengine][rtc] input bootstrap starting channel_open={} handshake_acked={} bootstrapped={} bootstrapped_after_handshake={}",
                        state.input_channel_open,
                        handshake_acked,
                        state.input_metadata_bootstrapped,
                        state.input_metadata_bootstrapped_after_handshake
                    );
                    state.input_metadata_bootstrapped = true;
                    if handshake_acked {
                        state.input_metadata_bootstrapped_after_handshake = true;
                    }
                    true
                } else {
                    crate::xbx_log_debug!(
                        "[xbxengine][rtc] input bootstrap skipped channel_open={} handshake_acked={} bootstrapped={} bootstrapped_after_handshake={}",
                        state.input_channel_open,
                        handshake_acked,
                        state.input_metadata_bootstrapped,
                        state.input_metadata_bootstrapped_after_handshake
                    );
                    false
                }
            }
        };
        if !should_send {
            return Ok(());
        }

        let packet = build_input_metadata_bootstrap_packet();
        let packet_len = packet.len();
        let summary = format!(
            "phase1 rtc input metadata bootstrap sent seq=0 maxTouchpoints=64 bytes={packet_len}"
        );
        crate::xbx_log_debug!(
            "[xbxengine][rtc] sending input metadata bootstrap channel_id={} bytes={}",
            channel_id,
            packet_len
        );
        match self.send_binary_on_channel_id(
            channel_id,
            packet,
            "rtcInputMetadataBootstrapSent",
            &summary,
            runtime_stats,
        ) {
            Ok(()) => Ok(()),
            Err(error) => {
                if let Ok(mut state) = self.state.lock() {
                    state.input_metadata_bootstrapped = false;
                    if handshake_acked {
                        state.input_metadata_bootstrapped_after_handshake = false;
                    }
                }
                Err(error)
            }
        }
    }

    pub(super) fn publish_channel_lifecycle(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        label: &str,
        summary: &str,
    ) {
        let channel = if label.contains("Control") {
            "control"
        } else if label.contains("Message") {
            "message"
        } else if label.contains("Input") {
            "input"
        } else if label.contains("Chat") {
            "chat"
        } else {
            "unknown"
        };
        let lifecycle = if label.contains("Opened") {
            "open"
        } else if label.contains("Closed") {
            "close"
        } else {
            "lifecycle"
        };
        let observation_id = self.next_data_channel_catalog_observation_id();
        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.latest_observation_label = Some(label.to_string());
            stats.latest_observation_summary = Some(summary.to_string());
            stats.latest_data_channel_message_catalog_observation =
                Some(XbxEngineDataChannelMessageCatalogObservation {
                    observation_id,
                    direction: "local".to_string(),
                    channel: channel.to_string(),
                    kind_type: Some("lifecycle".to_string()),
                    kind_message: Some(lifecycle.to_string()),
                    target: Some(channel.to_string()),
                    keys: vec!["channel".to_string(), "state".to_string()],
                    payload_len: 0,
                    observed_at_ms: now_ms_f64(),
                });
        });
    }

    pub(super) fn run_delayed_control_actions(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let now_ms = now_ms_f64();
        if self
            .delayed_gamepad_added_due_at_ms
            .is_some_and(|deadline| now_ms >= deadline)
        {
            self.delayed_gamepad_added_due_at_ms = None;
            self.send_control_payload(
                build_control_gamepad_changed_payload(true),
                "rtcControlGamepadAddedSent",
                "phase1 rtc control gamepad added sent",
                runtime_stats,
            )?;
        }
        if self
            .delayed_pli_prime_due_at_ms
            .is_some_and(|deadline| now_ms >= deadline)
        {
            self.delayed_pli_prime_due_at_ms = None;
            let outcome = self.request_video_pli_with_outcome(runtime_stats)?;
            if matches!(
                outcome,
                crate::transport::rtc::connection::VideoRecoveryRequestOutcome::FeedbackTransportNotReady
                    | crate::transport::rtc::connection::VideoRecoveryRequestOutcome::FeedbackTargetPending
            ) {
                RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
                    stats.latest_observation_label =
                        Some("rtcDelayedPliPrimeFeedbackTargetPending".to_string());
                    stats.latest_observation_summary = Some(
                        "phase1 rtc delayed pli prime deferred until feedback target ready"
                            .to_string(),
                    );
                });
            }
        }
        Ok(())
    }

    pub(super) fn record_chat_text_observation(
        &self,
        observation_id: u64,
        payload_text: &str,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) {
        let preview = short_text_preview(payload_text, 48);
        RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
            stats.latest_data_channel_message_catalog_observation =
                Some(XbxEngineDataChannelMessageCatalogObservation {
                    observation_id,
                    direction: "inbound".to_string(),
                    channel: "chat".to_string(),
                    kind_type: None,
                    kind_message: Some("text".to_string()),
                    target: Some("chat".to_string()),
                    keys: vec!["text".to_string()],
                    payload_len: payload_text.len(),
                    observed_at_ms: now_ms_f64(),
                });
            stats.latest_observation_label = Some("rtcChatTextObserved".to_string());
            stats.latest_observation_summary = Some(format!(
                "phase1 rtc chat text observed len={} preview={preview:?}",
                payload_text.len()
            ));
        });
    }

    pub(super) fn apply_data_channel_event(
        &mut self,
        event: RTCDataChannelEvent,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        match event {
            RTCDataChannelEvent::OnOpen(channel_id) => {
                let label = self
                    .state
                    .lock()
                    .ok()
                    .and_then(|state| state.data_channel_labels.get(&channel_id).cloned());
                match label.as_deref() {
                    Some(CONTROL_CHANNEL_LABEL) => {
                        self.control_service.open_control_channel();
                        self.publish_channel_lifecycle(
                            runtime_stats,
                            "rtcControlChannelOpened",
                            "phase1 rtc control channel opened",
                        );
                        self.try_bootstrap_control_channel(runtime_stats)?;
                    }
                    Some(MESSAGE_CHANNEL_LABEL) => {
                        self.control_service.open_message_channel();
                        self.publish_channel_lifecycle(
                            runtime_stats,
                            "rtcMessageChannelOpened",
                            "phase1 rtc message channel opened",
                        );
                        self.try_send_message_handshake(runtime_stats)?;
                    }
                    Some(INPUT_CHANNEL_LABEL) => {
                        if let Ok(mut state) = self.state.lock() {
                            state.input_channel_open = true;
                            state.input_backpressure_high = false;
                        }
                        if let Some(peer_connection) = self.peer_connection.as_mut() {
                            if let Some(mut data_channel) = peer_connection.data_channel(channel_id)
                            {
                                data_channel.set_buffered_amount_high_threshold(
                                    RTC_INPUT_BUFFERED_AMOUNT_HIGH_THRESHOLD_BYTES,
                                );
                                data_channel.set_buffered_amount_low_threshold(
                                    RTC_INPUT_BUFFERED_AMOUNT_LOW_THRESHOLD_BYTES,
                                );
                            }
                        }
                        self.publish_channel_lifecycle(
                            runtime_stats,
                            "rtcInputChannelOpened",
                            "phase1 rtc input channel opened",
                        );
                        self.try_bootstrap_input_channel(runtime_stats)?;
                    }
                    Some(CHAT_CHANNEL_LABEL) => {
                        if let Ok(mut state) = self.state.lock() {
                            state.chat_channel_open = true;
                        }
                        self.publish_channel_lifecycle(
                            runtime_stats,
                            "rtcChatChannelOpened",
                            "phase1 rtc chat channel opened",
                        );
                    }
                    _ => {}
                }
                if let Some(label) = label
                    .as_deref()
                    .and_then(crate::transport::rtc::connection::map_data_channel_label_fact)
                {
                    self.push_transport_fact(TransportFact::Peer(PeerFact::DataChannelOpened {
                        label,
                        observed_at_ms: now_ms_f64(),
                    }));
                }
                self.observe_control_replay_if_ready(runtime_stats)?;
            }
            RTCDataChannelEvent::OnClose(channel_id) => {
                let label = self
                    .state
                    .lock()
                    .ok()
                    .and_then(|state| state.data_channel_labels.get(&channel_id).cloned());
                match label.as_deref() {
                    Some(CONTROL_CHANNEL_LABEL) => {
                        self.control_service.close_control_channel();
                        self.delayed_gamepad_added_due_at_ms = None;
                        self.delayed_pli_prime_due_at_ms = None;
                        self.raise_disconnect_signal(
                            runtime_stats,
                            "rtcControlChannelClosed",
                            "phase1 rtc control channel closed",
                            "control channel closed",
                        );
                    }
                    Some(MESSAGE_CHANNEL_LABEL) => {
                        self.control_service.close_message_channel();
                        if let Ok(mut state) = self.state.lock() {
                            state.input_channel_open = false;
                            state.input_metadata_bootstrapped = false;
                            state.input_metadata_bootstrapped_after_handshake = false;
                            state.input_backpressure_high = false;
                            state.chat_channel_open = false;
                        }
                        self.delayed_gamepad_added_due_at_ms = None;
                        self.delayed_pli_prime_due_at_ms = None;
                        self.raise_disconnect_signal(
                            runtime_stats,
                            "rtcMessageChannelClosed",
                            "phase1 rtc message channel closed",
                            "message channel closed",
                        );
                    }
                    Some(INPUT_CHANNEL_LABEL) => {
                        if let Ok(mut state) = self.state.lock() {
                            state.input_channel_open = false;
                            state.input_metadata_bootstrapped = false;
                            state.input_metadata_bootstrapped_after_handshake = false;
                            state.input_backpressure_high = false;
                        }
                        self.publish_channel_lifecycle(
                            runtime_stats,
                            "rtcInputChannelClosed",
                            "phase1 rtc input channel closed",
                        );
                    }
                    Some(CHAT_CHANNEL_LABEL) => {
                        if let Ok(mut state) = self.state.lock() {
                            state.chat_channel_open = false;
                        }
                        self.publish_channel_lifecycle(
                            runtime_stats,
                            "rtcChatChannelClosed",
                            "phase1 rtc chat channel closed",
                        );
                    }
                    _ => {}
                }
                if let Some(label) = label
                    .as_deref()
                    .and_then(crate::transport::rtc::connection::map_data_channel_label_fact)
                {
                    self.push_transport_fact(TransportFact::Peer(PeerFact::DataChannelClosed {
                        label,
                        observed_at_ms: now_ms_f64(),
                    }));
                }
            }
            RTCDataChannelEvent::OnBufferedAmountHigh(channel_id) => {
                let label = self
                    .state
                    .lock()
                    .ok()
                    .and_then(|state| state.data_channel_labels.get(&channel_id).cloned());
                if label.as_deref() == Some(INPUT_CHANNEL_LABEL) {
                    if let Ok(mut state) = self.state.lock() {
                        state.input_backpressure_high = true;
                    }
                    self.publish_channel_lifecycle(
                        runtime_stats,
                        "rtcInputBackpressureHigh",
                        "phase1 rtc input channel buffered amount high",
                    );
                    self.push_transport_fact(TransportFact::Peer(
                        PeerFact::DataChannelBufferedAmountHigh {
                            label: DataChannelLabelFact::Input,
                            observed_at_ms: now_ms_f64(),
                        },
                    ));
                }
            }
            RTCDataChannelEvent::OnBufferedAmountLow(channel_id) => {
                let label = self
                    .state
                    .lock()
                    .ok()
                    .and_then(|state| state.data_channel_labels.get(&channel_id).cloned());
                if label.as_deref() == Some(INPUT_CHANNEL_LABEL) {
                    if let Ok(mut state) = self.state.lock() {
                        state.input_backpressure_high = false;
                    }
                    self.publish_channel_lifecycle(
                        runtime_stats,
                        "rtcInputBackpressureLow",
                        "phase1 rtc input channel buffered amount low",
                    );
                    self.push_transport_fact(TransportFact::Peer(
                        PeerFact::DataChannelBufferedAmountLow {
                            label: DataChannelLabelFact::Input,
                            observed_at_ms: now_ms_f64(),
                        },
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn drain_peer_reads_core(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let Some(peer_connection) = self.peer_connection.as_mut() else {
            return Ok(());
        };
        let mut pending_reads = Vec::new();
        while let Some(message) = peer_connection.poll_read() {
            pending_reads.push(message);
        }
        let mut changed = false;
        let mut should_ack_message_handshake = false;
        let mut chat_text_observations = Vec::new();
        for message in pending_reads {
            match message {
                RTCMessage::RtpPacket(track_id, packet) => {
                    let remote_answer_sdp = self
                        .state
                        .lock()
                        .ok()
                        .and_then(|state| state.remote_answer_sdp.clone());
                    let fallback_mime_type =
                        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
                            stats
                                .latest_video_track_status
                                .as_ref()
                                .and_then(|status| status.mime_type.clone())
                        })
                        .flatten();
                    self.controlled_twcc_feedback.observe_inbound_rtp(
                        &track_id,
                        &packet,
                        runtime_stats,
                        remote_answer_sdp.as_deref(),
                        fallback_mime_type,
                    )?;
                    self.read_counters.rtp_packets =
                        self.read_counters.rtp_packets.saturating_add(1);
                    self.pending_media_ingress_packets.push((
                        RtcMediaIngressPacket::new(
                            MediaPacketKind::Rtp,
                            packet.payload.len(),
                            RtcMediaPacketSource::Track {
                                track_id: format!("{track_id:?}"),
                            },
                        )
                        .with_rtp_payload(packet.payload.to_vec()),
                        Some(RtcRtpPacketMeta {
                            ssrc: packet.header.ssrc,
                            payload_type: packet.header.payload_type,
                            sequence_number: packet.header.sequence_number,
                            timestamp: packet.header.timestamp,
                            marker: packet.header.marker,
                        }),
                    ));
                    changed = true;
                }
                RTCMessage::RtcpPacket(track_id, packets) => {
                    self.read_counters.rtcp_packets =
                        self.read_counters.rtcp_packets.saturating_add(1);
                    for packet in packets.iter() {
                        let Some(twcc) = packet.as_any().downcast_ref::<TransportLayerCc>() else {
                            continue;
                        };
                        self.remote_rtcp_twcc_observation_id =
                            self.remote_rtcp_twcc_observation_id.saturating_add(1);
                        if let Some(observation) = build_twcc_observation(
                            self.remote_rtcp_twcc_observation_id,
                            twcc,
                            runtime_stats,
                            TWCC_OBSERVATION_SOURCE_REMOTE_RTCP,
                        ) {
                            RuntimeStatsSink::new(runtime_stats.clone())
                                .record_latest_video_twcc_observation(observation);
                        }
                    }
                    let byte_len = packets.iter().map(|packet| packet.marshal_size()).sum();
                    self.pending_media_ingress_packets.push((
                        RtcMediaIngressPacket::new(
                            MediaPacketKind::Rtcp,
                            byte_len,
                            RtcMediaPacketSource::Track {
                                track_id: format!("{track_id:?}"),
                            },
                        ),
                        None,
                    ));
                    changed = true;
                }
                RTCMessage::DataChannelMessage(channel_id, payload) => {
                    self.read_counters.data_channel_messages =
                        self.read_counters.data_channel_messages.saturating_add(1);
                    let last_label = self
                        .state
                        .lock()
                        .ok()
                        .and_then(|state| state.data_channel_labels.get(&channel_id).cloned())
                        .unwrap_or_else(|| format!("id:{channel_id}"));
                    self.read_counters.last_data_channel_label = Some(last_label.clone());
                    if last_label == MESSAGE_CHANNEL_LABEL && payload.is_string {
                        let payload_text = String::from_utf8_lossy(payload.data.as_ref());
                        let preview = short_text_preview(payload_text.as_ref(), 96);
                        let is_handshake_ack = is_handshake_ack_payload(payload_text.as_ref());
                        if !is_handshake_ack {
                            crate::xbx_log_warn!(
                                "[xbxengine][rtc] inbound message payload observed observation_id={} len={} preview={preview:?}",
                                self.read_counters.data_channel_messages,
                                payload_text.len()
                            );
                        }
                        if is_handshake_ack {
                            should_ack_message_handshake = true;
                        }
                        if payload_text.contains("KickForClosedGame") {
                            RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
                                stats.latest_observation_label =
                                    Some("rtcSessionKickedForClosedGame".to_string());
                                stats.latest_observation_summary = Some(format!(
                                    "phase1 rtc inbound message kick reason=KickForClosedGame observationId={}",
                                    self.read_counters.data_channel_messages
                                ));
                            });
                        }
                    } else if last_label == CHAT_CHANNEL_LABEL && payload.is_string {
                        let payload_text =
                            String::from_utf8_lossy(payload.data.as_ref()).to_string();
                        chat_text_observations
                            .push((self.read_counters.data_channel_messages, payload_text));
                    } else if last_label == INPUT_CHANNEL_LABEL && !payload.is_string {
                        let requests = parse_rumble_requests(payload.data.as_ref());
                        if !requests.is_empty() {
                            crate::xbx_log_warn!(
                                "[xbxengine][rtc] inbound input rumble observed observation_id={} requests={}",
                                self.read_counters.data_channel_messages,
                                requests.len()
                            );
                            self.enqueue_pending_gamepad_rumble_requests(requests);
                        }
                    }
                    changed = true;
                }
            }
        }
        if let Some(peer_connection) = self.peer_connection.as_mut() {
            self.controlled_twcc_feedback
                .flush_due_feedback(peer_connection, runtime_stats)?;
        }
        if should_ack_message_handshake {
            let first_ack = self.control_service.ack_handshake();
            if first_ack {
                crate::xbx_log_debug!(
                    "[xbxengine][rtc] inbound message handshake ack observed observation_id={} firstAck=true",
                    self.read_counters.data_channel_messages
                );
                RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
                    if stats.message_handshake_acked_at_ms.is_none() {
                        stats.message_handshake_acked_at_ms = Some(now_ms_f64());
                    }
                });
                self.send_post_handshake_messages(runtime_stats)?;
            } else {
                crate::xbx_log_debug!(
                    "[xbxengine][rtc] inbound message handshake ack observed observation_id={} firstAck=false",
                    self.read_counters.data_channel_messages
                );
            }
            self.try_bootstrap_control_channel(runtime_stats)?;
            self.try_bootstrap_input_channel(runtime_stats)?;
            self.observe_control_replay_if_ready(runtime_stats)?;
            if self.control_service.is_control_ready() {
                RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
                    if stats.control_ready_at_ms.is_none() {
                        stats.control_ready_at_ms = Some(now_ms_f64());
                    }
                });
            }
        }
        if changed {
            let last_label = self
                .read_counters
                .last_data_channel_label
                .clone()
                .unwrap_or_else(|| "none".to_string());
            let ingress_catalog_observation_id = if self.read_counters.data_channel_messages > 0 {
                Some(self.next_data_channel_catalog_observation_id())
            } else {
                None
            };
            RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
                stats.latest_observation_label = Some("rtcReadIngressObserved".to_string());
                stats.latest_observation_summary = Some(format!(
                    "phase1 rtc read ingress rtp={} rtcp={} dc={} lastDc={}",
                    self.read_counters.rtp_packets,
                    self.read_counters.rtcp_packets,
                    self.read_counters.data_channel_messages,
                    last_label
                ));
                if let Some(observation_id) = ingress_catalog_observation_id {
                    stats.latest_data_channel_message_catalog_observation =
                        Some(XbxEngineDataChannelMessageCatalogObservation {
                            observation_id,
                            direction: "inbound".to_string(),
                            channel: last_label.clone(),
                            kind_type: Some("ingress".to_string()),
                            kind_message: Some("message".to_string()),
                            target: Some(last_label.clone()),
                            keys: vec!["channel".to_string()],
                            payload_len: 0,
                            observed_at_ms: now_ms_f64(),
                        });
                }
            });
            for (observation_id, payload_text) in chat_text_observations {
                self.record_chat_text_observation(observation_id, &payload_text, runtime_stats);
            }
        }
        // 同一轮 read 内可能刚完成握手 / bootstrap / 首包 RTP，补一次 pending 控制面重放。
        self.observe_control_replay_if_ready(runtime_stats)?;
        Ok(())
    }

    pub(super) fn data_channel_id_for_label(&self, label: &str) -> Option<u16> {
        self.state.lock().ok().and_then(|state| {
            state
                .data_channel_labels
                .iter()
                .find_map(|(channel_id, channel_label)| {
                    (channel_label == label).then_some(*channel_id)
                })
        })
    }
}
