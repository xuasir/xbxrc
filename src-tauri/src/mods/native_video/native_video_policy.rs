use super::types::{
    DecodedVideoSurface, VideoEffectPipelineKind, VideoNativeSurfaceKind, VideoPipelinePlan,
    VideoPlatformCapabilities, VideoPlatformKind, VideoPresenterMode,
};

pub fn resolve_video_pipeline_plan(
    surface: &DecodedVideoSurface,
    requested_surface_id: Option<&str>,
    capabilities: VideoPlatformCapabilities,
) -> VideoPipelinePlan {
    match capabilities.platform {
        // macOS 现阶段默认优先系统视频层，保证清晰度和稳定性；
        // 后续只有在显式增强模式下再切到 GPU path。
        VideoPlatformKind::MacOs
            if capabilities.supports_native_direct
                && surface.native_kind == VideoNativeSurfaceKind::MacOsCvPixelBuffer =>
        {
            VideoPipelinePlan {
                presenter_mode: VideoPresenterMode::NativeDirect,
                effect_pipeline: VideoEffectPipelineKind::Noop,
            }
        }
        // Windows 现阶段先把 GPU direct 定成默认方向；
        // presenter/effect 能力位先收进 policy，后续补真实实现时不再改合同。
        VideoPlatformKind::Windows if capabilities.supports_gpu_direct => {
            resolve_gpu_plan(requested_surface_id, capabilities)
        }
        _ if capabilities.supports_gpu_direct => {
            resolve_gpu_plan(requested_surface_id, capabilities)
        }
        _ if capabilities.supports_native_direct => VideoPipelinePlan {
            presenter_mode: VideoPresenterMode::NativeDirect,
            effect_pipeline: VideoEffectPipelineKind::Noop,
        },
        _ => VideoPipelinePlan {
            presenter_mode: VideoPresenterMode::GpuDirect,
            effect_pipeline: VideoEffectPipelineKind::Noop,
        },
    }
}

pub fn resolve_initial_video_pipeline_plan(
    requested_surface_id: Option<&str>,
    capabilities: VideoPlatformCapabilities,
) -> VideoPipelinePlan {
    match capabilities.platform {
        VideoPlatformKind::MacOs if capabilities.supports_native_direct => VideoPipelinePlan {
            presenter_mode: VideoPresenterMode::NativeDirect,
            effect_pipeline: VideoEffectPipelineKind::Noop,
        },
        VideoPlatformKind::Windows if capabilities.supports_gpu_direct => {
            resolve_gpu_plan(requested_surface_id, capabilities)
        }
        _ if capabilities.supports_gpu_direct => {
            resolve_gpu_plan(requested_surface_id, capabilities)
        }
        _ if capabilities.supports_native_direct => VideoPipelinePlan {
            presenter_mode: VideoPresenterMode::NativeDirect,
            effect_pipeline: VideoEffectPipelineKind::Noop,
        },
        _ => VideoPipelinePlan {
            presenter_mode: VideoPresenterMode::GpuDirect,
            effect_pipeline: VideoEffectPipelineKind::Noop,
        },
    }
}

fn resolve_gpu_plan(
    requested_surface_id: Option<&str>,
    capabilities: VideoPlatformCapabilities,
) -> VideoPipelinePlan {
    if requested_surface_id.is_some_and(|surface_id| surface_id.starts_with("wgpu:"))
        && capabilities.supports_wgpu_effects
    {
        return VideoPipelinePlan {
            presenter_mode: VideoPresenterMode::GpuDirect,
            effect_pipeline: VideoEffectPipelineKind::Wgpu,
        };
    }

    VideoPipelinePlan {
        presenter_mode: VideoPresenterMode::GpuDirect,
        effect_pipeline: VideoEffectPipelineKind::Noop,
    }
}
