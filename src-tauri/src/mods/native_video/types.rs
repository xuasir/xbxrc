use xbxengine::{
    MacOsCVPixelBufferDescriptor, MacOsVideoChromaLocation, MacOsVideoColorMatrix,
    MacOsVideoColorPrimaries, MacOsVideoColorRange, MacOsVideoTransferFunction,
    XbxEngineRenderFrame, XbxEngineRenderPixelData,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoPlatformKind {
    MacOs,
    Windows,
    Other,
}

impl VideoPlatformKind {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Other
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoSurfacePixelFormat {
    Rgba8,
    Bgra8,
    Nv12,
    MacOsCvPixelBuffer,
    UnknownDescriptor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoSurfaceAccessKind {
    CpuMemory,
    NativeHandle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoNativeSurfaceKind {
    None,
    MacOsCvPixelBuffer,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoColorMetadata {
    pub matrix: MacOsVideoColorMatrix,
    pub primaries: MacOsVideoColorPrimaries,
    pub transfer: MacOsVideoTransferFunction,
    pub range: MacOsVideoColorRange,
    pub chroma_location: MacOsVideoChromaLocation,
}

impl Default for VideoColorMetadata {
    fn default() -> Self {
        Self {
            matrix: MacOsVideoColorMatrix::Bt709,
            primaries: MacOsVideoColorPrimaries::Bt709,
            transfer: MacOsVideoTransferFunction::Bt709,
            range: MacOsVideoColorRange::Video,
            chroma_location: MacOsVideoChromaLocation::Center,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodedVideoSurface {
    pub width: u32,
    pub height: u32,
    pub pixel_format: VideoSurfacePixelFormat,
    pub color: VideoColorMetadata,
    pub access_kind: VideoSurfaceAccessKind,
    pub native_kind: VideoNativeSurfaceKind,
}

impl DecodedVideoSurface {
    pub fn is_native_handle(&self) -> bool {
        self.access_kind == VideoSurfaceAccessKind::NativeHandle
    }

    pub fn from_render_frame(frame: &XbxEngineRenderFrame) -> Self {
        match &frame.pixel_data {
            XbxEngineRenderPixelData::Rgba { .. } => Self {
                width: frame.width,
                height: frame.height,
                pixel_format: VideoSurfacePixelFormat::Rgba8,
                color: VideoColorMetadata::default(),
                access_kind: VideoSurfaceAccessKind::CpuMemory,
                native_kind: VideoNativeSurfaceKind::None,
            },
            XbxEngineRenderPixelData::Bgra { .. } => Self {
                width: frame.width,
                height: frame.height,
                pixel_format: VideoSurfacePixelFormat::Bgra8,
                color: VideoColorMetadata::default(),
                access_kind: VideoSurfaceAccessKind::CpuMemory,
                native_kind: VideoNativeSurfaceKind::None,
            },
            XbxEngineRenderPixelData::Nv12 { .. } => Self {
                width: frame.width,
                height: frame.height,
                pixel_format: VideoSurfacePixelFormat::Nv12,
                color: VideoColorMetadata::default(),
                access_kind: VideoSurfaceAccessKind::CpuMemory,
                native_kind: VideoNativeSurfaceKind::None,
            },
            XbxEngineRenderPixelData::Descriptor { handle } => {
                let any_ref = handle.as_ref();
                if let Some(descriptor) = any_ref.downcast_ref::<MacOsCVPixelBufferDescriptor>() {
                    Self {
                        width: frame.width,
                        height: frame.height,
                        pixel_format: VideoSurfacePixelFormat::MacOsCvPixelBuffer,
                        color: VideoColorMetadata {
                            matrix: descriptor.color_matrix,
                            primaries: descriptor.color_primaries,
                            transfer: descriptor.transfer_function,
                            range: descriptor.color_range,
                            chroma_location: descriptor.chroma_location,
                        },
                        access_kind: VideoSurfaceAccessKind::NativeHandle,
                        native_kind: VideoNativeSurfaceKind::MacOsCvPixelBuffer,
                    }
                } else {
                    Self {
                        width: frame.width,
                        height: frame.height,
                        pixel_format: VideoSurfacePixelFormat::UnknownDescriptor,
                        color: VideoColorMetadata::default(),
                        access_kind: VideoSurfaceAccessKind::NativeHandle,
                        native_kind: VideoNativeSurfaceKind::Unknown,
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoPlatformCapabilities {
    pub platform: VideoPlatformKind,
    pub supports_native_direct: bool,
    pub supports_gpu_direct: bool,
    pub supports_wgpu_effects: bool,
}

impl VideoPlatformCapabilities {
    pub fn current() -> Self {
        match VideoPlatformKind::current() {
            VideoPlatformKind::MacOs => Self {
                platform: VideoPlatformKind::MacOs,
                supports_native_direct: true,
                supports_gpu_direct: true,
                supports_wgpu_effects: true,
            },
            VideoPlatformKind::Windows => Self {
                platform: VideoPlatformKind::Windows,
                supports_native_direct: false,
                supports_gpu_direct: true,
                supports_wgpu_effects: true,
            },
            VideoPlatformKind::Other => Self {
                platform: VideoPlatformKind::Other,
                supports_native_direct: false,
                supports_gpu_direct: false,
                supports_wgpu_effects: false,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoPresenterMode {
    NativeDirect,
    GpuDirect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoEffectPipelineKind {
    Noop,
    Wgpu,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoPipelinePlan {
    pub presenter_mode: VideoPresenterMode,
    pub effect_pipeline: VideoEffectPipelineKind,
}
