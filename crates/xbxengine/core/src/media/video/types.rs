use bytes::Bytes;
use std::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
}

#[derive(Clone, Debug)]
pub struct EncodedFrame {
    pub codec: VideoCodec,

    pub is_keyframe: bool,
    pub config_changed: bool,

    pub width: u32,
    pub height: u32,

    pub rtp_timestamp: u32,

    pub assembled_at: Instant,
    pub target_playout_time: Instant,

    pub payload: Bytes,
}

pub trait FrameSurface: Send + Sync {}

pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,

    pub pts: Instant,

    pub surface: crate::media::video::render::renderer::XbxRenderFrame,
}

