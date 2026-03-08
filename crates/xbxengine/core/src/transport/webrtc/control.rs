use tokio::runtime::Handle;

use crate::{transport::webrtc::transport::XbxTransportState, XbxEngineRuntimeError};

pub(crate) struct XbxMediaControlContext<'a> {
    pub runtime: &'a Handle,
    pub transport: &'a XbxTransportState,
}

/**
 * active `webrtc-rs` 媒体栈的控制面入口：
 * - recovery/runtime 后续只依赖这里触发动作
 * - 当前真实 keyframe 请求先复用现有 control data channel 能力
 * - 后续如果补 flush/render control，再继续沿这里扩
 */
pub(crate) trait XbxMediaControlPort: Send {
    fn request_video_keyframe(
        &mut self,
        context: XbxMediaControlContext<'_>,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn request_decoder_reset(
        &mut self,
        context: XbxMediaControlContext<'_>,
    ) -> Result<(), XbxEngineRuntimeError>;
}

#[allow(dead_code)]
#[derive(Default)]
pub(crate) struct XbxNoopMediaControl;

impl XbxMediaControlPort for XbxNoopMediaControl {
    fn request_video_keyframe(
        &mut self,
        _context: XbxMediaControlContext<'_>,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }

    fn request_decoder_reset(
        &mut self,
        _context: XbxMediaControlContext<'_>,
    ) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct XbxDataChannelMediaControl;

impl XbxMediaControlPort for XbxDataChannelMediaControl {
    fn request_video_keyframe(
        &mut self,
        context: XbxMediaControlContext<'_>,
    ) -> Result<(), XbxEngineRuntimeError> {
        context.transport.request_video_keyframe(context.runtime)
    }

    fn request_decoder_reset(
        &mut self,
        context: XbxMediaControlContext<'_>,
    ) -> Result<(), XbxEngineRuntimeError> {
        context.transport.request_decoder_reset(context.runtime)
    }
}
