use crate::{
    XbxEngineRenderFrame, XbxEngineRenderPixelData, XbxEngineRuntimeError, XbxEngineVideoFrameStats,
};
use xbxengine_protocol::XbxEngineDisplayStateDto;
#[allow(dead_code)]
const RENDER_STALL_THRESHOLD_MS: f64 = 1_500.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum XbxRenderCandidateState {
    #[default]
    Nominal,
    LatestOverwrite,
}

impl XbxRenderCandidateState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Nominal => "nominal",
            Self::LatestOverwrite => "latest-overwrite",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct XbxRenderCandidateDecisionSnapshot {
    pub(crate) decision_id: u64,
    pub(crate) state: XbxRenderCandidateState,
    pub(crate) action: &'static str,
    pub(crate) detail: &'static str,
    pub(crate) frame_seq: Option<u64>,
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
    pub(crate) overwritten_previous_latest: bool,
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
    pub is_keyframe: bool,
    pub frame_recovery_disposition: Option<String>,
    pub frame_unrecoverable_reason: Option<String>,
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
            is_keyframe: value.is_keyframe,
            frame_recovery_disposition: value.frame_recovery_disposition,
            frame_unrecoverable_reason: value.frame_unrecoverable_reason,
            pixel_data: value.pixel_data,
        }
    }
}

/**
 * `core` 只负责“最新帧缓存”和显示状态同步，不在这里做 GPU 上传。
 * 真实上传/present 由宿主侧渲染器负责，避免同一帧在 Rust 内重复上传。
 */
#[derive(Default)]
pub(crate) struct XbxRenderState {
    latest_display_state: Option<XbxEngineDisplayStateDto>,
    latest_frame: Option<XbxEngineRenderFrame>,
    last_acknowledged_present_time_ms: Option<f64>,
    render_candidate_state: XbxRenderCandidateState,
    latest_render_candidate_decision: Option<XbxRenderCandidateDecisionSnapshot>,
    render_candidate_decision_id: u64,
}

impl XbxRenderState {
    pub(crate) fn reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.latest_display_state = None;
        self.latest_frame = None;
        self.last_acknowledged_present_time_ms = None;
        self.render_candidate_state = XbxRenderCandidateState::Nominal;
        self.latest_render_candidate_decision = None;
        self.render_candidate_decision_id = 0;
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
        let overwritten_previous_latest = self.latest_frame.is_some();
        let overwritten_frame = self
            .latest_frame
            .as_ref()
            .map(|frame| (frame.frame_seq, frame.width, frame.height));
        self.latest_frame = Some(frame.into());
        if overwritten_previous_latest {
            self.record_render_candidate_decision(
                XbxRenderCandidateState::LatestOverwrite,
                "replace",
                "latestSlotOverwrite",
                overwritten_frame.map(|frame| frame.0),
                observed_at_ms,
            );
        } else if matches!(
            self.render_candidate_state,
            XbxRenderCandidateState::LatestOverwrite
        ) {
            self.record_render_candidate_decision(
                XbxRenderCandidateState::Nominal,
                "accept",
                "latestSlotRecovered",
                Some(presented_frame_seq),
                observed_at_ms,
            );
        }
        Ok((
            XbxEngineVideoFrameStats {
                fps: 0.0,
                ..frame_stats
            },
            XbxPresentFrameOutcome {
                overwritten_previous_latest,
                overwritten_frame_seq: overwritten_frame.map(|frame| frame.0),
                overwritten_frame_width: overwritten_frame.map(|frame| frame.1),
                overwritten_frame_height: overwritten_frame.map(|frame| frame.2),
            },
        ))
    }

    pub(crate) fn stop(&mut self) {
        self.latest_display_state = None;
        self.latest_frame = None;
        self.last_acknowledged_present_time_ms = None;
        self.render_candidate_state = XbxRenderCandidateState::Nominal;
        self.latest_render_candidate_decision = None;
        self.render_candidate_decision_id = 0;
    }

    pub(crate) fn take_latest_frame(&mut self) -> Option<XbxEngineRenderFrame> {
        // latest-slot 语义：读取最新帧不应清空槽位，避免上层消费时把渲染状态误判为“无帧”。
        self.latest_frame.clone()
    }

    // 非消费读取：供上层在不丢帧的情况下查看当前 latest-slot。
    #[allow(dead_code)]
    pub(crate) fn peek_latest_frame(&self) -> Option<&XbxEngineRenderFrame> {
        self.latest_frame.as_ref()
    }

    // 可选 ACK 语义：只有序号匹配当前 latest-slot 时才清空，避免误删新帧。
    #[allow(dead_code)]
    pub(crate) fn acknowledge_latest_frame(&mut self, frame_seq: u64) -> bool {
        if self
            .latest_frame
            .as_ref()
            .is_some_and(|frame| frame.frame_seq == frame_seq)
        {
            self.last_acknowledged_present_time_ms =
                self.latest_frame.as_ref().map(|frame| frame.rendered_at_ms);
            self.latest_frame = None;
            return true;
        }
        false
    }

    #[allow(dead_code)]
    pub(crate) fn render_signal_snapshot(&self, now_ms: f64) -> XbxRenderSignalSnapshot {
        let latest_present_time_ms = self
            .last_acknowledged_present_time_ms
            .or_else(|| self.latest_frame.as_ref().map(|frame| frame.rendered_at_ms));
        let renderer_stalled = latest_present_time_ms.map(|presented_at_ms| {
            (now_ms - presented_at_ms).max(0.0) >= RENDER_STALL_THRESHOLD_MS
        });
        XbxRenderSignalSnapshot {
            latest_present_time_ms,
            renderer_stalled,
        }
    }

    pub(crate) fn latest_render_candidate_decision(
        &self,
    ) -> Option<&XbxRenderCandidateDecisionSnapshot> {
        self.latest_render_candidate_decision.as_ref()
    }

    fn record_render_candidate_decision(
        &mut self,
        state: XbxRenderCandidateState,
        action: &'static str,
        detail: &'static str,
        frame_seq: Option<u64>,
        observed_at_ms: f64,
    ) {
        self.render_candidate_state = state;
        self.render_candidate_decision_id = self.render_candidate_decision_id.saturating_add(1);
        self.latest_render_candidate_decision = Some(XbxRenderCandidateDecisionSnapshot {
            decision_id: self.render_candidate_decision_id,
            state,
            action,
            detail,
            frame_seq,
            observed_at_ms,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{XbxPresentFrameOutcome, XbxRenderFrame, XbxRenderState};
    use crate::XbxEngineRenderPixelData;

    #[test]
    fn latest_slot_supports_peek_take_and_ack() {
        let mut state = XbxRenderState::default();
        let frame = XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 1,
            rendered_at_ms: 1_000.0,
            rtp_timestamp: Some(1),
            is_keyframe: true,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        };
        state
            .present_frame(frame)
            .expect("present frame should work");

        assert_eq!(
            state.peek_latest_frame().map(|frame| frame.frame_seq),
            Some(1)
        );
        assert!(!state.acknowledge_latest_frame(2));
        assert!(state.acknowledge_latest_frame(1));
        assert!(state.peek_latest_frame().is_none());

        let frame = XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 3,
            rendered_at_ms: 1_016.0,
            rtp_timestamp: Some(3),
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([1u8; 16]),
            },
        };
        state
            .present_frame(frame)
            .expect("present frame should work");
        assert_eq!(
            state.take_latest_frame().map(|frame| frame.frame_seq),
            Some(3)
        );
        // take 不消费槽位，后续仍可 peek/ack。
        assert_eq!(
            state.peek_latest_frame().map(|frame| frame.frame_seq),
            Some(3)
        );
        assert!(state.acknowledge_latest_frame(3));
        assert!(state.peek_latest_frame().is_none());
    }

    #[test]
    fn present_frame_reports_overwritten_latest_metadata() {
        let mut state = XbxRenderState::default();
        let first_frame = XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 1,
            rendered_at_ms: 1_000.0,
            rtp_timestamp: Some(1),
            is_keyframe: true,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        };
        let second_frame = XbxRenderFrame {
            width: 4,
            height: 4,
            frame_seq: 2,
            rendered_at_ms: 1_016.0,
            rtp_timestamp: Some(2),
            is_keyframe: false,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([1u8; 64]),
            },
        };

        let (_, first_outcome) = state
            .present_frame(first_frame)
            .expect("first present should work");
        let (_, second_outcome) = state
            .present_frame(second_frame)
            .expect("second present should work");

        assert_eq!(
            first_outcome,
            XbxPresentFrameOutcome {
                overwritten_previous_latest: false,
                overwritten_frame_seq: None,
                overwritten_frame_width: None,
                overwritten_frame_height: None,
            }
        );
        assert_eq!(
            second_outcome,
            XbxPresentFrameOutcome {
                overwritten_previous_latest: true,
                overwritten_frame_seq: Some(1),
                overwritten_frame_width: Some(2),
                overwritten_frame_height: Some(2),
            }
        );
    }

    #[test]
    fn acknowledge_keeps_last_present_time_for_snapshot() {
        let mut state = XbxRenderState::default();
        state
            .present_frame(XbxRenderFrame {
                width: 2,
                height: 2,
                frame_seq: 1,
                rendered_at_ms: 1_000.0,
                rtp_timestamp: Some(1),
                is_keyframe: true,
                frame_recovery_disposition: Some("repairing".to_string()),
                frame_unrecoverable_reason: None,
                pixel_data: XbxEngineRenderPixelData::Rgba {
                    bytes: Arc::<[u8]>::from([0u8; 16]),
                },
            })
            .expect("present frame should work");

        assert!(state.acknowledge_latest_frame(1));
        let snapshot = state.render_signal_snapshot(1_200.0);

        assert_eq!(snapshot.latest_present_time_ms, Some(1_000.0));
        assert_eq!(snapshot.renderer_stalled, Some(false));
        assert!(state.peek_latest_frame().is_none());
    }

    #[test]
    fn render_signal_snapshot_marks_stall_after_threshold() {
        let mut state = XbxRenderState::default();
        let frame = XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 1,
            rendered_at_ms: 1_000.0,
            rtp_timestamp: Some(1),
            is_keyframe: true,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        };
        state
            .present_frame(frame)
            .expect("present frame should work");
        let snapshot = state.render_signal_snapshot(2_700.0);
        assert_eq!(snapshot.latest_present_time_ms, Some(1_000.0));
        assert_eq!(snapshot.renderer_stalled, Some(true));
    }

    #[test]
    fn render_signal_snapshot_reports_latest_present_time_when_recent() {
        let mut state = XbxRenderState::default();
        for index in 0..4u64 {
            state
                .present_frame(XbxRenderFrame {
                    width: 2,
                    height: 2,
                    frame_seq: index + 1,
                    rendered_at_ms: 1_000.0 + index as f64 * 16.0,
                    rtp_timestamp: Some(index as u32 + 1),
                    is_keyframe: index == 0,
                    frame_recovery_disposition: Some("repairing".to_string()),
                    frame_unrecoverable_reason: None,
                    pixel_data: XbxEngineRenderPixelData::Rgba {
                        bytes: Arc::<[u8]>::from([0u8; 16]),
                    },
                })
                .expect("present frame should work");
        }

        let snapshot = state.render_signal_snapshot(1_050.0);
        assert_eq!(snapshot.latest_present_time_ms, Some(1_048.0));
        assert_eq!(snapshot.renderer_stalled, Some(false));
    }

    #[test]
    fn render_signal_snapshot_marks_stall_when_latest_present_is_stale() {
        let mut state = XbxRenderState::default();
        for index in 0..4u64 {
            state
                .present_frame(XbxRenderFrame {
                    width: 2,
                    height: 2,
                    frame_seq: index + 1,
                    rendered_at_ms: 1_000.0 + index as f64 * 16.0,
                    rtp_timestamp: Some(index as u32 + 1),
                    is_keyframe: index == 0,
                    frame_recovery_disposition: Some("repairing".to_string()),
                    frame_unrecoverable_reason: None,
                    pixel_data: XbxEngineRenderPixelData::Rgba {
                        bytes: Arc::<[u8]>::from([0u8; 16]),
                    },
                })
                .expect("present frame should work");
        }

        let snapshot = state.render_signal_snapshot(2_200.0);
        assert_eq!(snapshot.latest_present_time_ms, Some(1_048.0));
        assert_eq!(snapshot.renderer_stalled, Some(false));

        let stalled_snapshot = state.render_signal_snapshot(2_700.0);
        assert_eq!(stalled_snapshot.latest_present_time_ms, Some(1_048.0));
        assert_eq!(stalled_snapshot.renderer_stalled, Some(true));
    }

    #[test]
    fn render_candidate_state_recovers_after_latest_slot_overwrite_is_cleared() {
        let mut state = XbxRenderState::default();
        let mk_frame = |frame_seq: u64, rendered_at_ms: f64| XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq,
            rendered_at_ms,
            rtp_timestamp: Some(frame_seq as u32),
            is_keyframe: frame_seq == 1,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        };

        state
            .present_frame(mk_frame(1, 1_000.0))
            .expect("first present should work");
        state
            .present_frame(mk_frame(2, 1_016.0))
            .expect("second present should overwrite");

        let pressured = state
            .latest_render_candidate_decision()
            .expect("overwrite decision");
        assert_eq!(
            pressured.state,
            super::XbxRenderCandidateState::LatestOverwrite
        );
        assert_eq!(pressured.action, "replace");
        assert_eq!(pressured.detail, "latestSlotOverwrite");
        assert_eq!(pressured.frame_seq, Some(1));

        assert!(state.acknowledge_latest_frame(2));
        state
            .present_frame(mk_frame(3, 1_032.0))
            .expect("third present should recover");
        let recovered = state
            .latest_render_candidate_decision()
            .expect("recovered decision");
        assert_eq!(recovered.state, super::XbxRenderCandidateState::Nominal);
        assert_eq!(recovered.action, "accept");
        assert_eq!(recovered.detail, "latestSlotRecovered");
        assert_eq!(recovered.frame_seq, Some(3));
    }

    #[test]
    fn render_candidate_state_stays_latest_overwrite_until_latest_slot_is_acknowledged() {
        let mut state = XbxRenderState::default();
        let mk_frame = |frame_seq: u64, rendered_at_ms: f64| XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq,
            rendered_at_ms,
            rtp_timestamp: Some(frame_seq as u32),
            is_keyframe: frame_seq == 1,
            frame_recovery_disposition: Some("repairing".to_string()),
            frame_unrecoverable_reason: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::<[u8]>::from([0u8; 16]),
            },
        };

        state
            .present_frame(mk_frame(1, 1_000.0))
            .expect("first present should work");
        state
            .present_frame(mk_frame(2, 1_016.0))
            .expect("second present should overwrite");
        state
            .present_frame(mk_frame(3, 1_032.0))
            .expect("third present should continue overwriting");

        let pressured = state
            .latest_render_candidate_decision()
            .expect("overwrite decision should exist");
        assert_eq!(
            pressured.state,
            super::XbxRenderCandidateState::LatestOverwrite
        );
        assert_eq!(pressured.action, "replace");
        assert_eq!(pressured.detail, "latestSlotOverwrite");
        assert_eq!(pressured.frame_seq, Some(2));
        assert_eq!(state.peek_latest_frame().map(|frame| frame.frame_seq), Some(3));
    }
}
