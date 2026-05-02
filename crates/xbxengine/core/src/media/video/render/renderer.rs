use crate::{
    XbxEngineRenderFrame, XbxEngineRenderPixelData, XbxEngineReplacementDecisionObservation,
    XbxEngineRuntimeError, XbxEngineVideoFrameStats,
};
use xbxengine_protocol::XbxEngineDisplayStateDto;
#[allow(dead_code)]
const RENDER_STALL_THRESHOLD_MS: f64 = 1_500.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum XbxRenderMailboxState {
    #[default]
    Nominal,
    LatestOverwrite,
}

impl XbxRenderMailboxState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Nominal => "nominal",
            Self::LatestOverwrite => "latest-overwrite",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct XbxRenderMailboxDecisionSnapshot {
    pub(crate) decision_id: u64,
    pub(crate) state: XbxRenderMailboxState,
    pub(crate) action: &'static str,
    pub(crate) detail: &'static str,
    pub(crate) frame_seq: Option<u64>,
    pub(crate) replacement_decision: Option<XbxEngineReplacementDecisionObservation>,
    pub(crate) observed_at_ms: f64,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct XbxRenderSignalSnapshot {
    pub latest_present_time_ms: Option<f64>,
    pub renderer_stalled: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct XbxPresentFrameOutcome {
    pub(crate) overwritten_pending_frame: bool,
    pub(crate) overwritten_frame_seq: Option<u64>,
    pub(crate) overwritten_frame_width: Option<u32>,
    pub(crate) overwritten_frame_height: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct XbxRenderFrame {
    pub width: u32,
    pub height: u32,
    pub frame_seq: u64,
    pub rendered_at_ms: f64,
    pub rtp_timestamp: Option<u32>,
    pub recovery_epoch_tag: Option<u64>,
    pub recovery_owner_rtp_timestamp: Option<u32>,
    pub is_keyframe: bool,
    pub frame_recovery_disposition: Option<String>,
    pub frame_unrecoverable_reason: Option<String>,
    pub presentation_value_role: Option<String>,
    pub pixel_data: XbxEngineRenderPixelData,
}

impl XbxRenderFrame {
    #[allow(dead_code)]
    pub(crate) fn video_stats(&self) -> XbxEngineVideoFrameStats {
        XbxEngineVideoFrameStats {
            width: self.width,
            height: self.height,
            frame_seq: self.frame_seq,
            fps: 0.0,
            rendered_at_ms: self.rendered_at_ms,
        }
    }
}

impl From<XbxRenderFrame> for XbxEngineRenderFrame {
    fn from(value: XbxRenderFrame) -> Self {
        Self {
            width: value.width,
            height: value.height,
            frame_seq: value.frame_seq,
            rendered_at_ms: value.rendered_at_ms,
            rtp_timestamp: value.rtp_timestamp,
            recovery_epoch_tag: value.recovery_epoch_tag,
            recovery_owner_rtp_timestamp: value.recovery_owner_rtp_timestamp,
            is_keyframe: value.is_keyframe,
            frame_recovery_disposition: value.frame_recovery_disposition,
            frame_unrecoverable_reason: value.frame_unrecoverable_reason,
            presentation_value_role: value.presentation_value_role,
            pixel_data: value.pixel_data,
        }
    }
}

/**
 * `core` 只负责“render 影子态 + host handoff staging queue”和显示状态同步。
 * 真实上传/present 由宿主侧渲染器负责，避免同一帧在 Rust 内重复上传。
 */
#[derive(Default)]
pub(crate) struct XbxRenderState {
    latest_display_state: Option<XbxEngineDisplayStateDto>,
    latest_renderable_frame: Option<XbxEngineRenderFrame>,
    render_mailbox_state: XbxRenderMailboxState,
    latest_render_mailbox_decision: Option<XbxRenderMailboxDecisionSnapshot>,
    render_mailbox_decision_id: u64,
}

impl XbxRenderState {
    pub(crate) fn reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.latest_display_state = None;
        self.latest_renderable_frame = None;
        self.render_mailbox_state = XbxRenderMailboxState::Nominal;
        self.latest_render_mailbox_decision = None;
        self.render_mailbox_decision_id = 0;
        Ok(())
    }

    pub(crate) fn apply_display_state(
        &mut self,
        state: XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.latest_display_state = Some(state);
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn present_frame(
        &mut self,
        frame: XbxRenderFrame,
    ) -> Result<(XbxEngineVideoFrameStats, XbxPresentFrameOutcome), XbxEngineRuntimeError> {
        match &frame.pixel_data {
            XbxEngineRenderPixelData::Rgba { bytes } | XbxEngineRenderPixelData::Bgra { bytes } => {
                let expected_len = frame.width as usize * frame.height as usize * 4;
                if expected_len != bytes.len() {
                    return Err(XbxEngineRuntimeError::new(format!(
                        "xbxEngineFrameSizeMismatch:expected={expected_len}:actual={}",
                        bytes.len()
                    )));
                }
            }
            XbxEngineRenderPixelData::Nv12 {
                y_plane,
                uv_plane,
                y_stride,
                uv_stride,
            } => {
                let y_height = frame.height as usize;
                let uv_height = frame.height.div_ceil(2) as usize;
                let y_expected_min = (*y_stride as usize).saturating_mul(y_height);
                let uv_expected_min = (*uv_stride as usize).saturating_mul(uv_height);
                if y_plane.len() < y_expected_min || uv_plane.len() < uv_expected_min {
                    return Err(XbxEngineRuntimeError::new(format!(
                        "xbxEngineNv12FrameSizeMismatch:y_min={y_expected_min}:y_actual={}:uv_min={uv_expected_min}:uv_actual={}",
                        y_plane.len(),
                        uv_plane.len()
                    )));
                }
            }
            XbxEngineRenderPixelData::Descriptor { .. } => {
                // 原生描述符不需要在 core 层进行字节长度校验
            }
        }
        let frame_stats = frame.video_stats();
        let presented_frame_seq = frame.frame_seq;
        let observed_at_ms = frame.rendered_at_ms;
        // `pacer` 已完成候选价值排序；render latest-slot 只保留单槽交接语义。
        let overwritten_frame = self
            .latest_renderable_frame
            .as_ref()
            .map(|frame| (frame.frame_seq, frame.width, frame.height));
        let overwritten_pending_frame = overwritten_frame.is_some();
        let overwritten_frame_contract = self.latest_renderable_frame.as_ref().map(|existing| {
            XbxEngineReplacementDecisionObservation {
                dropped_frame_seq: Some(existing.frame_seq),
                dropped_rtp_timestamp: existing.rtp_timestamp,
                dropped_presentation_value_role: existing.presentation_value_role.clone(),
                kept_frame_seq: Some(frame.frame_seq),
                kept_rtp_timestamp: frame.rtp_timestamp,
                kept_presentation_value_role: frame.presentation_value_role.clone(),
                same_recovery_epoch: Some(existing.recovery_epoch_tag == frame.recovery_epoch_tag),
                same_recovery_owner_chain: Some(
                    existing.recovery_epoch_tag == frame.recovery_epoch_tag
                        && existing.recovery_owner_rtp_timestamp
                            == frame.recovery_owner_rtp_timestamp,
                ),
                supersede_reason: Some("mailboxLatestExecution".to_string()),
            }
        });
        let engine_frame: XbxEngineRenderFrame = frame.into();
        self.latest_renderable_frame = Some(engine_frame);
        if overwritten_pending_frame {
            self.record_render_mailbox_decision(
                XbxRenderMailboxState::LatestOverwrite,
                "replace",
                "mailboxOverwrite",
                overwritten_frame.map(|frame| frame.0),
                overwritten_frame_contract,
                observed_at_ms,
            );
        } else if matches!(
            self.render_mailbox_state,
            XbxRenderMailboxState::LatestOverwrite
        ) {
            self.record_render_mailbox_decision(
                XbxRenderMailboxState::Nominal,
                "accept",
                "mailboxRecovered",
                Some(presented_frame_seq),
                None,
                observed_at_ms,
            );
        }
        Ok((
            XbxEngineVideoFrameStats {
                fps: 0.0,
                ..frame_stats
            },
            XbxPresentFrameOutcome {
                overwritten_pending_frame,
                overwritten_frame_seq: overwritten_frame.map(|frame| frame.0),
                overwritten_frame_width: overwritten_frame.map(|frame| frame.1),
                overwritten_frame_height: overwritten_frame.map(|frame| frame.2),
            },
        ))
    }

    pub(crate) fn stop(&mut self) {
        self.latest_display_state = None;
        self.latest_renderable_frame = None;
        self.render_mailbox_state = XbxRenderMailboxState::Nominal;
        self.latest_render_mailbox_decision = None;
        self.render_mailbox_decision_id = 0;
    }

    pub(crate) fn take_latest_renderable_frame(&mut self) -> Option<XbxEngineRenderFrame> {
        self.latest_renderable_frame.take()
    }

    // 非消费读取：供上层在不丢帧的情况下查看当前 latest-slot。
    #[allow(dead_code)]
    pub(crate) fn peek_latest_frame(&self) -> Option<&XbxEngineRenderFrame> {
        self.latest_renderable_frame.as_ref()
    }

    #[allow(dead_code)]
    pub(crate) fn render_signal_snapshot(&self, now_ms: f64) -> XbxRenderSignalSnapshot {
        let latest_present_time_ms = self
            .latest_renderable_frame
            .as_ref()
            .map(|frame| frame.rendered_at_ms);
        let renderer_stalled = latest_present_time_ms.map(|presented_at_ms| {
            (now_ms - presented_at_ms).max(0.0) >= RENDER_STALL_THRESHOLD_MS
        });
        XbxRenderSignalSnapshot {
            latest_present_time_ms,
            renderer_stalled,
        }
    }

    pub(crate) fn latest_render_mailbox_decision(
        &self,
    ) -> Option<&XbxRenderMailboxDecisionSnapshot> {
        self.latest_render_mailbox_decision.as_ref()
    }

    fn record_render_mailbox_decision(
        &mut self,
        state: XbxRenderMailboxState,
        action: &'static str,
        detail: &'static str,
        frame_seq: Option<u64>,
        replacement_decision: Option<XbxEngineReplacementDecisionObservation>,
        observed_at_ms: f64,
    ) {
        self.render_mailbox_state = state;
        self.render_mailbox_decision_id = self.render_mailbox_decision_id.saturating_add(1);
        self.latest_render_mailbox_decision = Some(XbxRenderMailboxDecisionSnapshot {
            decision_id: self.render_mailbox_decision_id,
            state,
            action,
            detail,
            frame_seq,
            replacement_decision,
            observed_at_ms,
        });
    }
}

#[cfg(test)]
#[path = "renderer.test.rs"]
mod tests;
