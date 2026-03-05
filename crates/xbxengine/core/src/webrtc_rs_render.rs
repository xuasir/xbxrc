use std::sync::Arc;

use crate::{XbxEngineRenderFrame, XbxEngineRuntimeError, XbxEngineVideoFrameStats};
use xbxengine_protocol::XbxEngineDisplayStateDto;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WebRtcRsRenderFrame {
    pub width: u32,
    pub height: u32,
    pub frame_seq: u64,
    pub rendered_at_ms: f64,
    pub rgba_bytes: Arc<[u8]>,
}

impl WebRtcRsRenderFrame {
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

impl From<WebRtcRsRenderFrame> for XbxEngineRenderFrame {
    fn from(value: WebRtcRsRenderFrame) -> Self {
        Self {
            width: value.width,
            height: value.height,
            frame_seq: value.frame_seq,
            rendered_at_ms: value.rendered_at_ms,
            rgba_bytes: value.rgba_bytes,
        }
    }
}

/**
 * `core` 只负责“最新帧缓存”和显示状态同步，不在这里做 GPU 上传。
 * 真实上传/present 留在 `xbxengine-app` 渲染器，避免同一帧在 Rust 内重复上传。
 */
#[derive(Default)]
pub(crate) struct WebRtcRsRenderState {
    latest_display_state: Option<XbxEngineDisplayStateDto>,
    latest_frame: Option<XbxEngineRenderFrame>,
}

impl WebRtcRsRenderState {
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
        frame: WebRtcRsRenderFrame,
    ) -> Result<XbxEngineVideoFrameStats, XbxEngineRuntimeError> {
        let expected_len = frame.width as usize * frame.height as usize * 4;
        let actual_len = frame.rgba_bytes.len();
        if expected_len != actual_len {
            return Err(XbxEngineRuntimeError::new(format!(
                "xbxEngineFrameSizeMismatch:expected={expected_len}:actual={actual_len}"
            )));
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
        self.latest_frame.take()
    }
}
