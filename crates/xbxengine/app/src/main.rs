mod stdio_mode;
mod wgpu_renderer;

use std::sync::{Arc, Mutex};

use stdio_mode::StdioSidecarMode;
use wgpu_renderer::WgpuFrameRenderer;
use winit::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use xbxengine::XbxEngineRenderFrame;
use xbxengine_app::{
    build_window_title, SharedStateXbxEngineWindowHost, SharedXbxEngineWindowState, XbxEngineApp,
    XbxEngineAppHostBridge,
};
use xbxengine_protocol::{
    XbxEngineControlCommandDto, XbxEngineHostRequestDto, XbxEngineHostResponseDto,
    XbxEngineSessionDto, XbxEngineTargetTypeDto, XbxEngineViewportDto,
};

fn main() {
    let event_loop = EventLoop::new().expect("xbxengine event loop should initialize");
    let shared_window_state = SharedXbxEngineWindowState::default();
    let stdio_enabled = std::env::args().any(|arg| arg == "--stdio");

    let (app, stdio_mode) = if stdio_enabled {
        let (app, mode) = StdioSidecarMode::spawn(shared_window_state.clone())
            .expect("stdio sidecar mode should initialize");
        (app, Some(mode))
    } else {
        let app = Arc::new(Mutex::new(XbxEngineApp::with_window_host(
            Box::new(PrintlnHostBridge),
            Box::new(SharedStateXbxEngineWindowHost::new(
                shared_window_state.clone(),
            )),
        )));

        if let Ok(mut app) = app.lock() {
            let runtime_start_result =
                app.handle_control(XbxEngineControlCommandDto::StartRuntime {
                    session: XbxEngineSessionDto {
                        session_id: "bootstrap-session".to_string(),
                        target_type: XbxEngineTargetTypeDto::Cloud,
                        turn_server: None,
                    },
                    viewport: XbxEngineViewportDto {
                        viewport_id: "bootstrap-viewport".to_string(),
                    },
                    audio_volume: 1.0,
                });
            if let Err(error) = runtime_start_result {
                eprintln!("[xbxengine-app] runtime bootstrap failed: {error}");
            }
        }

        (app, None)
    };

    let window: &'static winit::window::Window = Box::leak(Box::new(
        WindowBuilder::new()
            .with_title(build_window_title(&shared_window_state.snapshot()))
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
            .build(&event_loop)
            .expect("xbxengine native window should open"),
    ));
    let mut renderer =
        pollster::block_on(WgpuFrameRenderer::new(window)).expect("wgpu renderer should start");
    let mut bootstrap_frame_seq = 0_u64;
    let mut has_received_remote_frame = false;
    let mut stdio_mode = stdio_mode;

    let _ = event_loop.run(move |event, event_loop_window_target| {
        event_loop_window_target.set_control_flow(ControlFlow::Wait);

        match event {
            Event::AboutToWait => {
                if stdio_mode.is_none() {
                    if let Ok(mut app) = app.lock() {
                        app.tick();
                        for runtime_event in app.drain_events() {
                            eprintln!("runtime-event: {runtime_event:?}");
                        }
                    }
                }

                let window_state = shared_window_state.snapshot();

                let mut rendered_real_frame = false;
                if let Ok(mut app) = app.lock() {
                    if let Ok(Some(frame)) = app.take_latest_render_frame() {
                        renderer.update_frame(frame);
                        rendered_real_frame = true;
                        has_received_remote_frame = true;
                    }
                }

                // 测试图案只用于“首帧到来前”的占位。
                // 一旦已经收到远端真实帧，就继续保留上一张真实帧，避免因某一拍没有新帧而闪回 bootstrap。
                if !rendered_real_frame && !has_received_remote_frame {
                    bootstrap_frame_seq = bootstrap_frame_seq.saturating_add(1);
                    renderer.update_frame(build_bootstrap_frame(bootstrap_frame_seq));
                }

                window.set_title(&build_window_title(&window_state));
                // 仅在有新帧（真实帧或 bootstrap 动画帧）时请求 redraw，避免空转重绘。
                if rendered_real_frame || !has_received_remote_frame {
                    window.request_redraw();
                }
            }
            Event::WindowEvent {
                window_id,
                event: WindowEvent::RedrawRequested,
            } if window_id == window.id() => {
                if let Err(error) = renderer.render() {
                    eprintln!("[xbxengine-app] render failed: {error}");
                }
            }
            Event::WindowEvent { window_id, event } if window_id == window.id() => match event {
                WindowEvent::Resized(size) => {
                    renderer.resize(size);
                    window.request_redraw();
                }
                WindowEvent::CloseRequested => {
                    if let Some(mut active_stdio_mode) = stdio_mode.take() {
                        active_stdio_mode.shutdown();
                    } else if let Ok(mut app) = app.lock() {
                        let _ = app.handle_control(XbxEngineControlCommandDto::StopRuntime);
                    }
                    event_loop_window_target.exit();
                }
                _ => {}
            },
            _ => {}
        }
    });
}

fn build_bootstrap_frame(frame_seq: u64) -> XbxEngineRenderFrame {
    let width = 640_u32;
    let height = 360_u32;
    let mut rgba_bytes = vec![0_u8; (width * height * 4) as usize];

    for y in 0..height {
        for x in 0..width {
            let pixel_index = ((y * width + x) * 4) as usize;
            let wave = ((x as u64 + frame_seq * 3) % 255) as u8;
            let band = ((y as u64 * 2 + frame_seq * 5) % 255) as u8;
            rgba_bytes[pixel_index] = wave;
            rgba_bytes[pixel_index + 1] = band;
            rgba_bytes[pixel_index + 2] = 255_u8.saturating_sub(wave / 2);
            rgba_bytes[pixel_index + 3] = 255;
        }
    }

    XbxEngineRenderFrame {
        width,
        height,
        frame_seq,
        rendered_at_ms: frame_seq as f64,
        rgba_bytes: Arc::from(rgba_bytes),
    }
}

struct PrintlnHostBridge;

impl XbxEngineAppHostBridge for PrintlnHostBridge {
    fn request(
        &mut self,
        request: XbxEngineHostRequestDto,
    ) -> Result<XbxEngineHostResponseDto, xbxengine::XbxEngineRuntimeError> {
        eprintln!("host-bridge request: {request:?}");
        match request {
            XbxEngineHostRequestDto::ExchangeOffer { .. } => {
                Ok(XbxEngineHostResponseDto::OfferExchanged {
                    answer_sdp: "bootstrap-answer".to_string(),
                })
            }
            XbxEngineHostRequestDto::ExchangeIce { .. } => {
                Ok(XbxEngineHostResponseDto::IceExchanged {
                    candidates: Vec::new(),
                })
            }
            XbxEngineHostRequestDto::KeepAliveRemoteSession { .. } => {
                Ok(XbxEngineHostResponseDto::KeepAliveAccepted)
            }
            XbxEngineHostRequestDto::CloseRemoteSession { .. } => {
                Ok(XbxEngineHostResponseDto::RemoteSessionClosed)
            }
        }
    }
}
