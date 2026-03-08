use std::borrow::Cow;

use metal::foreign_types::ForeignType;
use winit::{dpi::PhysicalSize, window::Window};
use xbxengine::{XbxEngineRenderFrame, XbxEngineRenderPixelData};

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

const NV12_SHADER: &str = r#"
@group(0) @binding(0)
var y_texture: texture_2d<f32>;

@group(0) @binding(1)
var uv_texture: texture_2d<f32>;

@group(0) @binding(2)
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

fn yuv_to_rgb(y: f32, u: f32, v: f32) -> vec3<f32> {
  let c = y - 16.0 / 255.0;
  let d = u - 0.5;
  let e = v - 0.5;
  let r = clamp(1.164383 * c + 1.596027 * e, 0.0, 1.0);
  let g = clamp(1.164383 * c - 0.391762 * d - 0.812968 * e, 0.0, 1.0);
  let b = clamp(1.164383 * c + 2.017232 * d, 0.0, 1.0);
  return vec3<f32>(r, g, b);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
  let y = textureSample(y_texture, frame_sampler, input.uv).r;
  let uv = textureSample(uv_texture, frame_sampler, input.uv).rg;
  let rgb = yuv_to_rgb(y, uv.x, uv.y);
  return vec4<f32>(rgb, 1.0);
}
"#;

struct RgbaTextureBundle {
    width: u32,
    height: u32,
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

struct Nv12TextureBundle {
    width: u32,
    height: u32,
    y_texture: wgpu::Texture,
    uv_texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    #[cfg(target_os = "macos")]
    cv_textures: Option<(
        macos_bindings::CVMetalTextureRef,
        macos_bindings::CVMetalTextureRef,
    )>,
}

impl Drop for Nv12TextureBundle {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some((y, uv)) = self.cv_textures.take() {
            unsafe {
                xbxengine::xbx_log_warn!(
                    "[xbxengine-app] dropping Nv12TextureBundle, releasing CVMetalTextureRef"
                );
                macos_bindings::CFRelease(y);
                macos_bindings::CFRelease(uv);
            }
        }
    }
}

enum FrameTextureBundle {
    Rgba(RgbaTextureBundle),
    Bgra(RgbaTextureBundle),
    Nv12(Nv12TextureBundle),
}

#[cfg(target_os = "macos")]
mod macos_bindings {
    use std::ffi::c_void;
    pub type CVReturn = i32;
    pub type CVMetalTextureCacheRef = *mut c_void;
    pub type CVMetalTextureRef = *mut c_void;
    pub type CVImageBufferRef = *mut c_void;

    #[link(name = "CoreVideo", kind = "framework")]
    extern "C" {
        pub fn CVMetalTextureCacheCreate(
            allocator: *mut c_void,
            cacheAttributes: *mut c_void,
            metalDevice: *mut c_void,
            textureAttributes: *mut c_void,
            cacheOut: *mut CVMetalTextureCacheRef,
        ) -> CVReturn;

        pub fn CVMetalTextureCacheCreateTextureFromImage(
            allocator: *mut c_void,
            textureCache: CVMetalTextureCacheRef,
            sourceImage: CVImageBufferRef,
            textureAttributes: *mut c_void,
            pixelFormat: usize,
            width: usize,
            height: usize,
            planeIndex: usize,
            textureOut: *mut CVMetalTextureRef,
        ) -> CVReturn;

        pub fn CVMetalTextureGetTexture(image: CVMetalTextureRef) -> *mut c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub fn CFRelease(c: *mut c_void);
    }
}

pub struct WgpuFrameRenderer<'window> {
    surface: wgpu::Surface<'window>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    copy_render_pipeline: wgpu::RenderPipeline,
    nv12_render_pipeline: wgpu::RenderPipeline,
    copy_bind_group_layout: wgpu::BindGroupLayout,
    nv12_bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    latest_frame: Option<XbxEngineRenderFrame>,
    frame_texture: Option<FrameTextureBundle>,
    #[cfg(target_os = "macos")]
    texture_cache: Option<macos_bindings::CVMetalTextureCacheRef>,
    #[cfg(target_os = "macos")]
    pending_releases: std::collections::VecDeque<(
        u64,
        (
            macos_bindings::CVMetalTextureRef,
            macos_bindings::CVMetalTextureRef,
        ),
    )>,
}

impl<'window> WgpuFrameRenderer<'window> {
    pub async fn new(window: &'window Window) -> Result<Self, String> {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window)
            .map_err(|error| format!("createWgpuSurfaceFailed:{error}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
            })
            .await
            .ok_or_else(|| "xbxengineAppWgpuAdapterUnavailable".to_string())?;
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("xbxengine-app-device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: adapter.limits(),
                },
                None,
            )
            .await
            .map_err(|error| format!("createXbxEngineAppDeviceFailed:{error}"))?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(surface_caps.formats[0]);
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: choose_present_mode(&surface_caps),
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        let copy_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xbxengine-app-copy-shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(COPY_SHADER)),
        });
        let nv12_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("xbxengine-app-nv12-shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(NV12_SHADER)),
        });

        let copy_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("xbxengine-app-copy-bind-group-layout"),
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
                label: Some("xbxengine-app-nv12-bind-group-layout"),
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
                ],
            });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("xbxengine-app-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let copy_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xbxengine-app-copy-pipeline-layout"),
            bind_group_layouts: &[&copy_bind_group_layout],
            push_constant_ranges: &[],
        });
        let copy_render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xbxengine-app-copy-render-pipeline"),
            layout: Some(&copy_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &copy_shader,
                entry_point: "vs_main",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &copy_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let nv12_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("xbxengine-app-nv12-pipeline-layout"),
            bind_group_layouts: &[&nv12_bind_group_layout],
            push_constant_ranges: &[],
        });
        let nv12_render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("xbxengine-app-nv12-render-pipeline"),
            layout: Some(&nv12_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &nv12_shader,
                entry_point: "vs_main",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &nv12_shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        #[cfg(target_os = "macos")]
        let texture_cache = unsafe {
            let mut cache: macos_bindings::CVMetalTextureCacheRef = std::ptr::null_mut();
            device.as_hal::<wgpu_hal::api::Metal, _, _>(|hal_device| {
                if let Some(dev) = hal_device {
                    let mtl_device = dev.raw_device().lock().as_ptr() as *mut std::ffi::c_void;
                    macos_bindings::CVMetalTextureCacheCreate(
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        mtl_device,
                        std::ptr::null_mut(),
                        &mut cache,
                    );
                }
            });
            if cache.is_null() {
                None
            } else {
                Some(cache)
            }
        };

        Ok(Self {
            surface,
            device,
            queue,
            surface_config,
            copy_render_pipeline,
            nv12_render_pipeline,
            copy_bind_group_layout,
            nv12_bind_group_layout,
            sampler,
            latest_frame: None,
            frame_texture: None,
            #[cfg(target_os = "macos")]
            texture_cache,
            #[cfg(target_os = "macos")]
            pending_releases: std::collections::VecDeque::new(),
        })
    }

    pub fn update_frame(&mut self, frame: XbxEngineRenderFrame) {
        xbxengine::xbx_log_warn!(
            "[xbxengine-app] update_frame seq={} data={:?}",
            frame.frame_seq,
            frame.pixel_data
        );
        self.latest_frame = Some(frame);
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }

        self.surface_config.width = size.width;
        self.surface_config.height = size.height;
        self.surface.configure(&self.device, &self.surface_config);
    }

    pub fn render(&mut self) -> Result<(), String> {
        if let Some(frame) = self.latest_frame.take() {
            xbxengine::xbx_log_warn!("[xbxengine-app] rendering frame seq={}", frame.frame_seq);
            self.upload_frame(&frame)?;

            #[cfg(target_os = "macos")]
            {
                // 逐帧清理：释放 5 帧之前的纹理，确保 GPU 已渲染完。
                let current_seq = frame.frame_seq;
                while self
                    .pending_releases
                    .front()
                    .map(|f| f.0 < current_seq.saturating_sub(5))
                    .unwrap_or(false)
                {
                    let (seq, (y, uv)) = self.pending_releases.pop_front().unwrap();
                    unsafe {
                        xbxengine::xbx_log_warn!(
                            "[xbxengine-app] deferred release CVMetalTextureRef for seq={}",
                            seq
                        );
                        macos_bindings::CFRelease(y);
                        macos_bindings::CFRelease(uv);
                    }
                }
            }
        }

        let surface_texture = match self.surface.get_current_texture() {
            Ok(texture) => texture,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.device, &self.surface_config);
                return Ok(());
            }
            Err(wgpu::SurfaceError::Timeout) => {
                return Ok(());
            }
            Err(wgpu::SurfaceError::OutOfMemory) => {
                return Err("xbxengineAppSurfaceOutOfMemory".to_string());
            }
        };

        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("xbxengine-app-render-encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("xbxengine-app-render-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.02,
                            g: 0.03,
                            b: 0.04,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            if let Some(frame_texture) = self.frame_texture.as_ref() {
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
                let expected_len = frame.width as usize * frame.height as usize * 4;
                if bytes.len() != expected_len {
                    return Err(format!(
                        "xbxengineAppFrameSizeMismatch:expected={expected_len}:actual={}",
                        bytes.len()
                    ));
                }
                self.ensure_rgba_texture(
                    frame.width,
                    frame.height,
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                );
                let Some(FrameTextureBundle::Rgba(bundle)) = self.frame_texture.as_ref() else {
                    return Err("xbxengineAppFrameTextureMissing".to_string());
                };
                self.queue.write_texture(
                    wgpu::ImageCopyTexture {
                        texture: &bundle.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    bytes.as_ref(),
                    wgpu::ImageDataLayout {
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
                let expected_len = frame.width as usize * frame.height as usize * 4;
                if bytes.len() != expected_len {
                    return Err(format!(
                        "xbxengineAppFrameSizeMismatch:expected={expected_len}:actual={}",
                        bytes.len()
                    ));
                }
                self.ensure_rgba_texture(
                    frame.width,
                    frame.height,
                    wgpu::TextureFormat::Bgra8UnormSrgb,
                );
                let Some(FrameTextureBundle::Bgra(bundle)) = self.frame_texture.as_ref() else {
                    return Err("xbxengineAppFrameTextureMissing".to_string());
                };
                self.queue.write_texture(
                    wgpu::ImageCopyTexture {
                        texture: &bundle.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    bytes.as_ref(),
                    wgpu::ImageDataLayout {
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
                if *y_stride < frame.width || *uv_stride < frame.width {
                    return Err("xbxengineAppNv12StrideInvalid".to_string());
                }
                self.ensure_nv12_texture(frame.width, frame.height);
                let Some(FrameTextureBundle::Nv12(bundle)) = self.frame_texture.as_ref() else {
                    return Err("xbxengineAppFrameTextureMissing".to_string());
                };
                self.queue.write_texture(
                    wgpu::ImageCopyTexture {
                        texture: &bundle.y_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    y_plane.as_ref(),
                    wgpu::ImageDataLayout {
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
                    wgpu::ImageCopyTexture {
                        texture: &bundle.uv_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    uv_plane.as_ref(),
                    wgpu::ImageDataLayout {
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
                #[cfg(target_os = "macos")]
                {
                    if let Some(desc) = handle
                        .downcast_ref::<xbxengine::api::backend::MacOsCVPixelBufferDescriptor>()
                    {
                        if let Some(cache) = self.texture_cache {
                            let pixel_buffer = desc.ptr;

                            // Plane 0: Y
                            let mut cv_y_tex: macos_bindings::CVMetalTextureRef =
                                std::ptr::null_mut();
                            unsafe {
                                macos_bindings::CVMetalTextureCacheCreateTextureFromImage(
                                    std::ptr::null_mut(),
                                    cache,
                                    pixel_buffer,
                                    std::ptr::null_mut(),
                                    10, // MTLPixelFormatR8Unorm = 10
                                    frame.width as _,
                                    frame.height as _,
                                    0,
                                    &mut cv_y_tex,
                                );
                            }

                            // Plane 1: UV
                            let mut cv_uv_tex: macos_bindings::CVMetalTextureRef =
                                std::ptr::null_mut();
                            unsafe {
                                macos_bindings::CVMetalTextureCacheCreateTextureFromImage(
                                    std::ptr::null_mut(),
                                    cache,
                                    pixel_buffer,
                                    std::ptr::null_mut(),
                                    30, // MTLPixelFormatRG8Unorm = 30
                                    (frame.width / 2) as _,
                                    frame.height.div_ceil(2) as _,
                                    1,
                                    &mut cv_uv_tex,
                                );
                            }

                            if cv_y_tex.is_null() || cv_uv_tex.is_null() {
                                if !cv_y_tex.is_null() {
                                    unsafe { macos_bindings::CFRelease(cv_y_tex) };
                                }
                                if !cv_uv_tex.is_null() {
                                    unsafe { macos_bindings::CFRelease(cv_uv_tex) };
                                }
                                return Err("xbxengineAppCreateMetalTextureFailed".to_string());
                            }

                            let mtl_y_tex =
                                unsafe { macos_bindings::CVMetalTextureGetTexture(cv_y_tex) };
                            let mtl_uv_tex =
                                unsafe { macos_bindings::CVMetalTextureGetTexture(cv_uv_tex) };
                            xbxengine::xbx_log_warn!("[xbxengine-app] CVMetalTextureCacheCreateTextureFromImage success, planes extracted");

                            if let Some(FrameTextureBundle::Nv12(bundle)) =
                                self.frame_texture.as_mut()
                            {
                                if let Some(textures) = bundle.cv_textures.take() {
                                    self.pending_releases.push_back((frame.frame_seq, textures));
                                }
                                bundle.cv_textures = Some((cv_y_tex, cv_uv_tex));
                            }

                            let (y_texture, uv_texture) = unsafe {
                                use metal::foreign_types::ForeignType;
                                use wgpu_hal::api::Metal;

                                let raw_y =
                                    metal::Texture::from_ptr(mtl_y_tex as *mut _).to_owned();
                                let hal_y = <Metal as wgpu_hal::Api>::Device::texture_from_raw(
                                    raw_y,
                                    wgpu::TextureFormat::R8Unorm,
                                    metal::MTLTextureType::D2,
                                    1,
                                    1,
                                    wgpu_hal::CopyExtent {
                                        width: frame.width,
                                        height: frame.height,
                                        depth: 1,
                                    },
                                );
                                let wgpu_y = self.device.create_texture_from_hal::<Metal>(
                                    hal_y,
                                    &wgpu::TextureDescriptor {
                                        label: Some("xbxengine-app-frame-y-texture-cv"),
                                        size: wgpu::Extent3d {
                                            width: frame.width,
                                            height: frame.height,
                                            depth_or_array_layers: 1,
                                        },
                                        mip_level_count: 1,
                                        sample_count: 1,
                                        dimension: wgpu::TextureDimension::D2,
                                        format: wgpu::TextureFormat::R8Unorm,
                                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                                            | wgpu::TextureUsages::COPY_DST,
                                        view_formats: &[],
                                    },
                                );

                                let raw_uv =
                                    metal::Texture::from_ptr(mtl_uv_tex as *mut _).to_owned();
                                let hal_uv = <Metal as wgpu_hal::Api>::Device::texture_from_raw(
                                    raw_uv,
                                    wgpu::TextureFormat::Rg8Unorm,
                                    metal::MTLTextureType::D2,
                                    1,
                                    1,
                                    wgpu_hal::CopyExtent {
                                        width: frame.width / 2,
                                        height: frame.height.div_ceil(2),
                                        depth: 1,
                                    },
                                );
                                let wgpu_uv = self.device.create_texture_from_hal::<Metal>(
                                    hal_uv,
                                    &wgpu::TextureDescriptor {
                                        label: Some("xbxengine-app-frame-uv-texture-cv"),
                                        size: wgpu::Extent3d {
                                            width: frame.width / 2,
                                            height: frame.height.div_ceil(2),
                                            depth_or_array_layers: 1,
                                        },
                                        mip_level_count: 1,
                                        sample_count: 1,
                                        dimension: wgpu::TextureDimension::D2,
                                        format: wgpu::TextureFormat::Rg8Unorm,
                                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                                            | wgpu::TextureUsages::COPY_DST,
                                        view_formats: &[],
                                    },
                                );

                                (wgpu_y, wgpu_uv)
                            };

                            let y_view =
                                y_texture.create_view(&wgpu::TextureViewDescriptor::default());
                            let uv_view =
                                uv_texture.create_view(&wgpu::TextureViewDescriptor::default());
                            let bind_group =
                                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                    label: Some("xbxengine-app-nv12-bind-group-cv"),
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
                                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                                        },
                                    ],
                                });

                            self.frame_texture =
                                Some(FrameTextureBundle::Nv12(Nv12TextureBundle {
                                    width: frame.width,
                                    height: frame.height,
                                    y_texture,
                                    uv_texture,
                                    bind_group,
                                    #[cfg(target_os = "macos")]
                                    cv_textures: Some((cv_y_tex, cv_uv_tex)),
                                }));

                            // NOTE: CFRelease is now handled in Nv12TextureBundle::drop to ensure
                            // textures are valid as long as the wgpu texture exists.
                        } else {
                            return Err("xbxengineAppTextureCacheMissing".to_string());
                        }
                    } else {
                        return Err("xbxengineAppInvalidDescriptor".to_string());
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    return Err("xbxengineAppDescriptorNotSupportedOnThisPlatform".to_string());
                }
            }
        }
        Ok(())
    }

    fn ensure_rgba_texture(&mut self, width: u32, height: u32, format: wgpu::TextureFormat) {
        let matches_existing = match (self.frame_texture.as_ref(), format) {
            (Some(FrameTextureBundle::Rgba(bundle)), wgpu::TextureFormat::Rgba8UnormSrgb) => {
                bundle.width == width && bundle.height == height
            }
            (Some(FrameTextureBundle::Bgra(bundle)), wgpu::TextureFormat::Bgra8UnormSrgb) => {
                bundle.width == width && bundle.height == height
            }
            _ => false,
        };
        let needs_recreate = !matches_existing;
        if !needs_recreate {
            return;
        }

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xbxengine-app-frame-texture"),
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
            label: Some("xbxengine-app-frame-bind-group"),
            layout: &self.copy_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let bundle = RgbaTextureBundle {
            width,
            height,
            texture,
            bind_group,
        };
        self.frame_texture = Some(if format == wgpu::TextureFormat::Bgra8UnormSrgb {
            FrameTextureBundle::Bgra(bundle)
        } else {
            FrameTextureBundle::Rgba(bundle)
        });
    }

    fn ensure_nv12_texture(&mut self, width: u32, height: u32) {
        let needs_recreate = !matches!(
            self.frame_texture.as_ref(),
            Some(FrameTextureBundle::Nv12(bundle)) if bundle.width == width && bundle.height == height
        );
        if !needs_recreate {
            return;
        }

        let y_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xbxengine-app-frame-y-texture"),
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
            label: Some("xbxengine-app-frame-uv-texture"),
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
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("xbxengine-app-nv12-bind-group"),
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
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        self.frame_texture = Some(FrameTextureBundle::Nv12(Nv12TextureBundle {
            width,
            height,
            y_texture,
            uv_texture,
            bind_group,
            #[cfg(target_os = "macos")]
            cv_textures: None,
        }));
    }
}

fn choose_present_mode(capabilities: &wgpu::SurfaceCapabilities) -> wgpu::PresentMode {
    if capabilities
        .present_modes
        .contains(&wgpu::PresentMode::AutoVsync)
    {
        return wgpu::PresentMode::AutoVsync;
    }
    wgpu::PresentMode::Fifo
}
