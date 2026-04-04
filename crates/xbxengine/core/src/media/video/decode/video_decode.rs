use std::collections::VecDeque;

#[cfg(test)]
use crate::media::video::types::FrameRecoveryDisposition;
use crate::{
    api::{
        MacOsVideoChromaLocation, MacOsVideoColorMatrix, MacOsVideoColorPrimaries,
        MacOsVideoColorRange, MacOsVideoTransferFunction,
    },
    media::video::h264::inspection::{H264AccessUnitInspection, H264ParameterSets},
    media::video::render::renderer::XbxRenderFrame,
    media::video::types::{DecodedFrame, EncodedFrame},
    XbxEngineRenderPixelData, XbxEngineRuntimeError,
};

const MAX_DECODED_FRAME_QUEUE_LEN: usize = 2;
const HARDWARE_DECODE_FAILURE_BURST_GAP_MS: f64 = 400.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum XbxDecodeWorkloadState {
    AwaitingInput,
    DrainOutput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct XbxDecodeWorkloadSnapshot {
    pub(crate) state: XbxDecodeWorkloadState,
    pub(crate) pending_output_queue_depth: usize,
}

impl XbxDecodeWorkloadSnapshot {
    pub(crate) fn should_drain_output_first(self) -> bool {
        matches!(self.state, XbxDecodeWorkloadState::DrainOutput)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum XbxDecodeCandidateState {
    Nominal,
    Backpressure,
}

impl XbxDecodeCandidateState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Nominal => "nominal",
            Self::Backpressure => "backpressure",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum XbxDecodeIngressDemand {
    PullOutputFirst,
    AcceptInput,
}

impl XbxDecodeIngressDemand {
    pub(crate) fn should_pull_output_first(self) -> bool {
        matches!(self, Self::PullOutputFirst)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum XbxVideoRecoveryState {
    Nominal,
    WaitingKeyframe,
    Recovering,
}

impl XbxVideoRecoveryState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Nominal => "nominal",
            Self::WaitingKeyframe => "waiting-keyframe",
            Self::Recovering => "recovering",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum XbxVideoRecoveryEvent {
    ExternalResetRequested,
    BackendFailureEscalated,
    BootstrapKeyframeAccepted,
    RecoverySettled,
}

impl XbxVideoRecoveryEvent {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ExternalResetRequested => "external-reset-requested",
            Self::BackendFailureEscalated => "backend-failure-escalated",
            Self::BootstrapKeyframeAccepted => "bootstrap-keyframe-accepted",
            Self::RecoverySettled => "recovery-settled",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct XbxDecodeCandidateDecisionSnapshot {
    pub(crate) decision_id: u64,
    pub(crate) state: XbxDecodeCandidateState,
    pub(crate) action: &'static str,
    pub(crate) detail: &'static str,
    pub(crate) frame_seq: Option<u64>,
    pub(crate) observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct XbxVideoRecoveryTransitionSnapshot {
    pub(crate) transition_id: u64,
    pub(crate) from_state: XbxVideoRecoveryState,
    pub(crate) to_state: XbxVideoRecoveryState,
    pub(crate) event: XbxVideoRecoveryEvent,
    pub(crate) detail: &'static str,
    pub(crate) frame_seq: Option<u64>,
    pub(crate) status: Option<i32>,
    pub(crate) observed_at_ms: f64,
}

trait XbxHardwareVideoDecoder: Send {
    fn backend_name(&self) -> &'static str;
    fn decode(
        &mut self,
        encoded_frame: EncodedFrame,
        now_ms: f64,
    ) -> Result<Option<XbxRenderFrame>, XbxEngineRuntimeError>;
    fn reset(&mut self) -> Result<(), XbxEngineRuntimeError>;
}

#[derive(Default)]
struct NoopXbxHardwareVideoDecoder;

impl XbxHardwareVideoDecoder for NoopXbxHardwareVideoDecoder {
    fn backend_name(&self) -> &'static str {
        "noop"
    }

    fn decode(
        &mut self,
        _encoded_frame: EncodedFrame,
        _now_ms: f64,
    ) -> Result<Option<XbxRenderFrame>, XbxEngineRuntimeError> {
        Ok(None)
    }

    fn reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }
}

fn create_hardware_video_decoder() -> Box<dyn XbxHardwareVideoDecoder> {
    #[cfg(target_os = "macos")]
    {
        match MacOsVideoToolboxDecoder::new() {
            Ok(decoder) => return Box::new(decoder),
            Err(error) => {
                crate::xbx_log_info!(
                    "[xbxengine][rtc] create macos videotoolbox decoder failed: {error}"
                );
            }
        }
    }
    Box::<NoopXbxHardwareVideoDecoder>::default()
}

pub(crate) struct XbxVideoDecodeState {
    decoder: Box<dyn XbxHardwareVideoDecoder>,
    latest_decoded_seq: u64,
    first_video_packet_logged: bool,
    decoded_frame_queue: VecDeque<DecodedFrame>,
    last_decode_ok_time_ms: Option<f64>,
    last_encoded_frame_time_ms: Option<f64>,
    decoder_reset_count: u64,
    latest_decoder_reset_time_ms: Option<f64>,
    decoded_frame_drop_count: u64,
    hardware_decode_failure_streak: u32,
    latest_hardware_decode_failure_time_ms: Option<f64>,
    latest_hardware_decode_failure_status: Option<i32>,
    recovery_state: XbxVideoRecoveryState,
    latest_recovery_state_change_time_ms: Option<f64>,
    latest_recovery_transition: Option<XbxVideoRecoveryTransitionSnapshot>,
    recovery_transition_id: u64,
    decode_candidate_state: XbxDecodeCandidateState,
    latest_decode_candidate_decision: Option<XbxDecodeCandidateDecisionSnapshot>,
    decode_candidate_decision_id: u64,
}

impl XbxVideoDecodeState {
    pub(crate) fn new(min_delay_ms: u64, max_delay_ms: u64) -> Result<Self, XbxEngineRuntimeError> {
        let _ = (min_delay_ms, max_delay_ms);
        Ok(Self {
            decoder: create_hardware_video_decoder(),
            latest_decoded_seq: 0,
            first_video_packet_logged: false,
            decoded_frame_queue: VecDeque::new(),
            last_decode_ok_time_ms: None,
            last_encoded_frame_time_ms: None,
            decoder_reset_count: 0,
            latest_decoder_reset_time_ms: None,
            decoded_frame_drop_count: 0,
            hardware_decode_failure_streak: 0,
            latest_hardware_decode_failure_time_ms: None,
            latest_hardware_decode_failure_status: None,
            recovery_state: XbxVideoRecoveryState::Nominal,
            latest_recovery_state_change_time_ms: None,
            latest_recovery_transition: None,
            recovery_transition_id: 0,
            decode_candidate_state: XbxDecodeCandidateState::Nominal,
            latest_decode_candidate_decision: None,
            decode_candidate_decision_id: 0,
        })
    }

    /**
     * 响应恢复控制面的 decoder reset：清空待释放队列，并重置硬解会话。
     * 这里不更改外部恢复阈值，只做局部状态收敛。
     */
    pub(crate) fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.decoded_frame_queue.clear();
        self.last_decode_ok_time_ms = None;
        self.decoder.reset()?;
        self.decoder_reset_count = self.decoder_reset_count.saturating_add(1);
        let now_ms = now_ms_f64();
        self.latest_decoder_reset_time_ms = Some(now_ms);
        self.reset_hardware_failure_streak();
        self.transition_recovery_state(
            XbxVideoRecoveryState::WaitingKeyframe,
            XbxVideoRecoveryEvent::ExternalResetRequested,
            "decoderResetRequested",
            None,
            None,
            now_ms,
        );
        Ok(())
    }

    pub(crate) fn process_encoded_frame(
        &mut self,
        encoded_frame: EncodedFrame,
        now_ms: f64,
    ) -> Option<DecodedFrame> {
        self.last_encoded_frame_time_ms = Some(now_ms);
        if matches!(self.recovery_state, XbxVideoRecoveryState::WaitingKeyframe)
            && !encoded_frame.h264.bootstrap_ready
        {
            return None;
        }
        if !self.first_video_packet_logged {
            self.first_video_packet_logged = true;
            crate::xbx_log_info!(
                "[xbxengine][rtc] first encoded video frame received ts={} bytes={}",
                encoded_frame.rtp_timestamp,
                encoded_frame.payload.len()
            );
        }
        let target_time = encoded_frame.target_playout_time;
        let rtp_timestamp = encoded_frame.rtp_timestamp;
        let is_keyframe = encoded_frame.is_keyframe;
        let budget = encoded_frame.budget;
        let frame_recovery_disposition = encoded_frame.frame_recovery_disposition;
        let frame_unrecoverable_reason = encoded_frame.frame_unrecoverable_reason.clone();
        let recovery_state_before_decode = self.recovery_state;
        let decoded_frame = match self.decoder.decode(encoded_frame, now_ms) {
            Ok(frame) => frame,
            Err(error) => {
                let status = parse_decoder_status_code(&error);
                crate::xbx_log_error!("[xbxengine][rtc] hardware decode failed: {error}");
                self.record_hardware_decode_failure(now_ms, status);
                if should_force_recovery_keyframe(status)
                    || self.hardware_decode_failure_streak >= 3
                {
                    crate::xbx_log_warn!(
                        "[xbxengine][rtc] decoder entered wait-keyframe recovery after backend failure"
                    );
                    let _ = self.request_decoder_reset_with_failure(status, now_ms);
                }
                return None;
            }
        };
        let Some(mut render_frame) = decoded_frame else {
            return None;
        };
        match recovery_state_before_decode {
            XbxVideoRecoveryState::WaitingKeyframe => self.transition_recovery_state(
                XbxVideoRecoveryState::Recovering,
                XbxVideoRecoveryEvent::BootstrapKeyframeAccepted,
                "bootstrapKeyframeDecoded",
                Some(rtp_timestamp as u64),
                None,
                now_ms,
            ),
            XbxVideoRecoveryState::Recovering => self.transition_recovery_state(
                XbxVideoRecoveryState::Nominal,
                XbxVideoRecoveryEvent::RecoverySettled,
                "recoverySettled",
                Some(rtp_timestamp as u64),
                None,
                now_ms,
            ),
            XbxVideoRecoveryState::Nominal => {}
        }
        self.reset_hardware_failure_streak();
        self.latest_decoded_seq = self.latest_decoded_seq.saturating_add(1);
        self.last_decode_ok_time_ms = Some(now_ms);
        render_frame.frame_seq = self.latest_decoded_seq;
        render_frame.rendered_at_ms = now_ms;
        render_frame.rtp_timestamp = Some(rtp_timestamp);
        render_frame.is_keyframe = is_keyframe;
        render_frame.frame_recovery_disposition =
            Some(frame_recovery_disposition.as_str().to_string());
        render_frame.frame_unrecoverable_reason = frame_unrecoverable_reason.clone();
        self.enqueue_decoded_frame(DecodedFrame {
            pts: target_time,
            rtp_timestamp,
            is_keyframe,
            budget,
            frame_recovery_disposition,
            frame_unrecoverable_reason,
            surface: render_frame,
        })
    }

    pub(crate) fn last_decode_ok_time_ms(&self) -> Option<f64> {
        self.last_decode_ok_time_ms
    }

    pub(crate) fn decoder_backend_name(&self) -> &'static str {
        self.decoder.backend_name()
    }

    pub(crate) fn decoder_reset_count(&self) -> u64 {
        self.decoder_reset_count
    }

    pub(crate) fn latest_decoder_reset_time_ms(&self) -> Option<f64> {
        self.latest_decoder_reset_time_ms
    }

    pub(crate) fn decoded_frame_drop_count(&self) -> u64 {
        self.decoded_frame_drop_count
    }

    pub(crate) fn hardware_decode_failure_streak(&self) -> u32 {
        self.hardware_decode_failure_streak
    }

    pub(crate) fn latest_hardware_decode_failure_time_ms(&self) -> Option<f64> {
        self.latest_hardware_decode_failure_time_ms
    }

    pub(crate) fn latest_hardware_decode_failure_status(&self) -> Option<i32> {
        self.latest_hardware_decode_failure_status
    }

    pub(crate) fn recovery_state(&self) -> XbxVideoRecoveryState {
        self.recovery_state
    }

    pub(crate) fn latest_recovery_state_change_time_ms(&self) -> Option<f64> {
        self.latest_recovery_state_change_time_ms
    }

    pub(crate) fn latest_recovery_transition(&self) -> Option<&XbxVideoRecoveryTransitionSnapshot> {
        self.latest_recovery_transition.as_ref()
    }

    pub(crate) fn latest_decode_candidate_decision(
        &self,
    ) -> Option<&XbxDecodeCandidateDecisionSnapshot> {
        self.latest_decode_candidate_decision.as_ref()
    }

    pub(crate) fn pop_decoded_frame(&mut self, _now_ms: f64) -> Option<DecodedFrame> {
        // native 路径已经有 pacer 负责 playout 节奏，decode stage 不再额外等待。
        self.decoded_frame_queue.pop_front()
    }

    pub(crate) fn has_decoded_frame(&self) -> bool {
        !self.decoded_frame_queue.is_empty()
    }

    pub(crate) fn workload_snapshot(&self) -> XbxDecodeWorkloadSnapshot {
        let pending_output_queue_depth = self.decoded_frame_queue.len();
        let state = if pending_output_queue_depth > 0 {
            XbxDecodeWorkloadState::DrainOutput
        } else {
            XbxDecodeWorkloadState::AwaitingInput
        };
        XbxDecodeWorkloadSnapshot {
            state,
            pending_output_queue_depth,
        }
    }

    pub(crate) fn ingress_demand(&self) -> XbxDecodeIngressDemand {
        if self.workload_snapshot().should_drain_output_first() {
            XbxDecodeIngressDemand::PullOutputFirst
        } else {
            XbxDecodeIngressDemand::AcceptInput
        }
    }

    #[cfg(test)]
    pub(crate) fn peek_decoded_frame(&self) -> Option<&DecodedFrame> {
        self.decoded_frame_queue.front()
    }

    pub(crate) fn decoded_frame_queue_len(&self) -> usize {
        self.decoded_frame_queue.len()
    }

    #[cfg(test)]
    pub(crate) fn decoded_frame_queue_is_full(&self) -> bool {
        self.decoded_frame_queue.len() >= MAX_DECODED_FRAME_QUEUE_LEN
    }

    pub(crate) fn requeue_decoded_frame_front(&mut self, frame: DecodedFrame) {
        self.decoded_frame_queue.push_front(frame);
    }

    fn enqueue_decoded_frame(&mut self, frame: DecodedFrame) -> Option<DecodedFrame> {
        let incoming_frame_seq = frame.surface.frame_seq;
        let observed_at_ms = frame.surface.rendered_at_ms;
        let mut dropped_frame = None;
        while self.decoded_frame_queue.len() >= MAX_DECODED_FRAME_QUEUE_LEN {
            let dropped = self.decoded_frame_queue.pop_front();
            if let Some(d) = dropped {
                crate::xbx_log_warn!(
                    "[xbxengine][vt] enqueue_decoded_frame: queue FULL, dropping old frame seq={}",
                    d.surface.frame_seq
                );
                dropped_frame = Some(d);
            }
            self.decoded_frame_drop_count = self.decoded_frame_drop_count.saturating_add(1);
        }
        self.decoded_frame_queue.push_back(frame);
        if let Some(dropped) = dropped_frame.as_ref() {
            self.record_decode_candidate_decision(
                XbxDecodeCandidateState::Backpressure,
                "drop",
                "outputQueueOverflow",
                Some(dropped.surface.frame_seq),
                observed_at_ms,
            );
        } else if matches!(
            self.decode_candidate_state,
            XbxDecodeCandidateState::Backpressure
        ) {
            self.record_decode_candidate_decision(
                XbxDecodeCandidateState::Nominal,
                "accept",
                "queueRecovered",
                Some(incoming_frame_seq),
                observed_at_ms,
            );
        }
        dropped_frame
    }

    // 连续硬解失败用于 recovery 诊断：只在短窗口内累加，避免偶发错误误触发。
    fn record_hardware_decode_failure(&mut self, now_ms: f64, status: Option<i32>) {
        let same_burst = self
            .latest_hardware_decode_failure_time_ms
            .map(|last| (now_ms - last).max(0.0) <= HARDWARE_DECODE_FAILURE_BURST_GAP_MS)
            .unwrap_or(false);
        self.hardware_decode_failure_streak = if same_burst {
            self.hardware_decode_failure_streak.saturating_add(1)
        } else {
            1
        };
        self.latest_hardware_decode_failure_time_ms = Some(now_ms);
        self.latest_hardware_decode_failure_status = status;
    }

    fn reset_hardware_failure_streak(&mut self) {
        self.hardware_decode_failure_streak = 0;
        self.latest_hardware_decode_failure_status = None;
    }

    fn request_decoder_reset_with_failure(
        &mut self,
        status: Option<i32>,
        now_ms: f64,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.decoded_frame_queue.clear();
        self.last_decode_ok_time_ms = None;
        self.decoder.reset()?;
        self.decoder_reset_count = self.decoder_reset_count.saturating_add(1);
        self.latest_decoder_reset_time_ms = Some(now_ms);
        self.reset_hardware_failure_streak();
        self.transition_recovery_state(
            XbxVideoRecoveryState::WaitingKeyframe,
            XbxVideoRecoveryEvent::BackendFailureEscalated,
            "backendFailureReset",
            None,
            status,
            now_ms,
        );
        Ok(())
    }

    fn transition_recovery_state(
        &mut self,
        to_state: XbxVideoRecoveryState,
        event: XbxVideoRecoveryEvent,
        detail: &'static str,
        frame_seq: Option<u64>,
        status: Option<i32>,
        observed_at_ms: f64,
    ) {
        let from_state = self.recovery_state;
        self.recovery_state = to_state;
        self.recovery_transition_id = self.recovery_transition_id.saturating_add(1);
        self.latest_recovery_state_change_time_ms = Some(observed_at_ms);
        self.latest_recovery_transition = Some(XbxVideoRecoveryTransitionSnapshot {
            transition_id: self.recovery_transition_id,
            from_state,
            to_state,
            event,
            detail,
            frame_seq,
            status,
            observed_at_ms,
        });
    }

    fn record_decode_candidate_decision(
        &mut self,
        state: XbxDecodeCandidateState,
        action: &'static str,
        detail: &'static str,
        frame_seq: Option<u64>,
        observed_at_ms: f64,
    ) {
        self.decode_candidate_state = state;
        self.decode_candidate_decision_id = self.decode_candidate_decision_id.saturating_add(1);
        self.latest_decode_candidate_decision = Some(XbxDecodeCandidateDecisionSnapshot {
            decision_id: self.decode_candidate_decision_id,
            state,
            action,
            detail,
            frame_seq,
            observed_at_ms,
        });
    }
}

#[cfg(test)]
impl XbxVideoDecodeState {
    fn new_for_test(
        min_delay_ms: u64,
        max_delay_ms: u64,
        decoder: Box<dyn XbxHardwareVideoDecoder>,
    ) -> Self {
        let _ = (min_delay_ms, max_delay_ms);
        Self {
            decoder,
            latest_decoded_seq: 0,
            first_video_packet_logged: false,
            decoded_frame_queue: VecDeque::new(),
            last_decode_ok_time_ms: None,
            last_encoded_frame_time_ms: None,
            decoder_reset_count: 0,
            latest_decoder_reset_time_ms: None,
            decoded_frame_drop_count: 0,
            hardware_decode_failure_streak: 0,
            latest_hardware_decode_failure_time_ms: None,
            latest_hardware_decode_failure_status: None,
            recovery_state: XbxVideoRecoveryState::Nominal,
            latest_recovery_state_change_time_ms: None,
            latest_recovery_transition: None,
            recovery_transition_id: 0,
            decode_candidate_state: XbxDecodeCandidateState::Nominal,
            latest_decode_candidate_decision: None,
            decode_candidate_decision_id: 0,
        }
    }

    pub(crate) fn enqueue_decoded_frame_for_test(&mut self, frame: XbxRenderFrame) {
        let _ = self.enqueue_decoded_frame(DecodedFrame {
            pts: std::time::Instant::now(),
            rtp_timestamp: frame.frame_seq as u32,
            is_keyframe: frame.is_keyframe,
            budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
            frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
            frame_unrecoverable_reason: None,
            surface: frame,
        });
    }
}

pub(crate) fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

fn parse_decoder_status_code(error: &XbxEngineRuntimeError) -> Option<i32> {
    let message = error.to_string();
    let status = message.split("status=").nth(1)?;
    let token = status
        .split(|ch: char| !(ch == '-' || ch.is_ascii_digit()))
        .next()?;
    token.parse::<i32>().ok()
}

fn should_force_recovery_keyframe(status: Option<i32>) -> bool {
    matches!(
        status,
        Some(K_VT_VIDEO_DECODER_BAD_DATA_ERR | K_VT_VIDEO_DECODER_REFERENCE_MISSING_ERR)
    )
}

#[cfg(target_os = "macos")]
struct MacOsVideoToolboxDecoder {
    format_description: CMVideoFormatDescriptionRef,
    decompression_session: VTDecompressionSessionRef,
    last_parameter_sets: Option<H264ParameterSets>,
}

#[cfg(target_os = "macos")]
unsafe impl Send for MacOsVideoToolboxDecoder {}

#[cfg(target_os = "macos")]
impl MacOsVideoToolboxDecoder {
    fn new() -> Result<Self, XbxEngineRuntimeError> {
        Ok(Self {
            format_description: std::ptr::null_mut(),
            decompression_session: std::ptr::null_mut(),
            last_parameter_sets: None,
        })
    }

    fn ensure_decoder_session(
        &mut self,
        inspection: &H264AccessUnitInspection,
    ) -> Result<bool, XbxEngineRuntimeError> {
        let Some(parameter_sets) = inspection.bootstrap_parameter_sets() else {
            return Ok(!self.decompression_session.is_null());
        };

        if self
            .last_parameter_sets
            .as_ref()
            .is_none_or(|committed| !committed.same_decoder_configuration(parameter_sets))
        {
            self.last_parameter_sets = Some(parameter_sets.clone());
            self.release_session();
        }

        if !self.decompression_session.is_null() {
            return Ok(true);
        }

        if !inspection.bootstrap_ready {
            return Ok(false);
        }

        let parameter_sets = self
            .last_parameter_sets
            .as_ref()
            .expect("parameter sets must be captured before creating a session");
        let parameter_set_pointers = [
            parameter_sets.sps.raw.as_ptr(),
            parameter_sets.pps.raw.as_ptr(),
        ];
        let parameter_set_sizes = [parameter_sets.sps.raw.len(), parameter_sets.pps.raw.len()];
        let mut format_description: CMVideoFormatDescriptionRef = std::ptr::null_mut();
        let status = unsafe {
            CMVideoFormatDescriptionCreateFromH264ParameterSets(
                std::ptr::null(),
                2,
                parameter_set_pointers.as_ptr(),
                parameter_set_sizes.as_ptr(),
                4,
                &mut format_description,
            )
        };
        if status != NO_ERR || format_description.is_null() {
            return Err(XbxEngineRuntimeError::new(format!(
                "xbxEngineCreateVideoFormatDescriptionFailed:status={status}"
            )));
        }

        let mut callback_record = VTDecompressionOutputCallbackRecord {
            decompression_output_callback: Some(vt_decompression_output_callback),
            decompression_output_ref_con: std::ptr::null_mut(),
        };

        // 指定输出像素缓冲区属性 (NV12)
        unsafe {
            let pixel_format = K_CVPIXEL_FORMAT_TYPE_420YPCBCR8_BIPLANAR_VIDEO_RANGE as i32;
            let pixel_format_num = CFNumberCreate(
                std::ptr::null(),
                kCFNumberSInt32Type,
                &pixel_format as *const i32 as *const std::ffi::c_void,
            );

            let keys = [kCVPixelBufferPixelFormatTypeKey];
            let values = [pixel_format_num as *const std::ffi::c_void];
            let pixel_buffer_attributes = CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            );

            if !pixel_format_num.is_null() {
                CFRelease(pixel_format_num as _);
            }

            if pixel_buffer_attributes.is_null() {
                crate::xbx_log_error!("[xbxengine][rtc][vt] create pixel buffer attributes failed");
            }

            let mut session: VTDecompressionSessionRef = std::ptr::null_mut();
            let status = VTDecompressionSessionCreate(
                std::ptr::null(),
                format_description,
                std::ptr::null(),
                pixel_buffer_attributes as _,
                &mut callback_record,
                &mut session,
            );

            if !pixel_buffer_attributes.is_null() {
                CFRelease(pixel_buffer_attributes as _);
            }

            if status != NO_ERR || session.is_null() {
                CFRelease(format_description as CFTypeRef);
                return Err(XbxEngineRuntimeError::new(format!(
                    "xbxEngineCreateVideoDecompressionSessionFailed:status={status}"
                )));
            }

            // 设置实时属性 (RealTime) 以确保低延迟输出
            let key = kVTDecompressionPropertyKey_RealTime;
            let val = kCFBooleanTrue;
            VTSessionSetProperty(session as _, key as _, val as _);

            self.decompression_session = session;
        }

        self.format_description = format_description;
        Ok(true)
    }

    fn release_session(&mut self) {
        if !self.decompression_session.is_null() {
            unsafe {
                // SAFETY: 会话由 VideoToolbox 创建，按官方顺序 invalidate + CFRelease。
                VTDecompressionSessionInvalidate(self.decompression_session);
                CFRelease(self.decompression_session as CFTypeRef);
            }
            self.decompression_session = std::ptr::null_mut();
        }
        if !self.format_description.is_null() {
            unsafe {
                // SAFETY: format description 由 CoreMedia 创建，需对称释放。
                CFRelease(self.format_description as CFTypeRef);
            }
            self.format_description = std::ptr::null_mut();
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for MacOsVideoToolboxDecoder {
    fn drop(&mut self) {
        self.release_session();
    }
}

#[cfg(target_os = "macos")]
impl XbxHardwareVideoDecoder for MacOsVideoToolboxDecoder {
    fn backend_name(&self) -> &'static str {
        "videotoolbox"
    }

    fn decode(
        &mut self,
        encoded_frame: EncodedFrame,
        _now_ms: f64,
    ) -> Result<Option<XbxRenderFrame>, XbxEngineRuntimeError> {
        if !self.ensure_decoder_session(&encoded_frame.h264)? {
            return Ok(None);
        }

        // 由 inspection 给出统一的 NAL 结果，避免这里再自行扫 Annex-B。
        let avcc_payload = encoded_frame
            .h264
            .build_avcc_payload(&encoded_frame.payload);

        if avcc_payload.is_empty() {
            return Ok(None);
        }

        let mut block_buffer: CMBlockBufferRef = std::ptr::null_mut();
        let status = unsafe {
            // 首先创建一个拥有指定长度但尚无实际数据的 BlockBuffer。
            // 使用 NULL 作为 blockSource 让系统自行管理内存，确保异步场景下的内存安全。
            CMBlockBufferCreateWithMemoryBlock(
                std::ptr::null(),
                std::ptr::null_mut(),
                avcc_payload.len(),
                std::ptr::null(),
                std::ptr::null_mut(),
                0,
                avcc_payload.len(),
                0, // kCMBlockBufferAssureMemoryNowFlag
                &mut block_buffer,
            )
        };
        if status != NO_ERR || block_buffer.is_null() {
            return Err(XbxEngineRuntimeError::new(format!(
                "xbxEngineCreateBlockBufferFailed:status={status}"
            )));
        }

        // 将 avcc_payload 内容拷贝进 BlockBuffer。此时 BlockBuffer 已拥有独立副本。
        let status = unsafe {
            CMBlockBufferReplaceDataBytes(
                avcc_payload.as_ptr() as *const std::ffi::c_void,
                block_buffer,
                0,
                avcc_payload.len(),
            )
        };
        if status != NO_ERR {
            unsafe {
                CFRelease(block_buffer as CFTypeRef);
            }
            return Err(XbxEngineRuntimeError::new(format!(
                "xbxEngineFillBlockBufferFailed:status={status}"
            )));
        }

        let sample_size = avcc_payload.len();
        let mut sample_buffer: CMSampleBufferRef = std::ptr::null_mut();
        let status = unsafe {
            CMSampleBufferCreateReady(
                std::ptr::null(),
                block_buffer,
                self.format_description,
                1,
                0,
                std::ptr::null(),
                1,
                &sample_size,
                &mut sample_buffer,
            )
        };
        unsafe {
            CFRelease(block_buffer as CFTypeRef);
        }
        if status != NO_ERR || sample_buffer.is_null() {
            return Err(XbxEngineRuntimeError::new(format!(
                "xbxEngineCreateSampleBufferFailed:status={status}"
            )));
        }

        // 使用堆分配的同步状态，确保回调中的 source_frame_ref_con 绝对有效直到本函数返回。
        let mut output_state = Box::new(VideoToolboxOutputState::default());
        let (sync_tx, sync_rx) = std::sync::mpsc::sync_channel(1);
        output_state.sync_tx = Some(sync_tx);

        let mut decode_info_flags = 0u32;
        let status = unsafe {
            VTDecompressionSessionDecodeFrame(
                self.decompression_session,
                sample_buffer,
                0,
                Box::into_raw(output_state) as *mut std::ffi::c_void,
                &mut decode_info_flags,
            )
        };
        unsafe {
            CFRelease(sample_buffer as CFTypeRef);
        }

        if status != NO_ERR {
            return Err(XbxEngineRuntimeError::new(format!(
                "xbxEngineVideoToolboxDecodeFailed:status={status}"
            )));
        }

        // 等待同步回答。
        let result_state = match sync_rx.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(state_ptr) => unsafe { Box::from_raw(state_ptr) },
            Err(_) => {
                crate::xbx_log_error!(
                    "[xbxengine][rtc][vt] decode session timed out or callback never fired"
                );
                return Err(XbxEngineRuntimeError::new(
                    "xbxEngineVideoToolboxDecodeTimeout".to_string(),
                ));
            }
        };

        if result_state.status != NO_ERR {
            return Err(XbxEngineRuntimeError::new(format!(
                "xbxEngineVideoToolboxOutputCallbackFailed:status={}",
                result_state.status
            )));
        }

        let pixel_buffer = result_state.pixel_buffer;
        if pixel_buffer.is_null() {
            return Ok(None);
        }

        let width = unsafe { CVPixelBufferGetWidth(pixel_buffer) as u32 };
        let height = unsafe { CVPixelBufferGetHeight(pixel_buffer) as u32 };

        // 已经由回调函数 retain 过了，此处直接接管所有权。
        let frame = XbxRenderFrame {
            width,
            height,
            frame_seq: 0,
            rendered_at_ms: 0.0,
            rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: None,
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Descriptor {
                handle: std::sync::Arc::new(crate::api::backend::MacOsCVPixelBufferDescriptor {
                    ptr: pixel_buffer as *mut _,
                    color_matrix: pixel_buffer_color_matrix(pixel_buffer),
                    color_primaries: pixel_buffer_color_primaries(pixel_buffer),
                    transfer_function: pixel_buffer_transfer_function(pixel_buffer),
                    color_range: pixel_buffer_color_range(pixel_buffer),
                    chroma_location: pixel_buffer_chroma_location(pixel_buffer),
                    drop_fn: Some(Box::new(|ptr| unsafe {
                        CFRelease(ptr as CFTypeRef);
                    })),
                }),
            },
        };

        Ok(Some(frame))
    }

    fn reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.release_session();
        Ok(())
    }
}

#[cfg(target_os = "macos")]
struct VideoToolboxOutputState {
    status: OSStatus,
    pixel_buffer: CVImageBufferRef,
    sync_tx: Option<std::sync::mpsc::SyncSender<*mut VideoToolboxOutputState>>,
}

impl Default for VideoToolboxOutputState {
    fn default() -> Self {
        Self {
            status: NO_ERR,
            pixel_buffer: std::ptr::null_mut(),
            sync_tx: None,
        }
    }
}

#[cfg(target_os = "macos")]
extern "C" fn vt_decompression_output_callback(
    _decompression_output_ref_con: *mut std::ffi::c_void,
    source_frame_ref_con: *mut std::ffi::c_void,
    status: OSStatus,
    _info_flags: VTDecodeInfoFlags,
    image_buffer: CVImageBufferRef,
    _presentation_time_stamp: CMTime,
    _presentation_duration: CMTime,
) {
    if source_frame_ref_con.is_null() {
        return;
    }
    let output_ptr = source_frame_ref_con as *mut VideoToolboxOutputState;
    let output = unsafe { &mut *output_ptr };
    output.status = status;
    if status == NO_ERR && !image_buffer.is_null() {
        unsafe {
            // SAFETY: 回调返回后仍需读取像素缓冲，先 retain 再在上层释放。
            CFRetain(image_buffer as CFTypeRef);
        }
        output.pixel_buffer = image_buffer;
    } else {
        output.pixel_buffer = std::ptr::null_mut();
    }

    // 通过 sync_tx 发送自己（指针），知会上层解码完成。
    if let Some(tx) = output.sync_tx.take() {
        let _ = tx.send(output_ptr);
    }
}

#[cfg(target_os = "macos")]
type OSStatus = i32;
#[cfg(target_os = "macos")]
type CFTypeRef = *const std::ffi::c_void;
#[cfg(target_os = "macos")]
type CFAllocatorRef = *const std::ffi::c_void;
#[cfg(target_os = "macos")]
type CMVideoFormatDescriptionRef = *mut std::ffi::c_void;
#[cfg(target_os = "macos")]
type CMBlockBufferRef = *mut std::ffi::c_void;
#[cfg(target_os = "macos")]
type CMSampleBufferRef = *mut std::ffi::c_void;
#[cfg(target_os = "macos")]
type VTDecompressionSessionRef = *mut std::ffi::c_void;
#[cfg(target_os = "macos")]
type CVImageBufferRef = *mut std::ffi::c_void;
#[cfg(target_os = "macos")]
type VTDecodeInfoFlags = u32;
#[cfg(target_os = "macos")]
type CFNumberRef = *const std::ffi::c_void;
#[cfg(target_os = "macos")]
type CFStringRef = *const std::ffi::c_void;
#[cfg(target_os = "macos")]
type CVAttachmentMode = u32;
#[allow(non_upper_case_globals)]
#[cfg(target_os = "macos")]
const kCFNumberSInt32Type: i32 = 3;

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct VTDecompressionOutputCallbackRecord {
    decompression_output_callback: Option<
        extern "C" fn(
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
            OSStatus,
            VTDecodeInfoFlags,
            CVImageBufferRef,
            CMTime,
            CMTime,
        ),
    >,
    decompression_output_ref_con: *mut std::ffi::c_void,
}

#[cfg(target_os = "macos")]
const NO_ERR: OSStatus = 0;
#[cfg(target_os = "macos")]
const K_CVPIXEL_FORMAT_TYPE_420YPCBCR8_BIPLANAR_VIDEO_RANGE: u32 = 0x3432_3076;
const K_VT_VIDEO_DECODER_BAD_DATA_ERR: i32 = -12909;
const K_VT_VIDEO_DECODER_REFERENCE_MISSING_ERR: i32 = -17694;

#[cfg(target_os = "macos")]
#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    fn VTDecompressionSessionCreate(
        allocator: CFAllocatorRef,
        video_format_description: CMVideoFormatDescriptionRef,
        video_decoder_specification: *const std::ffi::c_void,
        destination_image_buffer_attributes: *const std::ffi::c_void,
        output_callback: *const VTDecompressionOutputCallbackRecord,
        decompression_session_out: *mut VTDecompressionSessionRef,
    ) -> OSStatus;
    fn VTDecompressionSessionDecodeFrame(
        session: VTDecompressionSessionRef,
        sample_buffer: CMSampleBufferRef,
        decode_flags: u32,
        source_frame_ref_con: *mut std::ffi::c_void,
        info_flags_out: *mut VTDecodeInfoFlags,
    ) -> OSStatus;
    fn VTDecompressionSessionInvalidate(session: VTDecompressionSessionRef);

    fn VTSessionSetProperty(
        session: *mut std::ffi::c_void,
        property_key: CFTypeRef,
        property_value: CFTypeRef,
    ) -> OSStatus;

    static kVTDecompressionPropertyKey_RealTime: CFTypeRef;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
        allocator: CFAllocatorRef,
        parameter_set_count: usize,
        parameter_set_pointers: *const *const u8,
        parameter_set_sizes: *const usize,
        nal_unit_header_length: i32,
        format_description_out: *mut CMVideoFormatDescriptionRef,
    ) -> OSStatus;
    fn CMBlockBufferCreateWithMemoryBlock(
        structure_allocator: CFAllocatorRef,
        memory_block: *mut std::ffi::c_void,
        block_length: usize,
        block_allocator: CFAllocatorRef,
        custom_block_source: *mut std::ffi::c_void,
        offset_to_data: usize,
        data_length: usize,
        flags: u32,
        new_block_buffer_out: *mut CMBlockBufferRef,
    ) -> OSStatus;
    fn CMBlockBufferReplaceDataBytes(
        source_bytes: *const std::ffi::c_void,
        destination_block_buffer: CMBlockBufferRef,
        offset_into_destination: usize,
        data_length: usize,
    ) -> OSStatus;
    fn CMSampleBufferCreateReady(
        allocator: CFAllocatorRef,
        data_buffer: CMBlockBufferRef,
        format_description: CMVideoFormatDescriptionRef,
        num_samples: i64,
        num_sample_timing_entries: i64,
        sample_timing_array: *const std::ffi::c_void,
        num_sample_size_entries: i64,
        sample_size_array: *const usize,
        sample_buffer_out: *mut CMSampleBufferRef,
    ) -> OSStatus;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    static kCVPixelBufferPixelFormatTypeKey: *const std::ffi::c_void;
    static kCVImageBufferYCbCrMatrixKey: CFStringRef;
    static kCVImageBufferYCbCrMatrix_ITU_R_709_2: CFStringRef;
    static kCVImageBufferYCbCrMatrix_ITU_R_601_4: CFStringRef;
    static kCVImageBufferYCbCrMatrix_SMPTE_240M_1995: CFStringRef;
    static kCVImageBufferYCbCrMatrix_ITU_R_2020: CFStringRef;
    static kCVImageBufferColorPrimariesKey: CFStringRef;
    static kCVImageBufferColorPrimaries_ITU_R_709_2: CFStringRef;
    static kCVImageBufferColorPrimaries_P3_D65: CFStringRef;
    static kCVImageBufferColorPrimaries_ITU_R_2020: CFStringRef;
    static kCVImageBufferTransferFunctionKey: CFStringRef;
    static kCVImageBufferTransferFunction_ITU_R_709_2: CFStringRef;
    static kCVImageBufferTransferFunction_sRGB: CFStringRef;
    static kCVImageBufferTransferFunction_Linear: CFStringRef;
    static kCVImageBufferChromaLocationTopFieldKey: CFStringRef;
    static kCVImageBufferChromaLocationBottomFieldKey: CFStringRef;
    static kCVImageBufferChromaLocation_Center: CFStringRef;
    static kCVImageBufferChromaLocation_Left: CFStringRef;
    static kCVImageBufferChromaLocation_TopLeft: CFStringRef;
    fn CVPixelBufferGetWidth(pixel_buffer: CVImageBufferRef) -> usize;
    fn CVPixelBufferGetHeight(pixel_buffer: CVImageBufferRef) -> usize;
    fn CVBufferGetAttachment(
        buffer: CVImageBufferRef,
        key: CFStringRef,
        attachment_mode: *mut CVAttachmentMode,
    ) -> CFTypeRef;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    fn CFRelease(cf: CFTypeRef);
    fn CFEqual(cf1: CFTypeRef, cf2: CFTypeRef) -> bool;

    static kCFTypeDictionaryKeyCallBacks: std::ffi::c_void;
    static kCFTypeDictionaryValueCallBacks: std::ffi::c_void;

    fn CFDictionaryCreate(
        allocator: CFAllocatorRef,
        keys: *const *const std::ffi::c_void,
        values: *const *const std::ffi::c_void,
        num_values: isize,
        key_callbacks: *const std::ffi::c_void,
        value_callbacks: *const std::ffi::c_void,
    ) -> crate::api::backend::CFDictionaryRef;

    fn CFNumberCreate(
        allocator: CFAllocatorRef,
        the_type: i32,
        value_ptr: *const std::ffi::c_void,
    ) -> CFNumberRef;

    static kCFBooleanTrue: CFTypeRef;
}

#[cfg(target_os = "macos")]
fn pixel_buffer_color_matrix(pixel_buffer: CVImageBufferRef) -> MacOsVideoColorMatrix {
    let attachment = cv_attachment(pixel_buffer, unsafe { kCVImageBufferYCbCrMatrixKey });
    match attachment {
        Some(value) if cf_equals(value, unsafe { kCVImageBufferYCbCrMatrix_ITU_R_709_2 }) => {
            MacOsVideoColorMatrix::Bt709
        }
        Some(value) if cf_equals(value, unsafe { kCVImageBufferYCbCrMatrix_ITU_R_601_4 }) => {
            MacOsVideoColorMatrix::Bt601
        }
        Some(value) if cf_equals(value, unsafe { kCVImageBufferYCbCrMatrix_SMPTE_240M_1995 }) => {
            MacOsVideoColorMatrix::Smpte240M
        }
        Some(value) if cf_equals(value, unsafe { kCVImageBufferYCbCrMatrix_ITU_R_2020 }) => {
            MacOsVideoColorMatrix::Bt2020
        }
        Some(_) => MacOsVideoColorMatrix::Unknown,
        None => MacOsVideoColorMatrix::Bt709,
    }
}

#[cfg(target_os = "macos")]
fn pixel_buffer_color_primaries(pixel_buffer: CVImageBufferRef) -> MacOsVideoColorPrimaries {
    let attachment = cv_attachment(pixel_buffer, unsafe { kCVImageBufferColorPrimariesKey });
    match attachment {
        Some(value) if cf_equals(value, unsafe { kCVImageBufferColorPrimaries_ITU_R_709_2 }) => {
            MacOsVideoColorPrimaries::Bt709
        }
        Some(value) if cf_equals(value, unsafe { kCVImageBufferColorPrimaries_P3_D65 }) => {
            MacOsVideoColorPrimaries::P3D65
        }
        Some(value) if cf_equals(value, unsafe { kCVImageBufferColorPrimaries_ITU_R_2020 }) => {
            MacOsVideoColorPrimaries::Bt2020
        }
        Some(_) => MacOsVideoColorPrimaries::Unknown,
        None => MacOsVideoColorPrimaries::Bt709,
    }
}

#[cfg(target_os = "macos")]
fn pixel_buffer_transfer_function(pixel_buffer: CVImageBufferRef) -> MacOsVideoTransferFunction {
    let attachment = cv_attachment(pixel_buffer, unsafe { kCVImageBufferTransferFunctionKey });
    match attachment {
        Some(value) if cf_equals(value, unsafe { kCVImageBufferTransferFunction_ITU_R_709_2 }) => {
            MacOsVideoTransferFunction::Bt709
        }
        Some(value) if cf_equals(value, unsafe { kCVImageBufferTransferFunction_sRGB }) => {
            MacOsVideoTransferFunction::Srgb
        }
        Some(value) if cf_equals(value, unsafe { kCVImageBufferTransferFunction_Linear }) => {
            MacOsVideoTransferFunction::Linear
        }
        Some(_) => MacOsVideoTransferFunction::Unknown,
        None => MacOsVideoTransferFunction::Bt709,
    }
}

#[cfg(target_os = "macos")]
fn pixel_buffer_color_range(_pixel_buffer: CVImageBufferRef) -> MacOsVideoColorRange {
    // 当前 VideoToolbox 会话固定请求 video-range NV12，先显式带入 descriptor。
    MacOsVideoColorRange::Video
}

#[cfg(target_os = "macos")]
fn pixel_buffer_chroma_location(pixel_buffer: CVImageBufferRef) -> MacOsVideoChromaLocation {
    let attachment = cv_attachment(pixel_buffer, unsafe {
        kCVImageBufferChromaLocationTopFieldKey
    })
    .or_else(|| {
        cv_attachment(pixel_buffer, unsafe {
            kCVImageBufferChromaLocationBottomFieldKey
        })
    });
    match attachment {
        Some(value) if cf_equals(value, unsafe { kCVImageBufferChromaLocation_Center }) => {
            MacOsVideoChromaLocation::Center
        }
        Some(value) if cf_equals(value, unsafe { kCVImageBufferChromaLocation_Left }) => {
            MacOsVideoChromaLocation::Left
        }
        Some(value) if cf_equals(value, unsafe { kCVImageBufferChromaLocation_TopLeft }) => {
            MacOsVideoChromaLocation::TopLeft
        }
        Some(_) => MacOsVideoChromaLocation::Unknown,
        None => MacOsVideoChromaLocation::Center,
    }
}

#[cfg(target_os = "macos")]
fn cv_attachment(pixel_buffer: CVImageBufferRef, key: CFStringRef) -> Option<CFTypeRef> {
    if pixel_buffer.is_null() || key.is_null() {
        return None;
    }
    let value = unsafe { CVBufferGetAttachment(pixel_buffer, key, std::ptr::null_mut()) };
    if value.is_null() {
        None
    } else {
        Some(value)
    }
}

#[cfg(target_os = "macos")]
fn cf_equals(lhs: CFTypeRef, rhs: CFTypeRef) -> bool {
    if lhs.is_null() || rhs.is_null() {
        return false;
    }
    unsafe { CFEqual(lhs, rhs) }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::{Duration, Instant};

    use super::{
        XbxDecodeCandidateState, XbxDecodeWorkloadState, XbxHardwareVideoDecoder,
        XbxVideoDecodeState, XbxVideoRecoveryEvent, XbxVideoRecoveryState,
    };
    use crate::media::video::h264::inspection::{
        H264AccessUnitInspection, H264AccessUnitInspector, H264BootstrapRejectReason,
    };
    use crate::{
        api::backend::XbxEngineMediaRuntimeStats,
        media::video::render::renderer::XbxRenderState,
        media::video::render::renderer::XbxRenderFrame,
        media::video::test_fixtures::{
            make_bootstrap_assembled_frame, make_video_source_for_test, send_bootstrap_access_unit,
        },
        media::video::types::{
            DecodedFrame, EncodedFrame, FrameRecoveryDisposition,
        },
        transport::rtc::stream::adapter_types::FrameSource,
        XbxEngineRenderPixelData,
    };
    use bytes::Bytes;

    struct SpyHardwareDecoder {
        reset_calls: Arc<AtomicUsize>,
    }

    impl XbxHardwareVideoDecoder for SpyHardwareDecoder {
        fn backend_name(&self) -> &'static str {
            "spy"
        }

        fn decode(
            &mut self,
            _encoded_frame: EncodedFrame,
            _now_ms: f64,
        ) -> Result<Option<XbxRenderFrame>, crate::XbxEngineRuntimeError> {
            Ok(None)
        }

        fn reset(&mut self) -> Result<(), crate::XbxEngineRuntimeError> {
            self.reset_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    #[test]
    fn request_decoder_reset_calls_hardware_decoder_reset() {
        let reset_calls = Arc::new(AtomicUsize::new(0));
        let decoder = SpyHardwareDecoder {
            reset_calls: reset_calls.clone(),
        };
        let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

        state
            .request_decoder_reset()
            .expect("decoder reset should succeed");

        assert_eq!(reset_calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            state.recovery_state(),
            XbxVideoRecoveryState::WaitingKeyframe
        );
        let transition = state
            .latest_recovery_transition()
            .expect("recovery transition should exist");
        assert_eq!(
            transition.event,
            XbxVideoRecoveryEvent::ExternalResetRequested
        );
        assert_eq!(transition.to_state, XbxVideoRecoveryState::WaitingKeyframe);
    }

    #[test]
    fn recovery_fsm_moves_from_waiting_keyframe_to_recovering_then_nominal() {
        let decode_calls = Arc::new(AtomicUsize::new(0));
        let reset_calls = Arc::new(AtomicUsize::new(0));
        let mut scripted_results = VecDeque::new();
        scripted_results.push_back(Ok(Some(XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 101,
            rendered_at_ms: 101.0,
            rtp_timestamp: Some(101),
            is_keyframe: true,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([9u8; 16]),
            },
        })));
        scripted_results.push_back(Ok(Some(XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 102,
            rendered_at_ms: 102.0,
            rtp_timestamp: Some(102),
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([8u8; 16]),
            },
        })));
        let decoder = ScriptedHardwareDecoder {
            decode_calls: decode_calls.clone(),
            reset_calls: reset_calls.clone(),
            scripted_results,
        };
        let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

        state
            .request_decoder_reset()
            .expect("decoder reset should succeed");
        assert_eq!(
            state.recovery_state(),
            XbxVideoRecoveryState::WaitingKeyframe
        );

        assert!(state
            .process_encoded_frame(make_encoded_frame(false), 1_000.0)
            .is_none());
        assert_eq!(
            state.recovery_state(),
            XbxVideoRecoveryState::WaitingKeyframe
        );

        assert!(state
            .process_encoded_frame(make_encoded_frame(true), 1_016.0)
            .is_none());
        assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Recovering);
        assert_eq!(
            state
                .peek_decoded_frame()
                .map(|frame| frame.surface.frame_seq),
            Some(1)
        );
        assert_eq!(state.decoded_frame_queue_len(), 1);

        assert!(state
            .process_encoded_frame(make_encoded_frame(false), 1_032.0)
            .is_none());
        assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Nominal);
        assert_eq!(state.decoded_frame_queue_len(), 2);
        assert_eq!(
            state
                .peek_decoded_frame()
                .map(|frame| frame.surface.frame_seq),
            Some(1)
        );

        let transition = state
            .latest_recovery_transition()
            .expect("latest recovery transition should exist");
        assert_eq!(transition.event, XbxVideoRecoveryEvent::RecoverySettled);
        assert_eq!(transition.from_state, XbxVideoRecoveryState::Recovering);
        assert_eq!(transition.to_state, XbxVideoRecoveryState::Nominal);
        assert_eq!(decode_calls.load(Ordering::Relaxed), 2);
        assert_eq!(reset_calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn hardware_decode_failure_escalates_recovery_state_to_waiting_keyframe() {
        let decode_calls = Arc::new(AtomicUsize::new(0));
        let reset_calls = Arc::new(AtomicUsize::new(0));
        let mut scripted_results = VecDeque::new();
        scripted_results.push_back(Err(crate::XbxEngineRuntimeError::new(
            "xbxEngineCreateVideoFormatDescriptionFailed:status=-12909",
        )));
        let decoder = ScriptedHardwareDecoder {
            decode_calls: decode_calls.clone(),
            reset_calls: reset_calls.clone(),
            scripted_results,
        };
        let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

        let result = state.process_encoded_frame(make_encoded_frame(true), 2_000.0);
        assert!(result.is_none());
        assert_eq!(
            state.recovery_state(),
            XbxVideoRecoveryState::WaitingKeyframe
        );
        let transition = state
            .latest_recovery_transition()
            .expect("recovery transition should exist after backend failure");
        assert_eq!(
            transition.event,
            XbxVideoRecoveryEvent::BackendFailureEscalated
        );
        assert_eq!(transition.status, Some(-12909));
        assert_eq!(reset_calls.load(Ordering::Relaxed), 1);
        assert_eq!(decode_calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn repeated_nonfatal_decode_failures_escalate_to_waiting_keyframe_on_third_failure() {
        let decode_calls = Arc::new(AtomicUsize::new(0));
        let reset_calls = Arc::new(AtomicUsize::new(0));
        let mut scripted_results = VecDeque::new();
        scripted_results.push_back(Err(crate::XbxEngineRuntimeError::new(
            "decode failed status=-1",
        )));
        scripted_results.push_back(Err(crate::XbxEngineRuntimeError::new(
            "decode failed status=-1",
        )));
        scripted_results.push_back(Err(crate::XbxEngineRuntimeError::new(
            "decode failed status=-1",
        )));
        let decoder = ScriptedHardwareDecoder {
            decode_calls: decode_calls.clone(),
            reset_calls: reset_calls.clone(),
            scripted_results,
        };
        let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

        assert!(state
            .process_encoded_frame(make_encoded_frame(true), 3_000.0)
            .is_none());
        assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Nominal);

        assert!(state
            .process_encoded_frame(make_encoded_frame(true), 3_016.0)
            .is_none());
        assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Nominal);

        assert!(state
            .process_encoded_frame(make_encoded_frame(true), 3_032.0)
            .is_none());
        assert_eq!(
            state.recovery_state(),
            XbxVideoRecoveryState::WaitingKeyframe
        );
        let transition = state
            .latest_recovery_transition()
            .expect("recovery transition should exist after third failure");
        assert_eq!(
            transition.event,
            XbxVideoRecoveryEvent::BackendFailureEscalated
        );
        assert_eq!(reset_calls.load(Ordering::Relaxed), 1);
        assert_eq!(decode_calls.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn decoded_queue_keeps_latest_two_frames_under_pressure() {
        let decoder = SpyHardwareDecoder {
            reset_calls: Arc::new(AtomicUsize::new(0)),
        };
        let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

        for seq in 1..=3 {
            state.enqueue_decoded_frame_for_test(XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: seq,
                rendered_at_ms: seq as f64,
                rtp_timestamp: Some(seq as u32),
                is_keyframe: seq == 1,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([0u8; 16]),
                },
            });
        }

        assert_eq!(state.decoded_frame_queue.len(), 2);
        assert_eq!(
            state
                .decoded_frame_queue
                .front()
                .map(|frame| frame.surface.frame_seq),
            Some(2)
        );
    }

    #[test]
    fn peek_decoded_frame_keeps_head_of_queue_intact() {
        let decoder = SpyHardwareDecoder {
            reset_calls: Arc::new(AtomicUsize::new(0)),
        };
        let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

        state.enqueue_decoded_frame_for_test(XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 7,
            rendered_at_ms: 7.0,
            rtp_timestamp: Some(7),
            is_keyframe: true,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        });

        assert_eq!(
            state
                .peek_decoded_frame()
                .map(|frame| frame.surface.frame_seq),
            Some(7)
        );
        assert!(state.has_decoded_frame());
        assert_eq!(
            state
                .peek_decoded_frame()
                .map(|frame| frame.surface.frame_seq),
            Some(7)
        );
        assert_eq!(
            state
                .pop_decoded_frame(8.0)
                .map(|frame| frame.surface.frame_seq),
            Some(7)
        );
        assert!(!state.has_decoded_frame());
    }

    #[test]
    fn peek_decoded_frame_reports_front_without_consuming() {
        let decoder = SpyHardwareDecoder {
            reset_calls: Arc::new(AtomicUsize::new(0)),
        };
        let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

        state.enqueue_decoded_frame_for_test(XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 1,
            rendered_at_ms: 1.0,
            rtp_timestamp: Some(1),
            is_keyframe: true,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        });

        assert!(state.has_decoded_frame());
        assert_eq!(
            state
                .peek_decoded_frame()
                .map(|frame| frame.surface.frame_seq),
            Some(1)
        );
        assert_eq!(
            state
                .peek_decoded_frame()
                .map(|frame| frame.surface.frame_seq),
            Some(1)
        );
        assert_eq!(
            state
                .pop_decoded_frame(2.0)
                .map(|frame| frame.surface.frame_seq),
            Some(1)
        );
        assert!(!state.has_decoded_frame());
    }

    #[test]
    fn decoded_frame_queue_is_full_tracks_capacity_without_consuming() {
        let decoder = SpyHardwareDecoder {
            reset_calls: Arc::new(AtomicUsize::new(0)),
        };
        let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

        assert!(!state.decoded_frame_queue_is_full());

        for seq in 1..=2 {
            state.enqueue_decoded_frame_for_test(XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: seq,
                rendered_at_ms: seq as f64,
                rtp_timestamp: Some(seq as u32),
                is_keyframe: seq == 1,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([0u8; 16]),
                },
            });
        }

        assert!(state.has_decoded_frame());
        assert_eq!(
            state
                .peek_decoded_frame()
                .map(|frame| frame.surface.frame_seq),
            Some(1)
        );
        assert!(state.decoded_frame_queue_is_full());

        assert_eq!(
            state
                .pop_decoded_frame(3.0)
                .map(|frame| frame.surface.frame_seq),
            Some(1)
        );
        assert!(!state.decoded_frame_queue_is_full());
        assert!(state.has_decoded_frame());
    }

    #[test]
    fn requeue_decoded_frame_front_restores_head_order_after_backpressure() {
        let decoder = SpyHardwareDecoder {
            reset_calls: Arc::new(AtomicUsize::new(0)),
        };
        let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

        state.enqueue_decoded_frame_for_test(XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 11,
            rendered_at_ms: 11.0,
            rtp_timestamp: Some(11),
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        });

        let frame = state.pop_decoded_frame(12.0).expect("frame should exist");
        state.requeue_decoded_frame_front(frame);

        assert_eq!(
            state
                .peek_decoded_frame()
                .map(|frame| frame.surface.frame_seq),
            Some(11)
        );
        assert_eq!(
            state
                .pop_decoded_frame(13.0)
                .map(|frame| frame.surface.frame_seq),
            Some(11)
        );
        assert!(!state.has_decoded_frame());
    }

    #[test]
    fn enqueue_decoded_frame_returns_dropped_oldest_frame() {
        let decoder = SpyHardwareDecoder {
            reset_calls: Arc::new(AtomicUsize::new(0)),
        };
        let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

        state.enqueue_decoded_frame_for_test(XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 1,
            rendered_at_ms: 1.0,
            rtp_timestamp: Some(1),
            is_keyframe: true,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        });
        state.enqueue_decoded_frame_for_test(XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 2,
            rendered_at_ms: 2.0,
            rtp_timestamp: Some(2),
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([1u8; 16]),
            },
        });

        let dropped = state.enqueue_decoded_frame(DecodedFrame {
            pts: Instant::now(),
            rtp_timestamp: 3,
            is_keyframe: false,
            budget: crate::media::video::ingress::budget::FrameBudgetContext::default(),
            frame_recovery_disposition: FrameRecoveryDisposition::Repairing,
            frame_unrecoverable_reason: None,
            surface: XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: 3,
                rendered_at_ms: 3.0,
                rtp_timestamp: Some(3),
                is_keyframe: false,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([2u8; 16]),
                },
            },
        });

        assert_eq!(dropped.map(|frame| frame.surface.frame_seq), Some(1));
        assert_eq!(state.decoded_frame_drop_count(), 1);
        let decision = state
            .latest_decode_candidate_decision()
            .expect("candidate decision");
        assert_eq!(decision.state, XbxDecodeCandidateState::Backpressure);
        assert_eq!(decision.action, "drop");
        assert_eq!(decision.detail, "outputQueueOverflow");
        assert_eq!(decision.frame_seq, Some(1));
    }

    #[test]
    fn decode_candidate_state_recovers_to_nominal_after_pressure_is_relieved() {
        let decoder = SpyHardwareDecoder {
            reset_calls: Arc::new(AtomicUsize::new(0)),
        };
        let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

        for seq in 1..=3 {
            state.enqueue_decoded_frame_for_test(XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: seq,
                rendered_at_ms: seq as f64,
                rtp_timestamp: Some(seq as u32),
                is_keyframe: seq == 1,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([0u8; 16]),
                },
            });
        }
        let pressured = state
            .latest_decode_candidate_decision()
            .expect("backpressure decision");
        assert_eq!(pressured.state, XbxDecodeCandidateState::Backpressure);

        let _ = state.pop_decoded_frame(4.0);
        state.enqueue_decoded_frame_for_test(XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 4,
            rendered_at_ms: 4.0,
            rtp_timestamp: Some(4),
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([1u8; 16]),
            },
        });
        let recovered = state
            .latest_decode_candidate_decision()
            .expect("recovered decision");
        assert_eq!(recovered.state, XbxDecodeCandidateState::Nominal);
        assert_eq!(recovered.action, "accept");
        assert_eq!(recovered.detail, "queueRecovered");
        assert_eq!(recovered.frame_seq, Some(4));
    }

    #[test]
    fn workload_snapshot_switches_to_drain_output_when_queue_is_non_empty() {
        let decoder = SpyHardwareDecoder {
            reset_calls: Arc::new(AtomicUsize::new(0)),
        };
        let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

        let initial = state.workload_snapshot();
        assert_eq!(initial.state, XbxDecodeWorkloadState::AwaitingInput);
        assert_eq!(initial.pending_output_queue_depth, 0);
        assert!(!initial.should_drain_output_first());

        state.enqueue_decoded_frame_for_test(XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 1,
            rendered_at_ms: 1.0,
            rtp_timestamp: Some(1),
            is_keyframe: true,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        });

        let queued = state.workload_snapshot();
        assert_eq!(queued.state, XbxDecodeWorkloadState::DrainOutput);
        assert_eq!(queued.pending_output_queue_depth, 1);
        assert!(queued.should_drain_output_first());

        let _ = state.pop_decoded_frame(2.0);
        let drained = state.workload_snapshot();
        assert_eq!(drained.state, XbxDecodeWorkloadState::AwaitingInput);
        assert_eq!(drained.pending_output_queue_depth, 0);
        assert!(!drained.should_drain_output_first());
    }

    struct ScriptedHardwareDecoder {
        decode_calls: Arc<AtomicUsize>,
        reset_calls: Arc<AtomicUsize>,
        scripted_results: VecDeque<Result<Option<XbxRenderFrame>, crate::XbxEngineRuntimeError>>,
    }

    impl XbxHardwareVideoDecoder for ScriptedHardwareDecoder {
        fn backend_name(&self) -> &'static str {
            "scripted"
        }

        fn decode(
            &mut self,
            _encoded_frame: EncodedFrame,
            _now_ms: f64,
        ) -> Result<Option<XbxRenderFrame>, crate::XbxEngineRuntimeError> {
            self.decode_calls.fetch_add(1, Ordering::Relaxed);
            self.scripted_results.pop_front().unwrap_or(Ok(None))
        }

        fn reset(&mut self) -> Result<(), crate::XbxEngineRuntimeError> {
            self.reset_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    fn make_encoded_frame(is_keyframe: bool) -> EncodedFrame {
        let now = Instant::now();
        EncodedFrame {
            codec: crate::media::video::types::VideoCodec::H264,
            is_keyframe,
            config_changed: false,
            value: crate::media::video::types::FrameValue::new(is_keyframe, false, 1024),
            budget: crate::media::video::ingress::budget::FrameBudgetContext::steady_for_value(
                crate::media::video::types::FrameValue::new(is_keyframe, false, 1024),
            ),
            width: 2560,
            height: 1440,
            rtp_timestamp: if is_keyframe { 1 } else { 2 },
            frame_playout_deadline_at_ms: None,
            frame_recovery_disposition:
                crate::media::video::types::FrameRecoveryDisposition::Repairing,
            frame_unrecoverable_reason: None,
            target_playout_time: now + Duration::from_millis(16),
            h264: make_h264_inspection(is_keyframe),
            payload: Bytes::from_static(b"\x00\x00\x00\x01\x65"),
        }
    }

    fn make_h264_inspection(bootstrap_ready: bool) -> H264AccessUnitInspection {
        H264AccessUnitInspection {
            nals: Vec::new(),
            parameter_sets: None,
            width: Some(2560),
            height: Some(1440),
            is_idr: bootstrap_ready,
            has_inband_sps: bootstrap_ready,
            has_inband_pps: bootstrap_ready,
            slice_headers_valid: bootstrap_ready,
            parameter_sets_changed: false,
            config_changed: false,
            bootstrap_ready,
            bootstrap_reject_reason: if bootstrap_ready {
                None
            } else {
                Some(H264BootstrapRejectReason::MissingSps)
            },
            commit_state: H264AccessUnitInspector::test_commit_state(),
        }
    }

    #[test]
    fn bad_data_failure_waits_for_next_keyframe_before_decoding_again() {
        let decode_calls = Arc::new(AtomicUsize::new(0));
        let reset_calls = Arc::new(AtomicUsize::new(0));
        let decoder = ScriptedHardwareDecoder {
            decode_calls: decode_calls.clone(),
            reset_calls: reset_calls.clone(),
            scripted_results: VecDeque::from([
                Err(crate::XbxEngineRuntimeError::new(
                    "xbxEngineVideoToolboxOutputCallbackFailed:status=-12909",
                )),
                Ok(None),
            ]),
        };
        let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

        state.process_encoded_frame(make_encoded_frame(true), 1_000.0);
        assert_eq!(decode_calls.load(Ordering::Relaxed), 1);
        assert_eq!(reset_calls.load(Ordering::Relaxed), 1);
        assert_eq!(state.decoder_reset_count(), 1);

        state.process_encoded_frame(make_encoded_frame(false), 1_016.0);
        assert_eq!(decode_calls.load(Ordering::Relaxed), 1);

        state.process_encoded_frame(make_encoded_frame(true), 1_032.0);
        assert_eq!(decode_calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn assembled_bootstrap_frame_decodes_and_renders_end_to_end() {
        let decoder = ScriptedHardwareDecoder {
            decode_calls: Arc::new(AtomicUsize::new(0)),
            reset_calls: Arc::new(AtomicUsize::new(0)),
            scripted_results: VecDeque::from([Ok(Some(XbxRenderFrame {
                width: 64,
                height: 64,
                frame_seq: 0,
                rendered_at_ms: 0.0,
                rtp_timestamp: None,
                is_keyframe: false,
                frame_recovery_disposition: None,
                frame_unrecoverable_reason: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from(vec![7u8; 64 * 64 * 4]),
                },
            }))]),
        };
        let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));
        let encoded = make_bootstrap_assembled_frame(9000)
            .into_encoded_frame(Instant::now() + Duration::from_millis(16));

        assert!(state.process_encoded_frame(encoded, 1_000.0).is_none());
        let decoded = state
            .pop_decoded_frame(1_016.0)
            .expect("decoded frame should be queued");
        assert_eq!(decoded.surface.frame_seq, 1);
        assert_eq!(decoded.surface.rtp_timestamp, Some(9000));
        assert!(decoded.surface.is_keyframe);
        assert_eq!(
            decoded.surface.frame_recovery_disposition.as_deref(),
            Some("repairing")
        );

        let mut render_state = XbxRenderState::default();
        let (_stats, outcome) = render_state
            .present_frame(decoded.surface)
            .expect("render should accept decoded frame");
        assert!(!outcome.overwritten_previous_latest);
        assert_eq!(
            render_state.peek_latest_frame().map(|frame| frame.frame_seq),
            Some(1)
        );
    }

    #[test]
    fn backend_failure_then_clean_bootstrap_frames_recover_pipeline_to_nominal() {
        let decoder = ScriptedHardwareDecoder {
            decode_calls: Arc::new(AtomicUsize::new(0)),
            reset_calls: Arc::new(AtomicUsize::new(0)),
            scripted_results: VecDeque::from([
                Err(crate::XbxEngineRuntimeError::new(
                    "xbxEngineCreateVideoFormatDescriptionFailed:status=-12909",
                )),
                Ok(Some(XbxRenderFrame {
                    width: 64,
                    height: 64,
                    frame_seq: 0,
                    rendered_at_ms: 0.0,
                    rtp_timestamp: None,
                    is_keyframe: false,
                    frame_recovery_disposition: None,
                    frame_unrecoverable_reason: None,
                    pixel_data: XbxEngineRenderPixelData::Rgba {
                        bytes: Arc::<[u8]>::from(vec![5u8; 64 * 64 * 4]),
                    },
                })),
                Ok(Some(XbxRenderFrame {
                    width: 64,
                    height: 64,
                    frame_seq: 0,
                    rendered_at_ms: 0.0,
                    rtp_timestamp: None,
                    is_keyframe: false,
                    frame_recovery_disposition: None,
                    frame_unrecoverable_reason: None,
                    pixel_data: XbxEngineRenderPixelData::Rgba {
                        bytes: Arc::<[u8]>::from(vec![6u8; 64 * 64 * 4]),
                    },
                })),
            ]),
        };
        let mut state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));

        let first = make_bootstrap_assembled_frame(9000)
            .into_encoded_frame(Instant::now() + Duration::from_millis(16));
        let second = make_bootstrap_assembled_frame(9016)
            .into_encoded_frame(Instant::now() + Duration::from_millis(32));
        let third = make_bootstrap_assembled_frame(9032)
            .into_encoded_frame(Instant::now() + Duration::from_millis(48));

        assert!(state.process_encoded_frame(first, 1_000.0).is_none());
        assert_eq!(
            state.recovery_state(),
            XbxVideoRecoveryState::WaitingKeyframe
        );

        assert!(state.process_encoded_frame(second, 1_016.0).is_none());
        assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Recovering);

        assert!(state.process_encoded_frame(third, 1_032.0).is_none());
        assert_eq!(state.recovery_state(), XbxVideoRecoveryState::Nominal);

        let first_recovered = state
            .pop_decoded_frame(1_040.0)
            .expect("first recovered frame should exist");
        let second_recovered = state
            .pop_decoded_frame(1_048.0)
            .expect("second recovered frame should exist");
        assert_eq!(first_recovered.surface.frame_seq, 1);
        assert_eq!(second_recovered.surface.frame_seq, 2);

        let mut render_state = XbxRenderState::default();
        render_state
            .present_frame(first_recovered.surface)
            .expect("first recovered frame should render");
        render_state
            .present_frame(second_recovered.surface)
            .expect("second recovered frame should render");
        assert_eq!(
            render_state.peek_latest_frame().map(|frame| frame.frame_seq),
            Some(2)
        );
    }

    #[tokio::test]
    async fn rtp_to_decode_to_pacer_to_renderer_pipeline_reaches_latest_frame_and_overwrite_signal() {
        let (tx, _transport_observation_rx, mut source) = make_video_source_for_test();
        let runtime_stats = Arc::new(std::sync::Mutex::new(XbxEngineMediaRuntimeStats::default()));

        send_bootstrap_access_unit(&tx, 100, 9000).await;
        send_bootstrap_access_unit(&tx, 103, 9016).await;
        send_bootstrap_access_unit(&tx, 106, 9032).await;
        // 再送一个后续 AU，确保前一个 sample 被 SampleBuilder 刷出。
        send_bootstrap_access_unit(&tx, 109, 9048).await;
        drop(tx);

        let decoder = ScriptedHardwareDecoder {
            decode_calls: Arc::new(AtomicUsize::new(0)),
            reset_calls: Arc::new(AtomicUsize::new(0)),
            scripted_results: VecDeque::from([
                Ok(Some(XbxRenderFrame {
                    width: 64,
                    height: 64,
                    frame_seq: 0,
                    rendered_at_ms: 0.0,
                    rtp_timestamp: None,
                    is_keyframe: false,
                    frame_recovery_disposition: None,
                    frame_unrecoverable_reason: None,
                    pixel_data: XbxEngineRenderPixelData::Rgba {
                        bytes: Arc::<[u8]>::from(vec![1u8; 64 * 64 * 4]),
                    },
                })),
                Ok(Some(XbxRenderFrame {
                    width: 64,
                    height: 64,
                    frame_seq: 0,
                    rendered_at_ms: 0.0,
                    rtp_timestamp: None,
                    is_keyframe: false,
                    frame_recovery_disposition: None,
                    frame_unrecoverable_reason: None,
                    pixel_data: XbxEngineRenderPixelData::Rgba {
                        bytes: Arc::<[u8]>::from(vec![2u8; 64 * 64 * 4]),
                    },
                })),
                Ok(Some(XbxRenderFrame {
                    width: 64,
                    height: 64,
                    frame_seq: 0,
                    rendered_at_ms: 0.0,
                    rtp_timestamp: None,
                    is_keyframe: false,
                    frame_recovery_disposition: None,
                    frame_unrecoverable_reason: None,
                    pixel_data: XbxEngineRenderPixelData::Rgba {
                        bytes: Arc::<[u8]>::from(vec![3u8; 64 * 64 * 4]),
                    },
                })),
            ]),
        };
        let mut decode_state = XbxVideoDecodeState::new_for_test(20, 30, Box::new(decoder));
        let render_state = Arc::new(std::sync::Mutex::new(XbxRenderState::default()));
        let renderer = Arc::new(crate::media::video::render::actor::RendererActorHandle::new(
            render_state.clone(),
            runtime_stats.clone(),
        ));
        let pacer = crate::media::video::pacer::actor::PacerActorHandle::new(
            renderer.clone(),
            runtime_stats.clone(),
            16,
        );

        for expected_timestamp in [9000u32, 9016u32, 9032u32] {
            let assembled = tokio::time::timeout(Duration::from_millis(250), source.recv_frame())
                .await
                .expect("source should assemble frame in time")
                .expect("assembled frame should exist");
            assert_eq!(assembled.rtp_timestamp, expected_timestamp);
            let encoded = assembled.into_encoded_frame(Instant::now());
            assert!(decode_state.process_encoded_frame(encoded, expected_timestamp as f64).is_none());
            let decoded = decode_state
                .pop_decoded_frame(expected_timestamp as f64 + 1.0)
                .expect("decoded frame should be available");

            let submit_deadline = Instant::now() + Duration::from_millis(150);
            let mut submitted = false;
            while Instant::now() < submit_deadline {
                match pacer.submit(decoded.clone()) {
                    Ok(_) => {
                        submitted = true;
                        break;
                    }
                    Err(std::sync::mpsc::TrySendError::Full(_)) => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(err) => panic!("unexpected pacer submit failure: {err:?}"),
                }
            }
            assert!(submitted, "decoded frame should eventually reach pacer");
        }

        let render_deadline = Instant::now() + Duration::from_millis(300);
        let mut latest_seq = None;
        while Instant::now() < render_deadline {
            let frame = render_state
                .lock()
                .expect("render state lock")
                .take_latest_frame();
            if let Some(frame) = frame {
                latest_seq = Some(frame.frame_seq);
                if frame.frame_seq >= 3 {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(4));
        }

        pacer.stop();
        renderer.stop();

        assert_eq!(latest_seq, Some(3));
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert!(stats.video_renderer_submit_count_total >= 2);
        let decision = stats
            .latest_render_candidate_decision
            .clone()
            .expect("render candidate decision should exist");
        assert_eq!(decision.state, "latest-overwrite");
        assert_eq!(decision.detail, "latestSlotOverwrite");
    }
}
