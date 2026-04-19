use crate::XbxEngineRuntimeError;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RtcControlReplayActions {
    pub(crate) request_keyframe: bool,
    pub(crate) request_decoder_reset: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RtcControlChannelState {
    pub(crate) message_channel_open: bool,
    pub(crate) message_handshake_acked: bool,
    pub(crate) message_handshake_pending: bool,
    pub(crate) post_handshake_messages_sent: bool,
    pub(crate) control_channel_open: bool,
    pub(crate) control_started: bool,
    pub(crate) control_bootstrapped_after_handshake: bool,
    pub(crate) pending_keyframe_request: bool,
    pub(crate) pending_decoder_reset: bool,
    pub(crate) pending_replay_since_ms: Option<f64>,
    pub(crate) keyboard_pointer_enabled: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RtcControlChannelService {
    state: RtcControlChannelState,
}

impl RtcControlChannelService {
    pub(crate) fn reset(&mut self) {
        let pending_keyframe_request = self.state.pending_keyframe_request;
        let pending_decoder_reset = self.state.pending_decoder_reset;
        let pending_replay_since_ms = self.state.pending_replay_since_ms;
        let keyboard_pointer_enabled = self.state.keyboard_pointer_enabled;
        self.state = RtcControlChannelState::default();
        self.state.pending_keyframe_request = pending_keyframe_request;
        self.state.pending_decoder_reset = pending_decoder_reset;
        self.state.pending_replay_since_ms = pending_replay_since_ms;
        self.state.keyboard_pointer_enabled = keyboard_pointer_enabled;
    }

    pub(crate) fn clear_pending_replay_actions(&mut self) {
        self.state.pending_keyframe_request = false;
        self.state.pending_decoder_reset = false;
        self.state.pending_replay_since_ms = None;
    }

    pub(crate) fn clear_pending_keyframe_request(&mut self) {
        self.state.pending_keyframe_request = false;
        self.refresh_pending_replay_since();
    }

    pub(crate) fn clear_pending_decoder_reset_request(&mut self) {
        self.state.pending_decoder_reset = false;
        self.refresh_pending_replay_since();
    }

    pub(crate) fn open_message_channel(&mut self) {
        self.state.message_channel_open = true;
        self.state.message_handshake_pending = true;
    }

    pub(crate) fn close_message_channel(&mut self) {
        self.state.message_channel_open = false;
        self.state.message_handshake_acked = false;
        self.state.message_handshake_pending = false;
        self.state.post_handshake_messages_sent = false;
        self.state.control_started = false;
        self.state.control_bootstrapped_after_handshake = false;
    }

    pub(crate) fn open_control_channel(&mut self) {
        self.state.control_channel_open = true;
    }

    pub(crate) fn close_control_channel(&mut self) {
        self.state.control_channel_open = false;
        self.state.control_started = false;
        self.state.control_bootstrapped_after_handshake = false;
    }

    pub(crate) fn ack_handshake(&mut self) -> bool {
        let first_ack = !self.state.message_handshake_acked;
        self.state.message_handshake_acked = true;
        self.state.message_handshake_pending = false;
        first_ack
    }

    pub(crate) fn should_send_message_handshake(&self) -> bool {
        self.state.message_channel_open
            && self.state.message_handshake_pending
            && !self.state.message_handshake_acked
    }

    pub(crate) fn should_send_post_handshake_messages(&self) -> bool {
        self.state.message_handshake_acked && !self.state.post_handshake_messages_sent
    }

    pub(crate) fn mark_post_handshake_messages_sent(&mut self) {
        self.state.post_handshake_messages_sent = true;
    }

    pub(crate) fn can_bootstrap_control(&self) -> bool {
        self.state.control_channel_open
            && (!self.state.control_started
                || (self.state.message_handshake_acked
                    && !self.state.control_bootstrapped_after_handshake))
    }

    pub(crate) fn mark_control_bootstrapped(&mut self) {
        self.state.control_started = true;
        if self.state.message_handshake_acked {
            self.state.control_bootstrapped_after_handshake = true;
        }
    }

    pub(crate) fn set_keyboard_pointer_enabled(&mut self, enabled: bool) {
        self.state.keyboard_pointer_enabled = enabled;
    }

    pub(crate) fn request_video_keyframe(&mut self) -> Result<(), XbxEngineRuntimeError> {
        if self.is_control_ready() {
            self.state.pending_keyframe_request = false;
            self.refresh_pending_replay_since();
            return Ok(());
        }
        self.state.pending_keyframe_request = true;
        self.mark_pending_replay();
        Err(XbxEngineRuntimeError::new(
            "xbxEngineRtcControlChannelNotReadyForKeyframe",
        ))
    }

    pub(crate) fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        if self.is_control_ready() {
            self.state.pending_decoder_reset = false;
            self.refresh_pending_replay_since();
            return Ok(());
        }
        self.state.pending_decoder_reset = true;
        self.mark_pending_replay();
        Err(XbxEngineRuntimeError::new(
            "xbxEngineRtcControlChannelNotReadyForDecoderReset",
        ))
    }

    pub(crate) fn is_control_ready(&self) -> bool {
        self.state.control_channel_open
            && self.state.message_handshake_acked
            && self.state.control_bootstrapped_after_handshake
    }

    pub(crate) fn has_pending_replay_actions(&self) -> bool {
        self.state.pending_keyframe_request || self.state.pending_decoder_reset
    }

    pub(crate) fn pending_replay_action_count(&self) -> u8 {
        self.state.pending_keyframe_request as u8 + self.state.pending_decoder_reset as u8
    }

    pub(crate) fn pending_replay_since_ms(&self) -> Option<f64> {
        self.state.pending_replay_since_ms
    }

    pub(crate) fn peek_replay_actions_if_ready(&self) -> Option<RtcControlReplayActions> {
        if !self.is_control_ready() {
            return None;
        }

        let actions = RtcControlReplayActions {
            request_keyframe: self.state.pending_keyframe_request,
            request_decoder_reset: self.state.pending_decoder_reset,
        };
        if actions == RtcControlReplayActions::default() {
            return None;
        }

        Some(actions)
    }

    #[cfg(test)]
    pub(crate) fn take_replay_actions_if_ready(&mut self) -> Option<RtcControlReplayActions> {
        let actions = self.peek_replay_actions_if_ready()?;
        // 控制面可执行后，待回放请求只消费一次，避免重复下发。
        self.clear_pending_replay_actions();
        Some(actions)
    }

    pub(crate) fn state(&self) -> &RtcControlChannelState {
        &self.state
    }

    fn mark_pending_replay(&mut self) {
        if self.state.pending_replay_since_ms.is_none() {
            self.state.pending_replay_since_ms = Some(crate::transport::rtc::stats::now_ms_f64());
        }
    }

    fn refresh_pending_replay_since(&mut self) {
        if !self.has_pending_replay_actions() {
            self.state.pending_replay_since_ms = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RtcControlChannelService, RtcControlReplayActions};

    #[test]
    fn handshake_and_control_open_together_make_service_ready() {
        let mut service = RtcControlChannelService::default();

        service.open_message_channel();
        service.ack_handshake();
        assert!(!service.is_control_ready());

        service.open_control_channel();
        assert!(!service.is_control_ready());

        service.mark_control_bootstrapped();
        assert!(service.is_control_ready());
    }

    #[test]
    fn request_before_ready_is_queued_for_replay() {
        let mut service = RtcControlChannelService::default();

        assert!(service.request_video_keyframe().is_err());
        assert!(service.request_decoder_reset().is_err());
        assert!(service.has_pending_replay_actions());

        service.open_message_channel();
        service.open_control_channel();
        assert!(service.peek_replay_actions_if_ready().is_none());

        service.ack_handshake();
        service.mark_control_bootstrapped();
        let actions = service.peek_replay_actions_if_ready();
        assert_eq!(
            actions,
            Some(RtcControlReplayActions {
                request_keyframe: true,
                request_decoder_reset: true,
            })
        );
        assert!(service.has_pending_replay_actions());
    }

    #[test]
    fn replay_actions_are_consumed_only_once() {
        let mut service = RtcControlChannelService::default();
        service.request_video_keyframe().unwrap_err();
        service.open_message_channel();
        service.open_control_channel();
        service.ack_handshake();
        service.mark_control_bootstrapped();

        let first = service.take_replay_actions_if_ready();
        assert_eq!(
            first,
            Some(RtcControlReplayActions {
                request_keyframe: true,
                request_decoder_reset: false,
            })
        );
        assert_eq!(service.peek_replay_actions_if_ready(), None);
    }

    #[test]
    fn request_when_ready_does_not_leave_pending_state() {
        let mut service = RtcControlChannelService::default();
        service.open_message_channel();
        service.open_control_channel();
        service.ack_handshake();
        service.mark_control_bootstrapped();

        assert!(service.request_video_keyframe().is_ok());
        assert!(service.request_decoder_reset().is_ok());
        assert!(!service.has_pending_replay_actions());
        assert_eq!(service.peek_replay_actions_if_ready(), None);
    }

    #[test]
    fn close_control_channel_keeps_handshake_but_blocks_ready() {
        let mut service = RtcControlChannelService::default();
        service.open_message_channel();
        service.open_control_channel();
        service.ack_handshake();
        service.mark_control_bootstrapped();
        assert!(service.is_control_ready());

        service.close_control_channel();
        assert!(!service.is_control_ready());
        assert!(service.state().message_handshake_acked);
    }

    #[test]
    fn startup_fallback_is_not_treated_as_ready_for_recovery_actions() {
        let mut service = RtcControlChannelService::default();
        service.open_control_channel();
        service.mark_control_bootstrapped();
        assert!(!service.is_control_ready());
        assert!(service.request_video_keyframe().is_err());
        assert!(service.request_decoder_reset().is_err());
    }

    #[test]
    fn closing_message_channel_clears_handshake_and_bootstrap_state() {
        let mut service = RtcControlChannelService::default();
        service.open_message_channel();
        service.open_control_channel();
        service.ack_handshake();
        service.mark_post_handshake_messages_sent();
        service.mark_control_bootstrapped();

        service.close_message_channel();

        assert!(!service.state().message_channel_open);
        assert!(!service.state().message_handshake_acked);
        assert!(!service.state().post_handshake_messages_sent);
        assert!(!service.state().control_started);
        assert!(!service.state().control_bootstrapped_after_handshake);
        assert!(!service.is_control_ready());
    }

    #[test]
    fn reset_preserves_pending_replay_requests_for_reconnect() {
        let mut service = RtcControlChannelService::default();

        assert!(service.request_video_keyframe().is_err());
        assert!(service.request_decoder_reset().is_err());
        assert!(service.has_pending_replay_actions());

        service.reset();

        assert!(service.has_pending_replay_actions());
        assert!(service.state().pending_keyframe_request);
        assert!(service.state().pending_decoder_reset);
        assert!(!service.state().message_channel_open);
        assert!(!service.state().control_channel_open);
    }
}
