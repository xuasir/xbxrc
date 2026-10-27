use super::GamepadProvider;
use ohmygamepad_protocol::{
    OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleResultDto, OhMyGamepadRumbleTargetDto,
    OhMyGamepadRuntimeSnapshotDto,
};
use ohmygamepad_sdl3::{OhMyGamepadService, OhMyGamepadServiceConfig};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::JoinHandle;
use std::time::Duration;
use tauri::AppHandle;

struct NativeSession {
    runtime: OhMyGamepadService,
    stopped: Arc<AtomicBool>,
    publisher: JoinHandle<()>,
}

/// 桌面原生后端只持有设备采样和触觉；输入归属由公共控制器管理。
pub struct GamepadService {
    app_handle: AppHandle,
    native: Mutex<Option<NativeSession>>,
}

impl GamepadService {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            app_handle,
            native: Mutex::new(None),
        }
    }
}

impl GamepadProvider for GamepadService {
    fn start_backend(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        let mut native = self.native.lock().map_err(|e| e.to_string())?;
        if let Some(session) = native.as_ref() {
            return session.runtime.snapshot().map_err(|e| format!("{e:?}"));
        }
        let mut config = OhMyGamepadServiceConfig::default();
        config.desktop_keyboard = None;
        config.sampling_strategy.enable_keyboard_fallback = false;
        let runtime = OhMyGamepadService::spawn(config).map_err(|e| e.to_string())?;
        let initial = runtime.snapshot().map_err(|e| format!("{e:?}"));
        if initial.is_err() {
            let _ = runtime.shutdown();
            return initial;
        }
        let receiver = runtime.subscribe_runtime_snapshot();
        let stopped = Arc::new(AtomicBool::new(false));
        let publisher_stop = stopped.clone();
        let app = self.app_handle.clone();
        let publisher = std::thread::spawn(move || {
            while !publisher_stop.load(Ordering::Acquire) {
                match receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(snapshot) => {
                        if publisher_stop.load(Ordering::Acquire) {
                            break;
                        }
                        if let Ok(value) = serde_json::to_value(snapshot) {
                            let _ = super::events::emit_runtime_snapshot(&app, &value);
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        *native = Some(NativeSession {
            runtime,
            stopped,
            publisher,
        });
        initial
    }

    fn stop_backend(&self) -> Result<(), String> {
        // 锁覆盖关闭过程，使后续启动等待设备句柄完全释放。
        let mut native = self.native.lock().map_err(|e| e.to_string())?;
        if let Some(session) = native.take() {
            session.stopped.store(true, Ordering::Release);
            let _ = session
                .runtime
                .stop_rumble(OhMyGamepadRumbleTargetDto::Auto);
            let result = session.runtime.shutdown().map_err(|e| format!("{e:?}"));
            let _ = session.publisher.join();
            result?;
        }
        Ok(())
    }

    fn get_runtime_snapshot(&self) -> Result<OhMyGamepadRuntimeSnapshotDto, String> {
        let native = self.native.lock().map_err(|e| e.to_string())?;
        match native.as_ref() {
            Some(session) => session.runtime.snapshot().map_err(|e| format!("{e:?}")),
            None => Ok(OhMyGamepadRuntimeSnapshotDto::default()),
        }
    }

    fn play_rumble(
        &self,
        request: OhMyGamepadRumbleRequestDto,
    ) -> Result<OhMyGamepadRumbleResultDto, String> {
        let native = self.native.lock().map_err(|e| e.to_string())?;
        let session = native.as_ref().ok_or("nativeBackendStopped")?;
        session
            .runtime
            .play_rumble(request)
            .map_err(|e| format!("{e:?}"))
    }

    fn stop_rumble(
        &self,
        target: OhMyGamepadRumbleTargetDto,
    ) -> Result<OhMyGamepadRumbleResultDto, String> {
        let native = self.native.lock().map_err(|e| e.to_string())?;
        let session = native.as_ref().ok_or("nativeBackendStopped")?;
        session
            .runtime
            .stop_rumble(target)
            .map_err(|e| format!("{e:?}"))
    }
}
