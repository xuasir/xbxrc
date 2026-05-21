//! `RtcReceiveCore`：接收主线对外类型（RFC 四层之 receive 层入口）。

use crate::media::video::types::AssembledVideoFrame;
use crate::transport::rtc::receive::RtcVideoFrameSource;
use crate::transport::rtc::stream::adapter_types::FrameSource;

/// RFC receive 层对外类型：ingress 持有 `ReceiveCoreBody` 与 decode gate 裁决。
pub struct RtcReceiveCore {
    ingress: RtcVideoFrameSource,
}

impl RtcReceiveCore {
    pub(crate) fn new(ingress: RtcVideoFrameSource) -> Self {
        Self { ingress }
    }
}

impl FrameSource for RtcReceiveCore {
    fn recv_frame<'a>(
        &'a mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<AssembledVideoFrame>> + Send + 'a>>
    {
        self.ingress.recv_frame()
    }
}
