use std::{num::NonZeroU32, sync::Arc};

use softbuffer::{Context, Surface};
use xbxengine::XbxEngineDecodedVideoFrame;
use xbxengine_app::XbxEngineWindowState;
use tao::{dpi::PhysicalSize, event_loop::EventLoopWindowTarget, window::Window};

/**
 * 这里先提供一个最小的软件渲染器：
 * - 输入是 Rust 软解后的 RGB 帧
 * - 输出是 softbuffer 绑定的原生窗口
 * 后续如果换成 GPU 渲染，窗口宿主和缩放逻辑仍可保留。
 */
pub struct SoftwareFrameRenderer<'a> {
    surface: Surface<Arc<Window>, Arc<Window>>,
    latest_frame: Option<XbxEngineDecodedVideoFrame>,
    last_surface_size: PhysicalSize<u32>,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> SoftwareFrameRenderer<'a> {
    pub fn new(
        _event_loop: &EventLoopWindowTarget<()>,
        window: Arc<Window>,
    ) -> Result<Self, String> {
        let context = Context::new(window.clone()).map_err(|error| error.to_string())?;
        let surface = Surface::new(&context, window.clone()).map_err(|error| error.to_string())?;
        Ok(Self {
            surface,
            latest_frame: None,
            last_surface_size: window.inner_size(),
            _marker: std::marker::PhantomData,
        })
    }

    pub fn update_frame(&mut self, frame: XbxEngineDecodedVideoFrame) {
        self.latest_frame = Some(frame);
    }

    pub fn render(
        &mut self,
        window: &Window,
        _window_state: &XbxEngineWindowState,
    ) -> Result<(), String> {
        let Some(frame) = self.latest_frame.as_ref() else {
            return Ok(());
        };

        let mut size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            size = PhysicalSize::new(frame.width.max(1), frame.height.max(1));
        }

        if self.last_surface_size != size {
            self.surface
                .resize(non_zero(size.width)?, non_zero(size.height)?)
                .map_err(|error| error.to_string())?;
            self.last_surface_size = size;
        }

        let mut buffer = self
            .surface
            .buffer_mut()
            .map_err(|error| error.to_string())?;
        blit_scaled_frame(
            &frame.pixels,
            frame.width,
            frame.height,
            &mut buffer,
            size.width,
            size.height,
        );
        buffer.present().map_err(|error| error.to_string())
    }
}

fn non_zero(value: u32) -> Result<NonZeroU32, String> {
    NonZeroU32::new(value).ok_or_else(|| "softwareRendererSurfaceSizeZero".to_string())
}

fn blit_scaled_frame(
    source: &[u32],
    source_width: u32,
    source_height: u32,
    target: &mut [u32],
    target_width: u32,
    target_height: u32,
) {
    if source_width == 0 || source_height == 0 || target_width == 0 || target_height == 0 {
        return;
    }

    let source_width_usize = source_width as usize;
    let target_width_usize = target_width as usize;

    for y in 0..target_height {
        let source_y = ((y as u64) * (source_height as u64) / (target_height as u64)) as usize;
        let target_row_start = (y as usize) * target_width_usize;
        let source_row_start = source_y * source_width_usize;
        for x in 0..target_width {
            let source_x = ((x as u64) * (source_width as u64) / (target_width as u64)) as usize;
            target[target_row_start + x as usize] = source[source_row_start + source_x];
        }
    }
}
