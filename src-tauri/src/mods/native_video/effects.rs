use super::types::{DecodedVideoSurface, VideoEffectPipelineKind};

pub trait VideoEffectPipeline: Send {
    fn kind(&self) -> VideoEffectPipelineKind;
    fn can_process(&self, _surface: &DecodedVideoSurface) -> bool;
}

pub struct NoopVideoEffectPipeline;

impl NoopVideoEffectPipeline {
    pub fn new() -> Self {
        Self
    }
}

impl VideoEffectPipeline for NoopVideoEffectPipeline {
    fn kind(&self) -> VideoEffectPipelineKind {
        VideoEffectPipelineKind::Noop
    }

    fn can_process(&self, _surface: &DecodedVideoSurface) -> bool {
        true
    }
}

pub struct WgpuVideoEffectPipeline;

impl WgpuVideoEffectPipeline {
    pub fn new() -> Self {
        Self
    }
}

impl VideoEffectPipeline for WgpuVideoEffectPipeline {
    fn kind(&self) -> VideoEffectPipelineKind {
        VideoEffectPipelineKind::Wgpu
    }

    fn can_process(&self, surface: &DecodedVideoSurface) -> bool {
        !surface.is_native_handle()
    }
}
