use super::{
    now_ms_f64,
    types::{DecodedVideoSurface, VideoEffectPipelineKind},
};
use xbxengine::XbxEngineRenderFrame;

#[derive(Clone, Debug)]
pub struct VideoEffectProcessFacts {
    pub kind: VideoEffectPipelineKind,
    pub active: bool,
    pub fallback_reason: Option<&'static str>,
    pub render_cost_ms: f64,
    pub input_surface: DecodedVideoSurface,
    pub output_surface: DecodedVideoSurface,
}

impl VideoEffectProcessFacts {
    fn new(
        kind: VideoEffectPipelineKind,
        active: bool,
        fallback_reason: Option<&'static str>,
        render_cost_ms: f64,
        input_surface: DecodedVideoSurface,
        output_surface: DecodedVideoSurface,
    ) -> Self {
        Self {
            kind,
            active,
            fallback_reason,
            render_cost_ms,
            input_surface,
            output_surface,
        }
    }
}

pub enum VideoEffectProcessOutcome {
    Accepted {
        frame: XbxEngineRenderFrame,
        facts: VideoEffectProcessFacts,
    },
    Rejected {
        facts: VideoEffectProcessFacts,
    },
}

pub trait VideoEffectPipeline: Send {
    fn kind(&self) -> VideoEffectPipelineKind;
    fn process_frame(
        &mut self,
        frame: &XbxEngineRenderFrame,
        surface: &DecodedVideoSurface,
    ) -> VideoEffectProcessOutcome;
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

    fn process_frame(
        &mut self,
        frame: &XbxEngineRenderFrame,
        surface: &DecodedVideoSurface,
    ) -> VideoEffectProcessOutcome {
        let started_at_ms = now_ms_f64();
        VideoEffectProcessOutcome::Accepted {
            frame: frame.clone(),
            facts: VideoEffectProcessFacts::new(
                self.kind(),
                false,
                None,
                (now_ms_f64() - started_at_ms).max(0.0),
                *surface,
                *surface,
            ),
        }
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

    fn process_frame(
        &mut self,
        frame: &XbxEngineRenderFrame,
        surface: &DecodedVideoSurface,
    ) -> VideoEffectProcessOutcome {
        let started_at_ms = now_ms_f64();
        let render_cost_ms = || (now_ms_f64() - started_at_ms).max(0.0);
        if surface.is_native_handle() {
            return VideoEffectProcessOutcome::Rejected {
                facts: VideoEffectProcessFacts::new(
                    self.kind(),
                    false,
                    Some("unsupportedNativeHandle"),
                    render_cost_ms(),
                    *surface,
                    *surface,
                ),
            };
        }

        VideoEffectProcessOutcome::Accepted {
            frame: frame.clone(),
            facts: VideoEffectProcessFacts::new(
                self.kind(),
                false,
                Some("passthroughPendingEffectRenderer"),
                render_cost_ms(),
                *surface,
                *surface,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::mods::native_video::types::{
        VideoColorMetadata, VideoNativeSurfaceKind, VideoSurfaceAccessKind, VideoSurfacePixelFormat,
    };
    use xbxengine::XbxEngineRenderPixelData;

    fn cpu_surface() -> DecodedVideoSurface {
        DecodedVideoSurface {
            width: 1280,
            height: 720,
            pixel_format: VideoSurfacePixelFormat::Rgba8,
            color: VideoColorMetadata::default(),
            access_kind: VideoSurfaceAccessKind::CpuMemory,
            native_kind: VideoNativeSurfaceKind::None,
        }
    }

    fn native_surface() -> DecodedVideoSurface {
        DecodedVideoSurface {
            width: 1280,
            height: 720,
            pixel_format: VideoSurfacePixelFormat::UnknownDescriptor,
            color: VideoColorMetadata::default(),
            access_kind: VideoSurfaceAccessKind::NativeHandle,
            native_kind: VideoNativeSurfaceKind::Unknown,
        }
    }

    fn frame() -> XbxEngineRenderFrame {
        XbxEngineRenderFrame {
            width: 1280,
            height: 720,
            frame_seq: 7,
            rendered_at_ms: 1_000.0,
            rtp_timestamp: Some(700),
            recovery_epoch_tag: None,
            recovery_owner_rtp_timestamp: None,
            is_keyframe: false,
            frame_recovery_disposition: None,
            frame_unrecoverable_reason: None,
            presentation_value_role: None,
            pixel_data: XbxEngineRenderPixelData::Rgba {
                bytes: Arc::from(vec![0_u8; 4].into_boxed_slice()),
            },
        }
    }

    #[test]
    fn noop_effect_accepts_passthrough_frame() {
        let mut pipeline = NoopVideoEffectPipeline::new();
        let frame = frame();
        let surface = cpu_surface();

        match pipeline.process_frame(&frame, &surface) {
            VideoEffectProcessOutcome::Accepted {
                frame: output,
                facts,
            } => {
                assert_eq!(output.frame_seq, frame.frame_seq);
                assert_eq!(facts.kind, VideoEffectPipelineKind::Noop);
                assert!(!facts.active);
                assert_eq!(facts.fallback_reason, None);
                assert_eq!(facts.input_surface, surface);
                assert_eq!(facts.output_surface, surface);
            }
            VideoEffectProcessOutcome::Rejected { .. } => {
                panic!("noop effect should accept passthrough frame")
            }
        }
    }

    #[test]
    fn wgpu_effect_rejects_native_handle_until_import_path_exists() {
        let mut pipeline = WgpuVideoEffectPipeline::new();
        let frame = frame();
        let surface = native_surface();

        match pipeline.process_frame(&frame, &surface) {
            VideoEffectProcessOutcome::Rejected { facts } => {
                assert_eq!(facts.kind, VideoEffectPipelineKind::Wgpu);
                assert!(!facts.active);
                assert_eq!(facts.fallback_reason, Some("unsupportedNativeHandle"));
                assert_eq!(facts.input_surface, surface);
                assert_eq!(facts.output_surface, surface);
            }
            VideoEffectProcessOutcome::Accepted { .. } => {
                panic!("wgpu effect should reject native handle surfaces")
            }
        }
    }
}
