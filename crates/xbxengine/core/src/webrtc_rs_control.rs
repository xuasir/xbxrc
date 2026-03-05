use tokio::runtime::Runtime;

use crate::{webrtc_rs_transport::WebRtcRsTransportState, XbxEngineRuntimeError};

pub(crate) struct WebRtcRsMediaControlContext<'a> {
    pub runtime: &'a Runtime,
    pub transport: &'a WebRtcRsTransportState,
}

/**
 * active `webrtc-rs` 媒体栈的控制面入口：
 * - recovery/runtime 后续只依赖这里触发动作
 * - 当前真实 keyframe 请求先复用现有 control data channel 能力
 * - 后续如果补 flush/render control，再继续沿这里扩
 */
pub(crate) trait WebRtcRsMediaControlPort: Send {
    fn request_video_keyframe(
        &mut self,
        context: WebRtcRsMediaControlContext<'_>,
    ) -> Result<(), XbxEngineRuntimeError>;
}

#[allow(dead_code)]
#[derive(Default)]
pub(crate) struct WebRtcRsNoopMediaControl;

impl WebRtcRsMediaControlPort for WebRtcRsNoopMediaControl {
    fn request_video_keyframe(
        &mut self,
        _context: WebRtcRsMediaControlContext<'_>,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct WebRtcRsDataChannelMediaControl;

impl WebRtcRsMediaControlPort for WebRtcRsDataChannelMediaControl {
    fn request_video_keyframe(
        &mut self,
        context: WebRtcRsMediaControlContext<'_>,
    ) -> Result<(), XbxEngineRuntimeError> {
        context.transport.request_video_keyframe(context.runtime)
    }
}
