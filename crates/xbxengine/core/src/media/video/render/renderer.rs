use crate::{
    XbxEngineRenderFrame, XbxEngineRenderPixelData, XbxEngineRuntimeError, XbxEngineVideoFrameStats,
};
use xbxengine_protocol::XbxEngineDisplayStateDto;
#[allow(dead_code)]
const RENDER_STALL_THRESHOLD_MS: f64 = 1_500.0;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct XbxRenderSignalSnapshot {
    pub latest_present_time_ms: Option<f64>,
    pub renderer_stalled: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct XbxRenderFrame {
    pub width: u32,
    pub height: u32,
    pub frame_seq: u64,
    pub rendered_at_ms: f64,
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
}

impl XbxRenderState {
    pub(crate) fn reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.latest_display_state = None;
        self.latest_frame = None;
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
    ) -> Result<XbxEngineVideoFrameStats, XbxEngineRuntimeError> {
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
        self.latest_frame = Some(frame.into());
        Ok(frame_stats)
    }

    pub(crate) fn stop(&mut self) {
        self.latest_display_state = None;
        self.latest_frame = None;
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
            self.latest_frame = None;
            return true;
        }
        false
    }

    #[allow(dead_code)]
    pub(crate) fn render_signal_snapshot(&self, now_ms: f64) -> XbxRenderSignalSnapshot {
        let latest_present_time_ms = self.latest_frame.as_ref().map(|frame| frame.rendered_at_ms);
        let renderer_stalled = latest_present_time_ms.map(|presented_at_ms| {
            (now_ms - presented_at_ms).max(0.0) >= RENDER_STALL_THRESHOLD_MS
        });
        XbxRenderSignalSnapshot {
            latest_present_time_ms,
            renderer_stalled,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{XbxRenderFrame, XbxRenderState};
    use crate::XbxEngineRenderPixelData;

    #[test]
    fn latest_slot_supports_peek_take_and_ack() {
        let mut state = XbxRenderState::default();
        let frame = XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 1,
            rendered_at_ms: 1_000.0,
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
    fn render_signal_snapshot_marks_stall_after_threshold() {
        let mut state = XbxRenderState::default();
        let frame = XbxRenderFrame {
            width: 2,
            height: 2,
            frame_seq: 1,
            rendered_at_ms: 1_000.0,
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
}
