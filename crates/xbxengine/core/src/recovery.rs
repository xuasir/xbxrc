use xbxengine_protocol::{XbxEngineReconnectReasonDto, XbxEngineTransportStateDto};

pub const FIRST_FRAME_GRACE_MS: f64 = 8_000.0;
pub const KEYFRAME_REQUEST_STALL_MS: f64 = 1_500.0;
pub const RECONNECT_STALL_MS: f64 = 4_000.0;
pub const STALL_RECOVERY_COOLDOWN_MS: f64 = 6_000.0;

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
    pub last_reconnect_started_at_ms: Option<f64>,
    pub keyframe_requested_for_current_stall: bool,
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
            last_reconnect_started_at_ms: None,
            keyframe_requested_for_current_stall: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum XbxEngineRecoveryAction {
    RequestVideoKeyframe,
    RequestReconnect(XbxEngineReconnectReasonDto),
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
            }
            XbxEngineTransportStateDto::Connecting => {
                self.connected_at_ms = None;
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
    }

    pub fn mark_keyframe_requested(&mut self, now_ms: f64) {
        self.last_keyframe_request_at_ms = Some(now_ms);
        self.keyframe_requested_for_current_stall = true;
    }

    pub fn mark_reconnect_started(&mut self, now_ms: f64) {
        self.last_reconnect_started_at_ms = Some(now_ms);
        self.keyframe_requested_for_current_stall = false;
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
        if !runtime_state_is_running {
            return None;
        }
        if transport_state != &XbxEngineTransportStateDto::Connected {
            return None;
        }

        let connected_at_ms = self.connected_at_ms.unwrap_or(now_ms);
        let activity_at_ms = self
            .last_frame_rendered_at_ms
            .or(self.last_video_packet_arrival_at_ms)
            .unwrap_or(connected_at_ms);
        let stalled_for_ms = now_ms - activity_at_ms;

        if self.last_frame_rendered_at_ms.is_none()
            && self.last_video_packet_arrival_at_ms.is_none()
            && now_ms - connected_at_ms < FIRST_FRAME_GRACE_MS
        {
            return None;
        }

        let should_request_keyframe = stalled_for_ms >= KEYFRAME_REQUEST_STALL_MS
            && !self.keyframe_requested_for_current_stall
            && self
                .last_keyframe_request_at_ms
                .map(|last| now_ms - last >= KEYFRAME_REQUEST_STALL_MS)
                .unwrap_or(true);
        if should_request_keyframe {
            return Some(XbxEngineRecoveryAction::RequestVideoKeyframe);
        }

        if stalled_for_ms < RECONNECT_STALL_MS {
            return None;
        }
        if self
            .last_reconnect_started_at_ms
            .map(|last| now_ms - last < STALL_RECOVERY_COOLDOWN_MS)
            .unwrap_or(false)
        {
            return None;
        }

        Some(XbxEngineRecoveryAction::RequestReconnect(
            XbxEngineReconnectReasonDto::MediaStalled,
        ))
    }
}
