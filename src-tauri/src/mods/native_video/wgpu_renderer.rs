#[cfg(target_os = "macos")]
use std::borrow::Cow;
#[cfg(target_os = "macos")]
use std::collections::VecDeque;
#[cfg(target_os = "macos")]
use std::ffi::c_void;
#[cfg(target_os = "macos")]
use std::ptr::NonNull;

#[cfg(target_os = "macos")]
use metal::foreign_types::ForeignType;
#[cfg(target_os = "macos")]
use raw_window_handle::{
    AppKitDisplayHandle, AppKitWindowHandle, DisplayHandle, HandleError, HasDisplayHandle,
    HasWindowHandle, WindowHandle,
};
#[cfg(target_os = "macos")]
use xbxengine::{
    MacOsCVPixelBufferDescriptor, MacOsVideoChromaLocation, MacOsVideoColorMatrix,
    MacOsVideoColorRange, XbxEngineRenderFrame, XbxEngineRenderPixelData,
};

#[cfg(target_os = "macos")]
const COPY_SHADER: &str = r#"
@group(0) @binding(0)
var frame_texture: texture_2d<f32>;

@group(0) @binding(1)
var frame_sampler: sampler;

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
  var positions = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(3.0, -1.0),
    vec2<f32>(-1.0, 3.0),
  );
  var uvs = array<vec2<f32>, 3>(
    vec2<f32>(0.0, 1.0),
    vec2<f32>(2.0, 1.0),
    vec2<f32>(0.0, -1.0),
  );

  var output: VertexOutput;
  output.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
  output.uv = uvs[vertex_index];
  return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
  return textureSample(frame_texture, frame_sampler, input.uv);
}
"#;

#[cfg(target_os = "macos")]
const NV12_SHADER: &str = r#"
@group(0) @binding(0)
var y_texture: texture_2d<f32>;

@group(0) @binding(1)
var uv_texture: texture_2d<f32>;

@group(0) @binding(2)
var uv_sampler: sampler;

struct Nv12Params {
  row0: vec4<f32>,
  row1: vec4<f32>,
  row2: vec4<f32>,
  uv_offset: vec4<f32>,
}

@group(0) @binding(3)
var<uniform> params: Nv12Params;

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
  var positions = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(3.0, -1.0),
    vec2<f32>(-1.0, 3.0),
  );
  var uvs = array<vec2<f32>, 3>(
    vec2<f32>(0.0, 1.0),
    vec2<f32>(2.0, 1.0),
    vec2<f32>(0.0, -1.0),
  );

  var output: VertexOutput;
  output.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
  output.uv = uvs[vertex_index];
  return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
  let y_size = vec2<i32>(textureDimensions(y_texture));
  let y_coord = clamp(vec2<i32>(input.uv * vec2<f32>(y_size)), vec2<i32>(0), y_size - vec2<i32>(1));
  let y = textureLoad(y_texture, y_coord, 0).r;
  let uv = textureSampleLevel(uv_texture, uv_sampler, input.uv + params.uv_offset.xy, 0.0).rg;
  let yuv = vec4<f32>(y, uv.x, uv.y, 1.0);
  let rgb = vec3<f32>(
    dot(params.row0, yuv),
    dot(params.row1, yuv),
    dot(params.row2, yuv),
  );
  return vec4<f32>(rgb, 1.0);
}
"#;

#[cfg(target_os = "macos")]
struct AppKitSurfaceTarget {
    ns_view: NonNull<c_void>,
}

#[cfg(target_os = "macos")]
unsafe impl Send for AppKitSurfaceTarget {}

#[cfg(target_os = "macos")]
unsafe impl Sync for AppKitSurfaceTarget {}

#[cfg(target_os = "macos")]
impl AppKitSurfaceTarget {
    fn new(ns_view: *mut c_void) -> Result<Self, String> {
        let Some(ns_view) = NonNull::new(ns_view) else {
            return Err("xbxEngineWgpuNsViewUnavailable".to_string());
        };
        Ok(Self { ns_view })
    }
}

#[cfg(target_os = "macos")]
impl HasDisplayHandle for AppKitSurfaceTarget {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(unsafe { DisplayHandle::borrow_raw(AppKitDisplayHandle::new().into()) })
    }
}

#[cfg(target_os = "macos")]
impl HasWindowHandle for AppKitSurfaceTarget {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = AppKitWindowHandle::new(self.ns_view);
        Ok(unsafe { WindowHandle::borrow_raw(raw.into()) })
    }
}

#[cfg(target_os = "macos")]
mod macos_bindings {
    use std::ffi::c_void;

    pub type CVImageBufferRef = *mut c_void;
    pub type CVPixelBufferRef = CVImageBufferRef;
    pub type CVMetalTextureRef = *mut c_void;
    pub type CVMetalTextureCacheRef = *mut c_void;
    pub type CVOptionFlags = u64;
    pub type CFAllocatorRef = *const c_void;
    pub type CFDictionaryRef = *const c_void;

    #[link(name = "CoreVideo", kind = "framework")]
    extern "C" {
        pub fn CVPixelBufferIsPlanar(pixel_buffer: CVPixelBufferRef) -> bool;
        pub fn CVPixelBufferLockBaseAddress(
            pixel_buffer: CVPixelBufferRef,
            lock_flags: CVOptionFlags,
        ) -> i32;
        pub fn CVPixelBufferUnlockBaseAddress(
            pixel_buffer: CVPixelBufferRef,
            lock_flags: CVOptionFlags,
        ) -> i32;
        pub fn CVPixelBufferGetBaseAddressOfPlane(
            pixel_buffer: CVPixelBufferRef,
            plane_index: usize,
        ) -> *mut c_void;
        pub fn CVPixelBufferGetBytesPerRowOfPlane(
            pixel_buffer: CVPixelBufferRef,
            plane_index: usize,
        ) -> usize;
        pub fn CVPixelBufferGetWidthOfPlane(
            pixel_buffer: CVPixelBufferRef,
            plane_index: usize,
        ) -> usize;
        pub fn CVPixelBufferGetHeightOfPlane(
            pixel_buffer: CVPixelBufferRef,
            plane_index: usize,
        ) -> usize;
        pub fn CVMetalTextureCacheCreate(
            allocator: CFAllocatorRef,
            cache_attributes: CFDictionaryRef,
            metal_device: *mut c_void,
            texture_attributes: CFDictionaryRef,
            cache_out: *mut CVMetalTextureCacheRef,
        ) -> i32;
        pub fn CVMetalTextureCacheCreateTextureFromImage(
            allocator: CFAllocatorRef,
            texture_cache: CVMetalTextureCacheRef,
            source_image: CVImageBufferRef,
            texture_attributes: CFDictionaryRef,
            pixel_format: u64,
            width: usize,
            height: usize,
            plane_index: usize,
            texture_out: *mut CVMetalTextureRef,
        ) -> i32;
        pub fn CVMetalTextureGetTexture(image: CVMetalTextureRef) -> *mut c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub fn CFRelease(c: *mut c_void);
    }
}

#[cfg(target_os = "macos")]
struct RgbaTextureBundle {
    width: u32,
    height: u32,
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

#[cfg(target_os = "macos")]
struct CoreVideoRetainedRef {
    ptr: *mut c_void,
}

#[cfg(target_os = "macos")]
impl CoreVideoRetainedRef {
    fn new(ptr: *mut c_void) -> Option<Self> {
        if ptr.is_null() {
            return None;
        }
        Some(Self { ptr })
    }

    fn as_ptr(&self) -> *mut c_void {
        self.ptr
    }
}

#[cfg(target_os = "macos")]
impl Drop for CoreVideoRetainedRef {
    fn drop(&mut self) {
        unsafe {
            macos_bindings::CFRelease(self.ptr);
        }
    }
}

#[cfg(target_os = "macos")]
struct Nv12TextureBundle {
    width: u32,
    height: u32,
    y_texture: wgpu::Texture,
    uv_texture: wgpu::Texture,
    params_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    _external_refs: Vec<CoreVideoRetainedRef>,
}

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Nv12ColorParamsStd140 {
    row0: [f32; 4],
    row1: [f32; 4],
    row2: [f32; 4],
    uv_offset: [f32; 4],
}

#[cfg(target_os = "macos")]
enum FrameTextureBundle {
    Rgba(RgbaTextureBundle),
    Bgra(RgbaTextureBundle),
    Nv12(Nv12TextureBundle),
}

#[cfg(target_os = "macos")]
impl FrameTextureBundle {
    fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::Rgba(bundle) | Self::Bgra(bundle) => (bundle.width, bundle.height),
            Self::Nv12(bundle) => (bundle.width, bundle.height),
        }
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone, Debug, Default)]
pub struct DescriptorUploadTelemetry {
    pub last_mode: Option<String>,
    pub metal_import_count_total: u64,
    pub cpu_upload_count_total: u64,
}

#[cfg(target_os = "macos")]
pub struct WgpuFrameRenderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    copy_render_pipeline: wgpu::RenderPipeline,
    nv12_render_pipeline: wgpu::RenderPipeline,
    copy_bind_group_layout: wgpu::BindGroupLayout,
    nv12_bind_group_layout: wgpu::BindGroupLayout,
    copy_sampler: wgpu::Sampler,
    uv_sampler: wgpu::Sampler,
    metal_texture_cache: Option<CoreVideoRetainedRef>,
    last_descriptor_upload_mode: Option<&'static str>,
    descriptor_metal_import_count_total: u64,
    descriptor_cpu_upload_count_total: u64,
    latest_frame: Option<XbxEngineRenderFrame>,
    frame_texture: Option<FrameTextureBundle>,
    retired_nv12_bundles: VecDeque<(u64, Nv12TextureBundle)>,
}

#[cfg(target_os = "macos")]
impl WgpuFrameRenderer {
    pub async fn new(ns_view: *mut c_void, width: u32, height: u32) -> Result<Self, String> {
        let target = AppKitSurfaceTarget::new(ns_view)?;
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(target)
            .map_err(|error| format!("xbxEngineCreateWgpuSurfaceFailed:{error}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .map_err(|error| format!("xbxEngineWgpuAdapterUnavailable:{error}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("xbxrc-native-video-device"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::default(),
            })
            .await
            .map_err(|error| format!("xbxEngineCreateWgpuDeviceFailed:{error}"))?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = choose_surface_format(&surface_caps);
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: width.max(1),
            height: height.max(1),
            present_mode: choose_present_mode(&surface_caps),
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        let copy_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xbxrc-native-video-copy-shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(COPY_SHADER)),
        });
        let nv12_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xbxrc-native-video-nv12-shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(NV12_SHADER)),
        });

        let copy_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("xbxrc-native-video-copy-bind-group-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let nv12_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("xbxrc-native-video-nv12-bind-group-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let copy_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("xbxrc-native-video-copy-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let uv_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("xbxrc-native-video-uv-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let copy_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xbxrc-native-video-copy-pipeline-layout"),
            bind_group_layouts: &[&copy_bind_group_layout],
            immediate_size: 0,
        });
        let copy_render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xbxrc-native-video-copy-pipeline"),
            layout: Some(&copy_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &copy_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &copy_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let nv12_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xbxrc-native-video-nv12-pipeline-layout"),
            bind_group_layouts: &[&nv12_bind_group_layout],
            immediate_size: 0,
        });
        let nv12_render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xbxrc-native-video-nv12-pipeline"),
            layout: Some(&nv12_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &nv12_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &nv12_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let metal_texture_cache = create_metal_texture_cache(&device);

        Ok(Self {
            surface,
            device,
            queue,
            surface_config,
            copy_render_pipeline,
            nv12_render_pipeline,
            copy_bind_group_layout,
            nv12_bind_group_layout,
            copy_sampler,
            uv_sampler,
            metal_texture_cache,
            last_descriptor_upload_mode: None,
            descriptor_metal_import_count_total: 0,
            descriptor_cpu_upload_count_total: 0,
            latest_frame: None,
            frame_texture: None,
            retired_nv12_bundles: VecDeque::new(),
        })
    }

    pub fn update_surface_size(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if self.surface_config.width == width && self.surface_config.height == height {
            return;
        }
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
    }

    pub fn update_frame(&mut self, frame: XbxEngineRenderFrame) {
        self.latest_frame = Some(frame);
    }

    pub fn descriptor_upload_telemetry(&self) -> DescriptorUploadTelemetry {
        DescriptorUploadTelemetry {
            last_mode: self.last_descriptor_upload_mode.map(str::to_string),
            metal_import_count_total: self.descriptor_metal_import_count_total,
            cpu_upload_count_total: self.descriptor_cpu_upload_count_total,
        }
    }

    pub fn render(&mut self) -> Result<(), String> {
        if let Some(frame) = self.latest_frame.take() {
            self.upload_frame(&frame)?;
            let current_seq = frame.frame_seq;
            while self
                .retired_nv12_bundles
                .front()
                .map(|item| item.0 < current_seq.saturating_sub(5))
                .unwrap_or(false)
            {
                let _ = self.retired_nv12_bundles.pop_front();
            }
        }

        let surface_texture = match self.surface.get_current_texture() {
            Ok(texture) => texture,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.surface_config);
                return Ok(());
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(()),
            Err(wgpu::SurfaceError::Other) => return Ok(()),
            Err(wgpu::SurfaceError::OutOfMemory) => {
                return Err("xbxEngineWgpuSurfaceOutOfMemory".to_string());
            }
        };

        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("xbxrc-native-video-render-encoder"),
            });
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xbxrc-native-video-render-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            if let Some(frame_texture) = self.frame_texture.as_ref() {
                let (source_width, source_height) = frame_texture.dimensions();
                let viewport = compute_aspect_fit_viewport(
                    self.surface_config.width,
                    self.surface_config.height,
                    source_width,
                    source_height,
                );
                render_pass.set_viewport(
                    viewport.x as f32,
                    viewport.y as f32,
                    viewport.width as f32,
                    viewport.height as f32,
                    0.0,
                    1.0,
                );
                render_pass.set_scissor_rect(
                    viewport.x,
                    viewport.y,
                    viewport.width.max(1),
                    viewport.height.max(1),
                );
                match frame_texture {
                    FrameTextureBundle::Rgba(bundle) | FrameTextureBundle::Bgra(bundle) => {
                        render_pass.set_pipeline(&self.copy_render_pipeline);
                        render_pass.set_bind_group(0, &bundle.bind_group, &[]);
                    }
                    FrameTextureBundle::Nv12(bundle) => {
                        render_pass.set_pipeline(&self.nv12_render_pipeline);
                        render_pass.set_bind_group(0, &bundle.bind_group, &[]);
                    }
                }
                render_pass.draw(0..3, 0..1);
            }
        }
        self.queue.submit(Some(encoder.finish()));
        surface_texture.present();
        Ok(())
    }

    fn upload_frame(&mut self, frame: &XbxEngineRenderFrame) -> Result<(), String> {
        match &frame.pixel_data {
            XbxEngineRenderPixelData::Rgba { bytes } => {
                self.ensure_rgba_texture(
                    frame.width,
                    frame.height,
                    wgpu::TextureFormat::Rgba8Unorm,
                );
                let Some(FrameTextureBundle::Rgba(bundle)) = self.frame_texture.as_ref() else {
                    return Err("xbxEngineWgpuFrameTextureMissing".to_string());
                };
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &bundle.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    bytes.as_ref(),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(frame.width * 4),
                        rows_per_image: Some(frame.height),
                    },
                    wgpu::Extent3d {
                        width: frame.width,
                        height: frame.height,
                        depth_or_array_layers: 1,
                    },
                );
            }
            XbxEngineRenderPixelData::Bgra { bytes } => {
                self.ensure_rgba_texture(
                    frame.width,
                    frame.height,
                    wgpu::TextureFormat::Bgra8Unorm,
                );
                let Some(FrameTextureBundle::Bgra(bundle)) = self.frame_texture.as_ref() else {
                    return Err("xbxEngineWgpuFrameTextureMissing".to_string());
                };
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &bundle.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    bytes.as_ref(),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(frame.width * 4),
                        rows_per_image: Some(frame.height),
                    },
                    wgpu::Extent3d {
                        width: frame.width,
                        height: frame.height,
                        depth_or_array_layers: 1,
                    },
                );
            }
            XbxEngineRenderPixelData::Nv12 {
                y_plane,
                uv_plane,
                y_stride,
                uv_stride,
            } => {
                self.ensure_nv12_texture(
                    frame.width,
                    frame.height,
                    build_nv12_color_params(
                        MacOsVideoColorMatrix::Bt709,
                        MacOsVideoColorRange::Video,
                        MacOsVideoChromaLocation::Center,
                        frame.width,
                        frame.height,
                    ),
                );
                let Some(FrameTextureBundle::Nv12(bundle)) = self.frame_texture.as_ref() else {
                    return Err("xbxEngineWgpuFrameTextureMissing".to_string());
                };
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &bundle.y_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    y_plane.as_ref(),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(*y_stride),
                        rows_per_image: Some(frame.height),
                    },
                    wgpu::Extent3d {
                        width: frame.width,
                        height: frame.height,
                        depth_or_array_layers: 1,
                    },
                );
                self.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &bundle.uv_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    uv_plane.as_ref(),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(*uv_stride),
                        rows_per_image: Some(frame.height.div_ceil(2)),
                    },
                    wgpu::Extent3d {
                        width: frame.width / 2,
                        height: frame.height.div_ceil(2),
                        depth_or_array_layers: 1,
                    },
                );
            }
            XbxEngineRenderPixelData::Descriptor { handle } => {
                let Some(descriptor) = handle.downcast_ref::<MacOsCVPixelBufferDescriptor>() else {
                    return Err("xbxEngineWgpuDescriptorUnsupported".to_string());
                };
                self.upload_descriptor_frame(
                    descriptor,
                    frame.frame_seq,
                    frame.width,
                    frame.height,
                )?;
            }
        }
        Ok(())
    }

    fn ensure_rgba_texture(&mut self, width: u32, height: u32, format: wgpu::TextureFormat) {
        let matches_existing = match (self.frame_texture.as_ref(), format) {
            (Some(FrameTextureBundle::Rgba(bundle)), wgpu::TextureFormat::Rgba8Unorm) => {
                bundle.width == width && bundle.height == height
            }
            (Some(FrameTextureBundle::Bgra(bundle)), wgpu::TextureFormat::Bgra8Unorm) => {
                bundle.width == width && bundle.height == height
            }
            _ => false,
        };
        if matches_existing {
            return;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xbxrc-native-video-rgba-texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xbxrc-native-video-rgba-bind-group"),
            layout: &self.copy_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.copy_sampler),
                },
            ],
        });
        let bundle = RgbaTextureBundle {
            width,
            height,
            texture,
            bind_group,
        };
        self.frame_texture = Some(if format == wgpu::TextureFormat::Bgra8Unorm {
            FrameTextureBundle::Bgra(bundle)
        } else {
            FrameTextureBundle::Rgba(bundle)
        });
    }

    fn ensure_nv12_texture(&mut self, width: u32, height: u32, params: Nv12ColorParamsStd140) {
        let matches_existing = matches!(
            self.frame_texture.as_ref(),
            Some(FrameTextureBundle::Nv12(bundle)) if bundle.width == width && bundle.height == height
        );
        if matches_existing {
            if let Some(FrameTextureBundle::Nv12(bundle)) = self.frame_texture.as_ref() {
                upload_nv12_params(&self.queue, &bundle.params_buffer, &params);
            }
            return;
        }
        let y_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xbxrc-native-video-y-texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let uv_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xbxrc-native-video-uv-texture"),
            size: wgpu::Extent3d {
                width: width / 2,
                height: height.div_ceil(2),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let y_view = y_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let uv_view = uv_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let params_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xbxrc-native-video-nv12-params-buffer"),
            size: std::mem::size_of::<Nv12ColorParamsStd140>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        upload_nv12_params(&self.queue, &params_buffer, &params);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xbxrc-native-video-nv12-bind-group"),
            layout: &self.nv12_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&y_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&uv_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.uv_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: params_buffer.as_entire_binding(),
                },
            ],
        });
        self.frame_texture = Some(FrameTextureBundle::Nv12(Nv12TextureBundle {
            width,
            height,
            y_texture,
            uv_texture,
            params_buffer,
            bind_group,
            _external_refs: Vec::new(),
        }));
    }

    fn upload_descriptor_frame(
        &mut self,
        descriptor: &MacOsCVPixelBufferDescriptor,
        frame_seq: u64,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        let pixel_buffer = descriptor.ptr as macos_bindings::CVPixelBufferRef;
        if pixel_buffer.is_null() {
            return Err("xbxEngineWgpuPixelBufferMissing".to_string());
        }
        let params = build_nv12_color_params(
            descriptor.color_matrix,
            descriptor.color_range,
            descriptor.chroma_location,
            width,
            height,
        );
        match self.try_import_descriptor_frame(pixel_buffer, frame_seq, width, height, params) {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(error) => {
                log::debug!(
                    "[native_video][wgpu] metal import failed, fallback to cpu upload error={}",
                    error
                );
            }
        }
        self.last_descriptor_upload_mode = Some("cpuUploadFallback");
        self.descriptor_cpu_upload_count_total =
            self.descriptor_cpu_upload_count_total.saturating_add(1);
        let lock_status = unsafe { macos_bindings::CVPixelBufferLockBaseAddress(pixel_buffer, 0) };
        if lock_status != 0 {
            return Err(format!("xbxEngineWgpuLockPixelBufferFailed:{lock_status}"));
        }

        let result = (|| {
            if !unsafe { macos_bindings::CVPixelBufferIsPlanar(pixel_buffer) } {
                return Err("xbxEngineWgpuPixelBufferNotPlanar".to_string());
            }
            let y_base =
                unsafe { macos_bindings::CVPixelBufferGetBaseAddressOfPlane(pixel_buffer, 0) };
            let uv_base =
                unsafe { macos_bindings::CVPixelBufferGetBaseAddressOfPlane(pixel_buffer, 1) };
            if y_base.is_null() || uv_base.is_null() {
                return Err("xbxEngineWgpuPixelBufferPlaneMissing".to_string());
            }

            let y_stride =
                unsafe { macos_bindings::CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer, 0) };
            let uv_stride =
                unsafe { macos_bindings::CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer, 1) };
            let y_height =
                unsafe { macos_bindings::CVPixelBufferGetHeightOfPlane(pixel_buffer, 0) };
            let uv_height =
                unsafe { macos_bindings::CVPixelBufferGetHeightOfPlane(pixel_buffer, 1) };

            self.ensure_nv12_texture(width, height, params);
            let Some(FrameTextureBundle::Nv12(bundle)) = self.frame_texture.as_ref() else {
                return Err("xbxEngineWgpuFrameTextureMissing".to_string());
            };

            let y_bytes =
                unsafe { std::slice::from_raw_parts(y_base.cast::<u8>(), y_stride * y_height) };
            let uv_bytes =
                unsafe { std::slice::from_raw_parts(uv_base.cast::<u8>(), uv_stride * uv_height) };

            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &bundle.y_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                y_bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(y_stride as u32),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &bundle.uv_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                uv_bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(uv_stride as u32),
                    rows_per_image: Some(height.div_ceil(2)),
                },
                wgpu::Extent3d {
                    width: width / 2,
                    height: height.div_ceil(2),
                    depth_or_array_layers: 1,
                },
            );
            Ok(())
        })();

        unsafe {
            let _ = macos_bindings::CVPixelBufferUnlockBaseAddress(pixel_buffer, 0);
        }
        result
    }

    fn try_import_descriptor_frame(
        &mut self,
        pixel_buffer: macos_bindings::CVPixelBufferRef,
        frame_seq: u64,
        width: u32,
        height: u32,
        params: Nv12ColorParamsStd140,
    ) -> Result<bool, String> {
        let Some(texture_cache) = self.metal_texture_cache.as_ref() else {
            return Ok(false);
        };
        if !unsafe { macos_bindings::CVPixelBufferIsPlanar(pixel_buffer) } {
            return Ok(false);
        }

        let y_width = unsafe { macos_bindings::CVPixelBufferGetWidthOfPlane(pixel_buffer, 0) };
        let y_height = unsafe { macos_bindings::CVPixelBufferGetHeightOfPlane(pixel_buffer, 0) };
        let uv_width = unsafe { macos_bindings::CVPixelBufferGetWidthOfPlane(pixel_buffer, 1) };
        let uv_height = unsafe { macos_bindings::CVPixelBufferGetHeightOfPlane(pixel_buffer, 1) };
        if y_width == 0 || y_height == 0 || uv_width == 0 || uv_height == 0 {
            return Ok(false);
        }

        let Some(y_cv_texture) = create_cv_metal_texture(
            texture_cache,
            pixel_buffer,
            metal::MTLPixelFormat::R8Unorm,
            y_width,
            y_height,
            0,
        )?
        else {
            return Ok(false);
        };
        let Some(uv_cv_texture) = create_cv_metal_texture(
            texture_cache,
            pixel_buffer,
            metal::MTLPixelFormat::RG8Unorm,
            uv_width,
            uv_height,
            1,
        )?
        else {
            return Ok(false);
        };

        let y_texture = import_metal_texture_to_wgpu(
            &self.device,
            y_cv_texture.as_ptr(),
            wgpu::TextureFormat::R8Unorm,
            y_width as u32,
            y_height as u32,
        )?;
        let uv_texture = import_metal_texture_to_wgpu(
            &self.device,
            uv_cv_texture.as_ptr(),
            wgpu::TextureFormat::Rg8Unorm,
            uv_width as u32,
            uv_height as u32,
        )?;
        let y_view = y_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let uv_view = uv_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let params_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xbxrc-native-video-imported-nv12-params-buffer"),
            size: std::mem::size_of::<Nv12ColorParamsStd140>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        upload_nv12_params(&self.queue, &params_buffer, &params);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xbxrc-native-video-nv12-bind-group"),
            layout: &self.nv12_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&y_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&uv_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.uv_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: params_buffer.as_entire_binding(),
                },
            ],
        });
        let imported_bundle = Nv12TextureBundle {
            width,
            height,
            y_texture,
            uv_texture,
            params_buffer,
            bind_group,
            _external_refs: vec![y_cv_texture, uv_cv_texture],
        };

        if let Some(FrameTextureBundle::Nv12(previous_bundle)) = self
            .frame_texture
            .replace(FrameTextureBundle::Nv12(imported_bundle))
        {
            self.retired_nv12_bundles
                .push_back((frame_seq, previous_bundle));
        }
        self.last_descriptor_upload_mode = Some("metalImport");
        self.descriptor_metal_import_count_total =
            self.descriptor_metal_import_count_total.saturating_add(1);
        Ok(true)
    }
}

#[cfg(target_os = "macos")]
fn upload_nv12_params(
    queue: &wgpu::Queue,
    params_buffer: &wgpu::Buffer,
    params: &Nv12ColorParamsStd140,
) {
    let bytes = unsafe {
        std::slice::from_raw_parts(
            (params as *const Nv12ColorParamsStd140).cast::<u8>(),
            std::mem::size_of::<Nv12ColorParamsStd140>(),
        )
    };
    queue.write_buffer(params_buffer, 0, bytes);
}

#[cfg(target_os = "macos")]
fn build_nv12_color_params(
    matrix: MacOsVideoColorMatrix,
    range: MacOsVideoColorRange,
    chroma_location: MacOsVideoChromaLocation,
    width: u32,
    height: u32,
) -> Nv12ColorParamsStd140 {
    let (row0, row1, row2) = match (matrix, range) {
        (MacOsVideoColorMatrix::Bt601, MacOsVideoColorRange::Full) => (
            [1.0, 0.0, 1.402, -0.701],
            [1.0, -0.344_136, -0.714_136, 0.529_136],
            [1.0, 1.772, 0.0, -0.886],
        ),
        (MacOsVideoColorMatrix::Bt601, MacOsVideoColorRange::Video) => (
            [1.164_383, 0.0, 1.596_027, -0.874_202],
            [1.164_383, -0.391_762, -0.812_968, 0.531_668],
            [1.164_383, 2.017_232, 0.0, -1.085_631],
        ),
        (MacOsVideoColorMatrix::Smpte240M, MacOsVideoColorRange::Full) => (
            [1.0, 0.0, 1.5756, -0.7878],
            [1.0, -0.2253, -0.4768, 0.35105],
            [1.0, 1.8270, 0.0, -0.9135],
        ),
        (MacOsVideoColorMatrix::Smpte240M, MacOsVideoColorRange::Video) => (
            [1.164_383, 0.0, 1.794_107, -0.916_742],
            [1.164_383, -0.257_985, -0.542_583, 0.396_729],
            [1.164_383, 2.078_705, 0.0, -1.133_403],
        ),
        (MacOsVideoColorMatrix::Bt2020, MacOsVideoColorRange::Full) => (
            [1.0, 0.0, 1.4746, -0.7373],
            [1.0, -0.164_553, -0.571_353, 0.367_953],
            [1.0, 1.8814, 0.0, -0.9407],
        ),
        (MacOsVideoColorMatrix::Bt2020, MacOsVideoColorRange::Video) => (
            [1.164_383, 0.0, 1.678_674, -0.915_688],
            [1.164_383, -0.187_326, -0.650_424, 0.347_458],
            [1.164_383, 2.141_772, 0.0, -1.148_145],
        ),
        (MacOsVideoColorMatrix::Bt709, MacOsVideoColorRange::Full)
        | (MacOsVideoColorMatrix::Unknown, MacOsVideoColorRange::Full) => (
            [1.0, 0.0, 1.5748, -0.7874],
            [1.0, -0.187_324, -0.468_124, 0.327_724],
            [1.0, 1.8556, 0.0, -0.9278],
        ),
        (MacOsVideoColorMatrix::Bt709, MacOsVideoColorRange::Video)
        | (MacOsVideoColorMatrix::Unknown, MacOsVideoColorRange::Video) => (
            [1.164_383, 0.0, 1.792_741, -0.972_945],
            [1.164_383, -0.213_249, -0.532_909, 0.301_483],
            [1.164_383, 2.112_402, 0.0, -1.133_402],
        ),
    };
    let uv_width = (width / 2).max(1) as f32;
    let uv_height = height.div_ceil(2).max(1) as f32;
    let (uv_offset_x, uv_offset_y) = match chroma_location {
        MacOsVideoChromaLocation::Left => (0.5 / uv_width, 0.0),
        MacOsVideoChromaLocation::TopLeft => (0.5 / uv_width, 0.5 / uv_height),
        MacOsVideoChromaLocation::Center | MacOsVideoChromaLocation::Unknown => (0.0, 0.0),
    };
    Nv12ColorParamsStd140 {
        row0: [row0[0], row0[1], row0[2], row0[3]],
        row1: [row1[0], row1[1], row1[2], row1[3]],
        row2: [row2[0], row2[1], row2[2], row2[3]],
        uv_offset: [uv_offset_x, uv_offset_y, 0.0, 0.0],
    }
}

#[cfg(target_os = "macos")]
fn create_metal_texture_cache(device: &wgpu::Device) -> Option<CoreVideoRetainedRef> {
    let hal_device = unsafe { device.as_hal::<wgpu_hal::api::Metal>() }?;
    let metal_device = hal_device.raw_device().clone();

    let mut texture_cache: macos_bindings::CVMetalTextureCacheRef = std::ptr::null_mut();
    let status = unsafe {
        macos_bindings::CVMetalTextureCacheCreate(
            std::ptr::null(),
            std::ptr::null(),
            metal_device.as_ptr() as *mut c_void,
            std::ptr::null(),
            &mut texture_cache,
        )
    };
    if status != 0 {
        return None;
    }
    CoreVideoRetainedRef::new(texture_cache)
}

#[cfg(target_os = "macos")]
fn create_cv_metal_texture(
    texture_cache: &CoreVideoRetainedRef,
    pixel_buffer: macos_bindings::CVPixelBufferRef,
    pixel_format: metal::MTLPixelFormat,
    width: usize,
    height: usize,
    plane_index: usize,
) -> Result<Option<CoreVideoRetainedRef>, String> {
    let mut cv_texture: macos_bindings::CVMetalTextureRef = std::ptr::null_mut();
    let status = unsafe {
        macos_bindings::CVMetalTextureCacheCreateTextureFromImage(
            std::ptr::null(),
            texture_cache.as_ptr() as macos_bindings::CVMetalTextureCacheRef,
            pixel_buffer,
            std::ptr::null(),
            pixel_format as u64,
            width,
            height,
            plane_index,
            &mut cv_texture,
        )
    };
    if status != 0 {
        return Ok(None);
    }
    Ok(CoreVideoRetainedRef::new(cv_texture))
}

#[cfg(target_os = "macos")]
fn import_metal_texture_to_wgpu(
    device: &wgpu::Device,
    cv_texture_ptr: *mut c_void,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> Result<wgpu::Texture, String> {
    use objc2::runtime::AnyObject;
    use objc2::{msg_send, rc::autoreleasepool};

    autoreleasepool(|_| unsafe {
        let raw_metal_texture = macos_bindings::CVMetalTextureGetTexture(
            cv_texture_ptr as macos_bindings::CVMetalTextureRef,
        );
        if raw_metal_texture.is_null() {
            return Err("xbxEngineWgpuMetalTextureUnavailable".to_string());
        }
        let retained_texture: *mut AnyObject =
            msg_send![raw_metal_texture.cast::<AnyObject>(), retain];
        let metal_texture = metal::Texture::from_ptr(retained_texture.cast());
        let hal_texture = wgpu_hal::metal::Device::texture_from_raw(
            metal_texture,
            format,
            metal::MTLTextureType::D2,
            1,
            1,
            wgpu_hal::CopyExtent {
                width,
                height,
                depth: 1,
            },
        );
        let descriptor = wgpu::TextureDescriptor {
            label: Some("xbxrc-native-video-imported-texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        Ok(device.create_texture_from_hal::<wgpu_hal::api::Metal>(hal_texture, &descriptor))
    })
}

#[cfg(target_os = "macos")]
fn choose_surface_format(capabilities: &wgpu::SurfaceCapabilities) -> wgpu::TextureFormat {
    for format in [
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8UnormSrgb,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ] {
        if capabilities.formats.contains(&format) {
            return format;
        }
    }
    capabilities.formats[0]
}

#[cfg(target_os = "macos")]
struct AspectFitViewport {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[cfg(target_os = "macos")]
fn compute_aspect_fit_viewport(
    surface_width: u32,
    surface_height: u32,
    source_width: u32,
    source_height: u32,
) -> AspectFitViewport {
    if surface_width == 0 || surface_height == 0 || source_width == 0 || source_height == 0 {
        return AspectFitViewport {
            x: 0,
            y: 0,
            width: surface_width.max(1),
            height: surface_height.max(1),
        };
    }
    let width_limited = (surface_width as u64 * source_height as u64)
        <= (surface_height as u64 * source_width as u64);
    let (draw_width, draw_height) = if width_limited {
        let draw_width = surface_width.max(1);
        let draw_height =
            ((surface_width as u64 * source_height as u64) / source_width as u64).max(1) as u32;
        (draw_width, draw_height.min(surface_height.max(1)))
    } else {
        let draw_height = surface_height.max(1);
        let draw_width =
            ((surface_height as u64 * source_width as u64) / source_height as u64).max(1) as u32;
        (draw_width.min(surface_width.max(1)), draw_height)
    };
    let offset_x = surface_width.saturating_sub(draw_width) / 2;
    let offset_y = surface_height.saturating_sub(draw_height) / 2;
    AspectFitViewport {
        x: offset_x,
        y: offset_y,
        width: draw_width.max(1),
        height: draw_height.max(1),
    }
}

#[cfg(target_os = "macos")]
fn choose_present_mode(capabilities: &wgpu::SurfaceCapabilities) -> wgpu::PresentMode {
    if capabilities
        .present_modes
        .contains(&wgpu::PresentMode::AutoVsync)
    {
        return wgpu::PresentMode::AutoVsync;
    }
    wgpu::PresentMode::Fifo
}
