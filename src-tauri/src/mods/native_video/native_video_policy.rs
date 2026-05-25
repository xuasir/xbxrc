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
        // macOS：CVPixelBuffer 走 CALayer；CPU/软件解码面走 wgpu，避免 layer 拒收。
        VideoPlatformKind::MacOs if capabilities.supports_native_direct => {
            if surface.native_kind == VideoNativeSurfaceKind::MacOsCvPixelBuffer {
                VideoPipelinePlan {
                    presenter_mode: VideoPresenterMode::NativeDirect,
                    effect_pipeline: VideoEffectPipelineKind::Noop,
                }
            } else if capabilities.supports_gpu_direct {
                resolve_gpu_plan(requested_surface_id, capabilities)
            } else {
                VideoPipelinePlan {
                    presenter_mode: VideoPresenterMode::NativeDirect,
                    effect_pipeline: VideoEffectPipelineKind::Noop,
                }
            }
        }
        // Windows 现阶段先把 GPU direct 定成默认方向；
        // presenter/effect 能力位先收进 policy，后续补真实实现时不再改合同。
        VideoPlatformKind::Windows
            if capabilities.supports_gpu_direct
                && (surface.native_kind == VideoNativeSurfaceKind::WindowsD3d11Texture
                    || !surface.is_native_handle()) =>
        {
            resolve_windows_gpu_plan(surface, requested_surface_id, capabilities)
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

fn resolve_windows_gpu_plan(
    surface: &DecodedVideoSurface,
    requested_surface_id: Option<&str>,
    capabilities: VideoPlatformCapabilities,
) -> VideoPipelinePlan {
    // Windows D3D11 纹理已经由 presenter 直接导入并渲染，
    // 这里不能再错误地挂到仅支持 CPU surface 的 wgpu effect pipeline，
    // 否则 present_frame 会在 can_process(native_handle)=false 处被提前短路。
    if surface.native_kind == VideoNativeSurfaceKind::WindowsD3d11Texture {
        return VideoPipelinePlan {
            presenter_mode: VideoPresenterMode::GpuDirect,
            effect_pipeline: VideoEffectPipelineKind::Noop,
        };
    }
    resolve_gpu_plan(requested_surface_id, capabilities)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::native_video::types::{
        VideoColorMetadata, VideoSurfaceAccessKind, VideoSurfacePixelFormat,
    };

    fn windows_caps() -> VideoPlatformCapabilities {
        VideoPlatformCapabilities {
            platform: VideoPlatformKind::Windows,
            supports_native_direct: false,
            supports_gpu_direct: true,
            supports_wgpu_effects: true,
        }
    }

    fn cpu_surface() -> DecodedVideoSurface {
        DecodedVideoSurface {
            width: 1280,
            height: 720,
            pixel_format: VideoSurfacePixelFormat::Nv12,
            color: VideoColorMetadata::default(),
            access_kind: VideoSurfaceAccessKind::CpuMemory,
            native_kind: VideoNativeSurfaceKind::None,
        }
    }

    fn windows_d3d11_surface() -> DecodedVideoSurface {
        DecodedVideoSurface {
            width: 1280,
            height: 720,
            pixel_format: VideoSurfacePixelFormat::WindowsD3d11Texture,
            color: VideoColorMetadata::default(),
            access_kind: VideoSurfaceAccessKind::NativeHandle,
            native_kind: VideoNativeSurfaceKind::WindowsD3d11Texture,
        }
    }

    fn macos_cv_surface() -> DecodedVideoSurface {
        DecodedVideoSurface {
            width: 1280,
            height: 720,
            pixel_format: VideoSurfacePixelFormat::MacOsCvPixelBuffer,
            color: VideoColorMetadata::default(),
            access_kind: VideoSurfaceAccessKind::NativeHandle,
            native_kind: VideoNativeSurfaceKind::MacOsCvPixelBuffer,
        }
    }

    #[test]
    fn macos_cv_surface_uses_native_direct() {
        let plan = resolve_video_pipeline_plan(
            &macos_cv_surface(),
            Some("stream-page-video"),
            VideoPlatformCapabilities {
                platform: VideoPlatformKind::MacOs,
                supports_native_direct: true,
                supports_gpu_direct: true,
                supports_wgpu_effects: true,
            },
        );

        assert_eq!(plan.presenter_mode, VideoPresenterMode::NativeDirect);
        assert_eq!(plan.effect_pipeline, VideoEffectPipelineKind::Noop);
    }

    #[test]
    fn macos_cpu_surface_uses_gpu_direct() {
        let plan = resolve_video_pipeline_plan(
            &cpu_surface(),
            Some("wgpu:stream-page-video"),
            VideoPlatformCapabilities {
                platform: VideoPlatformKind::MacOs,
                supports_native_direct: true,
                supports_gpu_direct: true,
                supports_wgpu_effects: true,
            },
        );

        assert_eq!(plan.presenter_mode, VideoPresenterMode::GpuDirect);
        assert_eq!(plan.effect_pipeline, VideoEffectPipelineKind::Wgpu);
    }

    #[test]
    fn windows_cpu_surface_can_use_wgpu_effect_pipeline() {
        let plan = resolve_video_pipeline_plan(
            &cpu_surface(),
            Some("wgpu:stream-page-video"),
            windows_caps(),
        );

        assert_eq!(plan.presenter_mode, VideoPresenterMode::GpuDirect);
        assert_eq!(plan.effect_pipeline, VideoEffectPipelineKind::Wgpu);
    }

    #[test]
    fn windows_d3d11_surface_skips_wgpu_effect_pipeline() {
        let plan = resolve_video_pipeline_plan(
            &windows_d3d11_surface(),
            Some("wgpu:stream-page-video"),
            windows_caps(),
        );

        assert_eq!(plan.presenter_mode, VideoPresenterMode::GpuDirect);
        assert_eq!(plan.effect_pipeline, VideoEffectPipelineKind::Noop);
    }
}
