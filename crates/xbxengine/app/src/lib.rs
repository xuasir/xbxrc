mod window;

use std::sync::{Arc, Mutex};

use xbxengine::{
    create_active_media_backend, NoopXbxEngineInputBackend, XbxEngineEventSink,
    XbxEngineHostBridge, XbxEngineInputBackend, XbxEngineMediaBackend, XbxEngineRenderFrame,
    XbxEngineRuntime, XbxEngineRuntimeConfig, XbxEngineRuntimeError, XbxEngineRuntimeSnapshot,
};
use xbxengine_protocol::{
    XbxEngineControlCommandDto, XbxEngineHostRequestDto, XbxEngineHostResponseDto,
    XbxEngineRuntimeEventDto,
};

pub use window::*;

pub trait XbxEngineAppHostBridge: Send {
    fn request(
        &mut self,
        request: XbxEngineHostRequestDto,
    ) -> Result<XbxEngineHostResponseDto, XbxEngineRuntimeError>;
}

#[derive(Default)]
pub struct NoopXbxEngineAppHostBridge;

impl XbxEngineAppHostBridge for NoopXbxEngineAppHostBridge {
    fn request(
        &mut self,
        _request: XbxEngineHostRequestDto,
    ) -> Result<XbxEngineHostResponseDto, XbxEngineRuntimeError> {
        Err(XbxEngineRuntimeError::new(
            "xbxEngineAppHostBridgeUnavailable",
        ))
    }
}

struct SharedHostBridge {
    inner: Arc<Mutex<Box<dyn XbxEngineAppHostBridge>>>,
}

impl XbxEngineHostBridge for SharedHostBridge {
    fn request(
        &mut self,
        request: XbxEngineHostRequestDto,
    ) -> Result<XbxEngineHostResponseDto, XbxEngineRuntimeError> {
        self.inner
            .lock()
            .expect("lock xbxengine app host bridge")
            .request(request)
    }
}

#[derive(Default, Clone)]
struct SharedEventSink {
    events: Arc<Mutex<Vec<XbxEngineRuntimeEventDto>>>,
}

impl XbxEngineEventSink for SharedEventSink {
    fn emit(&mut self, event: XbxEngineRuntimeEventDto) {
        self.events
            .lock()
            .expect("lock xbxengine app events")
            .push(event);
    }
}

/**
 * sidecar app 负责在进程内托管 runtime，并把控制命令、宿主 bridge、事件回收串起来。
 */
pub struct XbxEngineApp {
    runtime: XbxEngineRuntime<SharedHostBridge, SharedEventSink, Box<dyn XbxEngineMediaBackend>>,
    event_sink: SharedEventSink,
    window_host: Box<dyn XbxEngineWindowHost>,
    drained_events: Vec<XbxEngineRuntimeEventDto>,
}

impl XbxEngineApp {
    pub fn new(host_bridge: Box<dyn XbxEngineAppHostBridge>) -> Self {
        Self::with_runtime_hosts(
            host_bridge,
            Box::<NoopXbxEngineInputBackend>::default(),
            Box::<NoopXbxEngineWindowHost>::default(),
        )
    }

    pub fn with_input_backend(
        host_bridge: Box<dyn XbxEngineAppHostBridge>,
        input_backend: Box<dyn XbxEngineInputBackend>,
    ) -> Self {
        Self::with_runtime_hosts(
            host_bridge,
            input_backend,
            Box::<NoopXbxEngineWindowHost>::default(),
        )
    }

    pub fn with_window_host(
        host_bridge: Box<dyn XbxEngineAppHostBridge>,
        window_host: Box<dyn XbxEngineWindowHost>,
    ) -> Self {
        Self::with_runtime_hosts(
            host_bridge,
            Box::<NoopXbxEngineInputBackend>::default(),
            window_host,
        )
    }

    pub fn with_runtime_hosts(
        host_bridge: Box<dyn XbxEngineAppHostBridge>,
        input_backend: Box<dyn XbxEngineInputBackend>,
        mut window_host: Box<dyn XbxEngineWindowHost>,
    ) -> Self {
        let event_sink = SharedEventSink::default();
        let host_bridge = SharedHostBridge {
            inner: Arc::new(Mutex::new(host_bridge)),
        };
        window_host.open_window("Rust Stream Window");
        let runtime_config = XbxEngineRuntimeConfig::default();
        let media_backend = create_active_media_backend(input_backend, runtime_config.clone());
        let runtime = XbxEngineRuntime::with_media_backend(
            runtime_config,
            host_bridge,
            event_sink.clone(),
            media_backend,
        );

        Self {
            runtime,
            event_sink,
            window_host,
            drained_events: Vec::new(),
        }
    }

    pub fn handle_control(
        &mut self,
        command: XbxEngineControlCommandDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        let result = self.runtime.apply_control(command);
        self.flush_runtime_events();
        result
    }

    pub fn snapshot(&self) -> &XbxEngineRuntimeSnapshot {
        self.runtime.snapshot()
    }

    pub fn tick(&mut self) {
        self.runtime.tick();
        self.flush_runtime_events();
    }

    pub fn take_latest_render_frame(
        &mut self,
    ) -> Result<Option<XbxEngineRenderFrame>, XbxEngineRuntimeError> {
        self.runtime.take_latest_render_frame()
    }

    pub fn window_snapshot(&self) -> XbxEngineWindowState {
        self.window_host.snapshot()
    }

    pub fn drain_events(&mut self) -> Vec<XbxEngineRuntimeEventDto> {
        self.flush_runtime_events();
        self.drained_events.drain(..).collect()
    }

    fn flush_runtime_events(&mut self) {
        let mut events = self
            .event_sink
            .events
            .lock()
            .expect("lock xbxengine app events");
        for event in events.drain(..) {
            self.window_host.apply_event(&event);
            self.drained_events.push(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use xbxengine::{XbxEngineInputStatus, XbxEngineRuntimeError};
    use xbxengine_protocol::{
        XbxEngineControlCommandDto, XbxEngineHostRequestDto, XbxEngineHostResponseDto,
        XbxEngineInputEventDto, XbxEngineRuntimeEventDto, XbxEngineSessionDto,
        XbxEngineTargetTypeDto, XbxEngineViewportDto,
    };

    use super::{XbxEngineApp, XbxEngineAppHostBridge, XbxEngineWindowHost, XbxEngineWindowState};

    struct TestHostBridge;

    impl XbxEngineAppHostBridge for TestHostBridge {
        fn request(
            &mut self,
            request: XbxEngineHostRequestDto,
        ) -> Result<XbxEngineHostResponseDto, XbxEngineRuntimeError> {
            Ok(match request {
                XbxEngineHostRequestDto::ExchangeOffer { .. } => {
                    XbxEngineHostResponseDto::OfferExchanged {
                        answer_sdp: "answer".to_string(),
                    }
                }
                XbxEngineHostRequestDto::ExchangeIce { .. } => {
                    XbxEngineHostResponseDto::IceExchanged {
                        candidates: Vec::new(),
                    }
                }
                XbxEngineHostRequestDto::KeepAliveRemoteSession { .. } => {
                    XbxEngineHostResponseDto::KeepAliveAccepted
                }
                XbxEngineHostRequestDto::CloseRemoteSession { .. } => {
                    XbxEngineHostResponseDto::RemoteSessionClosed
                }
            })
        }
    }

    #[derive(Default)]
    struct TestInputBackend;

    impl xbxengine::XbxEngineInputBackend for TestInputBackend {
        fn attach_session(
            &mut self,
            _session_id: &str,
        ) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
            Ok(XbxEngineInputStatus {
                device_count: 1,
                pad_count: 1,
                route_attached: true,
            })
        }

        fn press_controller_button(
            &mut self,
            _button: &str,
            _duration_ms: u64,
        ) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
            Ok(XbxEngineInputStatus {
                device_count: 1,
                pad_count: 1,
                route_attached: true,
            })
        }

        fn snapshot_status(&self) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
            Ok(XbxEngineInputStatus {
                device_count: 1,
                pad_count: 1,
                route_attached: true,
            })
        }

        fn stop(&mut self) -> Result<(), XbxEngineRuntimeError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct TestWindowHost {
        state: XbxEngineWindowState,
    }

    impl XbxEngineWindowHost for TestWindowHost {
        fn open_window(&mut self, title: &str) {
            self.state.title = title.to_string();
        }

        fn apply_event(&mut self, event: &XbxEngineRuntimeEventDto) {
            match event {
                XbxEngineRuntimeEventDto::RuntimePhaseChanged { phase } => {
                    self.state.runtime_phase = Some(phase.clone());
                }
                XbxEngineRuntimeEventDto::TransportConnectionStateChanged { state } => {
                    self.state.transport_state = Some(state.clone());
                }
                XbxEngineRuntimeEventDto::MediaSurfaceReady { surface_id } => {
                    self.state.surface_id = Some(surface_id.clone());
                }
                XbxEngineRuntimeEventDto::MediaVideoReady { width, height } => {
                    self.state.video_size = Some((*width, *height));
                }
                _ => {}
            }
        }

        fn snapshot(&self) -> XbxEngineWindowState {
            self.state.clone()
        }
    }

    fn session() -> XbxEngineSessionDto {
        XbxEngineSessionDto {
            session_id: "session-1".to_string(),
            target_type: XbxEngineTargetTypeDto::Cloud,
            turn_server: None,
        }
    }

    fn viewport() -> XbxEngineViewportDto {
        XbxEngineViewportDto {
            viewport_id: "viewport-1".to_string(),
        }
    }

    #[test]
    fn app_drives_runtime_and_drains_events() {
        let mut app = XbxEngineApp::with_runtime_hosts(
            Box::new(TestHostBridge),
            Box::new(TestInputBackend),
            Box::new(TestWindowHost::default()),
        );

        app.handle_control(XbxEngineControlCommandDto::StartRuntime {
            session: session(),
            viewport: viewport(),
            audio_volume: 0.8,
        })
        .expect("runtime should start");
        app.handle_control(XbxEngineControlCommandDto::PushKeyboardPointerInput {
            event: XbxEngineInputEventDto::Keyboard {
                at_ms: 1,
                event: "down".to_string(),
                code: "KeyA".to_string(),
                key: "a".to_string(),
                repeat: false,
                ctrl_key: false,
                shift_key: false,
                alt_key: false,
                meta_key: false,
            },
        })
        .expect("input forwarding should succeed");

        assert_eq!(app.snapshot().audio_volume, 0.8);
        assert_eq!(app.snapshot().input_device_count, 1);
        assert!(app.snapshot().input_route_attached);
        assert_eq!(
            app.window_snapshot(),
            XbxEngineWindowState {
                title: "Rust Stream Window".to_string(),
                surface_id: Some("wgpu:viewport-1".to_string()),
                video_size: Some((1920, 1080)),
                runtime_phase: Some(xbxengine_protocol::XbxEngineRuntimePhaseDto::Connecting,),
                transport_state: Some(xbxengine_protocol::XbxEngineTransportStateDto::Connected,),
                last_error: None,
            }
        );

        let events = app.drain_events();
        assert!(events
            .iter()
            .any(|event| matches!(event, XbxEngineRuntimeEventDto::MediaSurfaceReady { .. })));
        assert!(events.iter().any(|event| matches!(
            event,
            XbxEngineRuntimeEventDto::StatsVideoFrameProcessed { .. }
        )));
    }
}
