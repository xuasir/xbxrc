use super::types::{
    DecodedVideoSurface, VideoEffectPipelineKind, VideoNativeSurfaceKind, VideoPipelinePlan,
    VideoPlatformCapabilities, VideoPlatformKind, VideoPresenterMode,
};

pub fn resolve_video_pipeline_plan(
    surface: &DecodedVideoSurface,
    requested_surface_id: Option<&str>,
    capabilities: VideoPlatformCapabilities,
) -> VideoPipelinePlan {
    normalize_video_pipeline_plan(match capabilities.platform {
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
        // Windows D3D11VA 解码纹理由 D3D11 native presenter 直出；
        // CPU surface 继续保留 WGPU/effect 路径。
        VideoPlatformKind::Windows
            if capabilities.supports_native_direct
                && surface.native_kind == VideoNativeSurfaceKind::WindowsD3d11Texture =>
        {
            VideoPipelinePlan {
                presenter_mode: VideoPresenterMode::NativeDirect,
                effect_pipeline: VideoEffectPipelineKind::Noop,
            }
        }
        VideoPlatformKind::Windows
            if capabilities.supports_gpu_direct && !surface.is_native_handle() =>
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
    })
}

pub fn resolve_initial_video_pipeline_plan(
    requested_surface_id: Option<&str>,
    capabilities: VideoPlatformCapabilities,
) -> VideoPipelinePlan {
    normalize_video_pipeline_plan(match capabilities.platform {
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
    })
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
    // Windows D3D11 纹理必须停留在 native direct 主线。
    if surface.native_kind == VideoNativeSurfaceKind::WindowsD3d11Texture {
        return VideoPipelinePlan {
            presenter_mode: VideoPresenterMode::NativeDirect,
            effect_pipeline: VideoEffectPipelineKind::Noop,
        };
    }
    resolve_gpu_plan(requested_surface_id, capabilities)
}

fn normalize_video_pipeline_plan(plan: VideoPipelinePlan) -> VideoPipelinePlan {
    if plan
        .effect_pipeline
        .required_presenter_mode()
        .is_some_and(|required_mode| required_mode != plan.presenter_mode)
    {
        return VideoPipelinePlan {
            presenter_mode: plan.presenter_mode,
            effect_pipeline: VideoEffectPipelineKind::Noop,
        };
    }
    plan
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
            supports_native_direct: true,
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
    fn windows_d3d11_surface_uses_native_direct_zero_copy() {
        let plan = resolve_video_pipeline_plan(
            &windows_d3d11_surface(),
            Some("wgpu:stream-page-video"),
            windows_caps(),
        );

        assert_eq!(plan.presenter_mode, VideoPresenterMode::NativeDirect);
        assert_eq!(plan.effect_pipeline, VideoEffectPipelineKind::Noop);
    }

    #[test]
    fn non_noop_effect_requires_matching_presenter_mode() {
        let plan = normalize_video_pipeline_plan(VideoPipelinePlan {
            presenter_mode: VideoPresenterMode::NativeDirect,
            effect_pipeline: VideoEffectPipelineKind::Wgpu,
        });

        assert_eq!(plan.presenter_mode, VideoPresenterMode::NativeDirect);
        assert_eq!(plan.effect_pipeline, VideoEffectPipelineKind::Noop);
    }
}
