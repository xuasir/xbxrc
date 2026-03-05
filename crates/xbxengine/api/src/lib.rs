#[cfg(feature = "napi")]
mod gamepad_json_contract;

#[cfg(feature = "napi")]
use gamepad_json_contract::{
    TsGamepadRouteTarget, TsGamepadRumbleRequest, TsGamepadRumbleResult, TsGamepadRumbleTarget,
    TsGamepadRuntimeSnapshot, TsGamepadSamplingConfig, TsGamepadSamplingStrategy,
    TsLogicalPadBinding,
};
#[cfg(feature = "napi")]
use napi::{
    threadsafe_function::{
        ErrorStrategy, ThreadSafeCallContext, ThreadsafeFunction, ThreadsafeFunctionCallMode,
    },
    Error as NapiError, JsFunction, JsString, Result as NapiResult, Status,
};
#[cfg(feature = "napi")]
use napi_derive::napi;
#[cfg(feature = "napi")]
use ohmygamepad_host::GamepadRuntimeHost;
#[cfg(feature = "napi")]
use ohmygamepad_protocol::{
    LogicalButtonsStateDto, LogicalPadBindingDto, LogicalPadStateDto,
    MultiControllerSamplingStrategyDto, OhMyGamepadRouteTargetDto, OhMyGamepadRumbleRequestDto,
    OhMyGamepadRumbleTargetDto, OhMyGamepadSamplingConfigDto,
};
#[cfg(feature = "napi")]
use serde::{de::DeserializeOwned, Deserialize, Serialize};
#[cfg(feature = "napi")]
use std::collections::HashMap;
#[cfg(feature = "napi")]
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc, Mutex,
};
#[cfg(feature = "napi")]
use std::thread::{self, JoinHandle};
#[cfg(feature = "napi")]
use std::time::Duration;

#[cfg(feature = "napi")]
use xbxengine::{
    create_active_media_backend, OhMyGamepadXbxEngineInputBackend, XbxEngineEventSink,
    XbxEngineHostBridge, XbxEngineRttDiagnosticsRuntimeConfig, XbxEngineRuntime,
    XbxEngineRuntimeConfig, XbxEngineRuntimeError, XbxEngineVideoPipelineRuntimeConfig,
};
#[cfg(feature = "napi")]
use xbxengine_protocol::{
    XbxEngineControlResponseDto, XbxEngineHostRequestDto, XbxEngineHostResponseDto,
    XbxEngineIncomingMessageDto, XbxEngineOutgoingMessageDto, XbxEngineRuntimeEventDto,
    XbxEngineStatsDto,
};

#[cfg(feature = "napi")]
type PendingHostResponses = Arc<Mutex<HashMap<String, mpsc::Sender<HostResponseResult>>>>;
#[cfg(feature = "napi")]
type HostResponseResult = Result<XbxEngineHostResponseDto, String>;
#[cfg(feature = "napi")]
const HOST_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(feature = "napi")]
const RUNTIME_SNAPSHOT_STOP_CHECK_INTERVAL_MS: u64 = 100;
#[cfg(feature = "napi")]
const XBXENGINE_VIRTUAL_DEVICE_ID: &str = "virtual:xbxengine-controller";

#[cfg(feature = "napi")]
#[derive(Deserialize)]
struct TsGamepadButtonPressPayload {
    button: String,
    duration_ms: u64,
}

#[cfg(feature = "napi")]
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TsXbxEngineRuntimeConfig {
    runtime_name: Option<String>,
    webrtc: Option<TsXbxEngineWebRtcRuntimeConfig>,
}

#[cfg(feature = "napi")]
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TsXbxEngineWebRtcRuntimeConfig {
    forced_remb_kbps: Option<u32>,
    adaptive_remb_enabled: Option<bool>,
    video_pipeline: Option<TsXbxEngineVideoPipelineRuntimeConfig>,
    rtt_diagnostics: Option<TsXbxEngineRttDiagnosticsRuntimeConfig>,
}

#[cfg(feature = "napi")]
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TsXbxEngineVideoPipelineRuntimeConfig {
    nack_window_ms: Option<u64>,
    nack_retry_interval_ms: Option<u64>,
    nack_max_retry_count: Option<u8>,
}

#[cfg(feature = "napi")]
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TsXbxEngineRttDiagnosticsRuntimeConfig {
    enabled: Option<bool>,
    log_interval_ms: Option<u64>,
}

#[cfg(feature = "napi")]
#[derive(Default)]
struct SharedOutgoingMessages {
    messages: Mutex<Vec<XbxEngineOutgoingMessageDto>>,
}

#[cfg(feature = "napi")]
impl SharedOutgoingMessages {
    fn push(&self, message: XbxEngineOutgoingMessageDto) {
        self.messages
            .lock()
            .expect("lock outgoing messages")
            .push(message);
    }

    fn drain(&self) -> Vec<XbxEngineOutgoingMessageDto> {
        self.messages
            .lock()
            .expect("lock outgoing messages")
            .drain(..)
            .collect()
    }
}

#[cfg(feature = "napi")]
struct NativeHostBridge {
    outgoing_messages: Arc<SharedOutgoingMessages>,
    pending_host_responses: PendingHostResponses,
    next_request_id: Arc<AtomicU64>,
    shutdown_flag: Arc<AtomicBool>,
}

#[cfg(feature = "napi")]
impl XbxEngineHostBridge for NativeHostBridge {
    fn request(
        &mut self,
        request: XbxEngineHostRequestDto,
    ) -> Result<XbxEngineHostResponseDto, XbxEngineRuntimeError> {
        let request_name = match &request {
            XbxEngineHostRequestDto::ExchangeOffer { .. } => "ExchangeOffer",
            XbxEngineHostRequestDto::ExchangeIce { .. } => "ExchangeIce",
            XbxEngineHostRequestDto::KeepAliveRemoteSession { .. } => "KeepAliveRemoteSession",
            XbxEngineHostRequestDto::CloseRemoteSession { .. } => "CloseRemoteSession",
        };
        let request_id = format!(
            "host-{}",
            self.next_request_id.fetch_add(1, Ordering::Relaxed) + 1
        );
        let (sender, receiver) = mpsc::channel::<HostResponseResult>();
        self.pending_host_responses
            .lock()
            .expect("lock pending host responses")
            .insert(request_id.clone(), sender);
        self.outgoing_messages
            .push(XbxEngineOutgoingMessageDto::HostRequest {
                request_id: request_id.clone(),
                request,
            });
        let deadline = std::time::Instant::now() + HOST_REQUEST_TIMEOUT;

        loop {
            if self.shutdown_flag.load(Ordering::Relaxed) {
                self.pending_host_responses
                    .lock()
                    .expect("lock pending host responses")
                    .remove(&request_id);
                return Err(XbxEngineRuntimeError::new("xbxEngineNativeBindingClosed"));
            }

            if std::time::Instant::now() >= deadline {
                self.pending_host_responses
                    .lock()
                    .expect("lock pending host responses")
                    .remove(&request_id);
                return Err(XbxEngineRuntimeError::new(format!(
                    "xbxEngineHostRequestTimedOut:{request_name}:{request_id}"
                )));
            }

            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(response)) => return Ok(response),
                Ok(Err(message)) => return Err(XbxEngineRuntimeError::new(message)),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(XbxEngineRuntimeError::new(
                        "xbxEngineHostResponseChannelClosed",
                    ))
                }
            }
        }
    }
}

#[cfg(feature = "napi")]
struct NativeEventSink {
    outgoing_messages: Arc<SharedOutgoingMessages>,
    last_runtime_event: Arc<Mutex<Option<XbxEngineRuntimeEventDto>>>,
}

#[cfg(feature = "napi")]
impl XbxEngineEventSink for NativeEventSink {
    fn emit(&mut self, event: XbxEngineRuntimeEventDto) {
        *self
            .last_runtime_event
            .lock()
            .expect("lock last runtime event") = Some(event.clone());
        self.outgoing_messages
            .push(XbxEngineOutgoingMessageDto::RuntimeEvent { event });
    }
}

#[cfg(feature = "napi")]
enum WorkerCommand {
    IncomingMessage(XbxEngineIncomingMessageDto),
    SnapshotStats {
        response: mpsc::Sender<Result<XbxEngineStatsDto, String>>,
    },
    Shutdown {
        response: mpsc::Sender<()>,
    },
}

#[cfg(feature = "napi")]
struct XbxEngineNativeWorker {
    sender: mpsc::Sender<WorkerCommand>,
    outgoing_messages: Arc<SharedOutgoingMessages>,
    last_runtime_event: Arc<Mutex<Option<XbxEngineRuntimeEventDto>>>,
    pending_host_responses: PendingHostResponses,
    shutdown_flag: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

#[cfg(feature = "napi")]
impl XbxEngineNativeWorker {
    fn spawn(runtime_config: XbxEngineRuntimeConfig) -> Self {
        let (sender, receiver) = mpsc::channel::<WorkerCommand>();
        let outgoing_messages = Arc::new(SharedOutgoingMessages::default());
        let last_runtime_event = Arc::new(Mutex::new(None));
        let pending_host_responses: PendingHostResponses = Arc::new(Mutex::new(HashMap::new()));
        let shutdown_flag = Arc::new(AtomicBool::new(false));
        let next_request_id = Arc::new(AtomicU64::new(0));

        let worker_outgoing_messages = outgoing_messages.clone();
        let worker_last_runtime_event = last_runtime_event.clone();
        let worker_pending_host_responses = pending_host_responses.clone();
        let worker_shutdown_flag = shutdown_flag.clone();
        let worker_next_request_id = next_request_id.clone();

        let handle = thread::spawn(move || {
            worker_outgoing_messages.push(XbxEngineOutgoingMessageDto::Ready);

            let mut runtime = XbxEngineRuntime::with_media_backend(
                runtime_config.clone(),
                NativeHostBridge {
                    outgoing_messages: worker_outgoing_messages.clone(),
                    pending_host_responses: worker_pending_host_responses.clone(),
                    next_request_id: worker_next_request_id,
                    shutdown_flag: worker_shutdown_flag.clone(),
                },
                NativeEventSink {
                    outgoing_messages: worker_outgoing_messages.clone(),
                    last_runtime_event: worker_last_runtime_event,
                },
                create_active_media_backend(
                    Box::new(OhMyGamepadXbxEngineInputBackend::new()),
                    runtime_config,
                ),
            );

            loop {
                match receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(command) => match command {
                        WorkerCommand::IncomingMessage(message) => match message {
                            XbxEngineIncomingMessageDto::ControlRequest {
                                request_id,
                                command,
                            } => {
                                let response = match runtime.apply_control(command) {
                                    Ok(()) => XbxEngineOutgoingMessageDto::ControlResponse {
                                        request_id,
                                        response: XbxEngineControlResponseDto::Ack,
                                    },
                                    Err(error) => XbxEngineOutgoingMessageDto::ControlError {
                                        request_id,
                                        message: error.to_string(),
                                    },
                                };
                                worker_outgoing_messages.push(response);
                            }
                            XbxEngineIncomingMessageDto::HostResponse { .. }
                            | XbxEngineIncomingMessageDto::HostError { .. } => {}
                        },
                        WorkerCommand::SnapshotStats { response } => {
                            let _ = response.send(Ok(runtime.snapshot_stats()));
                        }
                        WorkerCommand::Shutdown { response } => {
                            worker_shutdown_flag.store(true, Ordering::Relaxed);
                            runtime.stop();
                            let _ = response.send(());
                            break;
                        }
                    },
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        runtime.tick();
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });

        Self {
            sender,
            outgoing_messages,
            last_runtime_event,
            pending_host_responses,
            shutdown_flag,
            handle: Some(handle),
        }
    }

    fn send_incoming_message(&self, message: XbxEngineIncomingMessageDto) -> Result<(), String> {
        if self.try_resolve_host_response(&message) {
            return Ok(());
        }
        self.sender
            .send(WorkerCommand::IncomingMessage(message))
            .map_err(|error| format!("sendXbxEngineIncomingMessageFailed:{error}"))
    }

    // HostResponse/HostError 必须绕过 worker 主循环，避免 runtime 内同步 request 等待时自锁。
    fn try_resolve_host_response(&self, message: &XbxEngineIncomingMessageDto) -> bool {
        match message {
            XbxEngineIncomingMessageDto::HostResponse {
                request_id,
                response,
            } => {
                if let Some(sender) = self
                    .pending_host_responses
                    .lock()
                    .expect("lock pending host responses")
                    .remove(request_id)
                {
                    let _ = sender.send(Ok(response.clone()));
                }
                true
            }
            XbxEngineIncomingMessageDto::HostError {
                request_id,
                message,
            } => {
                if let Some(sender) = self
                    .pending_host_responses
                    .lock()
                    .expect("lock pending host responses")
                    .remove(request_id)
                {
                    let _ = sender.send(Err(message.clone()));
                }
                true
            }
            XbxEngineIncomingMessageDto::ControlRequest { .. } => false,
        }
    }

    fn snapshot_stats(&self) -> Result<XbxEngineStatsDto, String> {
        let (sender, receiver) = mpsc::channel();
        self.sender
            .send(WorkerCommand::SnapshotStats { response: sender })
            .map_err(|error| format!("requestXbxEngineStatsFailed:{error}"))?;
        receiver
            .recv()
            .map_err(|error| format!("recvXbxEngineStatsFailed:{error}"))?
    }

    fn drain_outgoing_messages(&self) -> Vec<XbxEngineOutgoingMessageDto> {
        self.outgoing_messages.drain()
    }

    fn last_runtime_event(&self) -> Option<XbxEngineRuntimeEventDto> {
        self.last_runtime_event
            .lock()
            .expect("lock last runtime event")
            .clone()
    }

    fn shutdown(&mut self) {
        self.shutdown_flag.store(true, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        let _ = self
            .sender
            .send(WorkerCommand::Shutdown { response: sender });
        let _ = receiver.recv_timeout(Duration::from_secs(1));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(feature = "napi")]
struct RuntimeSnapshotSubscription {
    stop_tx: mpsc::Sender<()>,
    join_handle: JoinHandle<()>,
}

#[cfg(feature = "napi")]
impl RuntimeSnapshotSubscription {
    fn stop(self) {
        let _ = self.stop_tx.send(());
        let _ = self.join_handle.join();
    }
}

#[cfg(feature = "napi")]
#[napi(js_name = "XbxEngineGamepadNativeBinding")]
pub struct XbxEngineGamepadNativeBinding {
    host: Option<GamepadRuntimeHost>,
    init_error: Option<String>,
    runtime_snapshot_subscription: Mutex<Option<RuntimeSnapshotSubscription>>,
}

#[cfg(feature = "napi")]
#[napi]
impl XbxEngineGamepadNativeBinding {
    #[napi(constructor)]
    pub fn new() -> Self {
        match GamepadRuntimeHost::shared() {
            Ok(host) => Self {
                host: Some(host),
                init_error: None,
                runtime_snapshot_subscription: Mutex::new(None),
            },
            Err(error) => Self {
                host: None,
                init_error: Some(format!("XbxEngine gamepad bootstrap failed: {error}")),
                runtime_snapshot_subscription: Mutex::new(None),
            },
        }
    }

    #[napi]
    pub fn get_runtime_snapshot_json(&self) -> NapiResult<String> {
        let host = self.host()?;
        let snapshot = host
            .snapshot()
            .map_err(|error| map_napi_error(format!("snapshotGamepadHost:{error:?}")))?;
        serialize_ts_json(TsGamepadRuntimeSnapshot::from(snapshot))
    }

    #[napi]
    pub fn set_route_target_json(&self, target_json: String) -> NapiResult<String> {
        let target = parse_ts_json::<TsGamepadRouteTarget>(&target_json)?;
        let route_target =
            OhMyGamepadRouteTargetDto::try_from(target).map_err(json_parse_error_to_napi)?;
        let host = self.host()?;
        host.set_route_target(route_target)
            .map_err(|error| map_napi_error(format!("setGamepadRouteTarget:{error:?}")))?;
        let snapshot = host
            .snapshot()
            .map_err(|error| map_napi_error(format!("snapshotGamepadHost:{error:?}")))?;
        serialize_ts_json(TsGamepadRuntimeSnapshot::from(snapshot))
    }

    #[napi]
    pub fn update_sampling_json(&self, sampling_json: String) -> NapiResult<String> {
        let sampling = parse_ts_json::<TsGamepadSamplingConfig>(&sampling_json)?;
        let sampling =
            OhMyGamepadSamplingConfigDto::try_from(sampling).map_err(json_parse_error_to_napi)?;
        let host = self.host()?;
        host.set_sampling(sampling)
            .map_err(|error| map_napi_error(format!("setGamepadSampling:{error:?}")))?;
        let snapshot = host
            .snapshot()
            .map_err(|error| map_napi_error(format!("snapshotGamepadHost:{error:?}")))?;
        serialize_ts_json(TsGamepadRuntimeSnapshot::from(snapshot))
    }

    #[napi]
    pub fn rebind_logical_pad_json(&self, binding_json: String) -> NapiResult<String> {
        let binding = parse_ts_json::<TsLogicalPadBinding>(&binding_json)?;
        let binding = LogicalPadBindingDto::try_from(binding).map_err(json_parse_error_to_napi)?;
        let host = self.host()?;
        host.rebind_logical_pad(binding)
            .map_err(|error| map_napi_error(format!("rebindGamepadLogicalPad:{error:?}")))?;
        let snapshot = host
            .snapshot()
            .map_err(|error| map_napi_error(format!("snapshotGamepadHost:{error:?}")))?;
        serialize_ts_json(TsGamepadRuntimeSnapshot::from(snapshot))
    }

    #[napi]
    pub fn set_sampling_strategy_json(&self, strategy_json: String) -> NapiResult<String> {
        let strategy = parse_ts_json::<TsGamepadSamplingStrategy>(&strategy_json)?;
        let strategy = MultiControllerSamplingStrategyDto::try_from(strategy)
            .map_err(json_parse_error_to_napi)?;
        let host = self.host()?;
        host.set_sampling_strategy(strategy)
            .map_err(|error| map_napi_error(format!("setGamepadSamplingStrategy:{error:?}")))?;
        let snapshot = host
            .snapshot()
            .map_err(|error| map_napi_error(format!("snapshotGamepadHost:{error:?}")))?;
        serialize_ts_json(TsGamepadRuntimeSnapshot::from(snapshot))
    }

    #[napi]
    pub fn set_primary_sampling_device_json(&self, device_id_json: String) -> NapiResult<String> {
        let device_id = parse_ts_json::<Option<String>>(&device_id_json)?;
        let host = self.host()?;
        host.set_primary_sampling_device(device_id)
            .map_err(|error| {
                map_napi_error(format!("setGamepadPrimarySamplingDevice:{error:?}"))
            })?;
        let snapshot = host
            .snapshot()
            .map_err(|error| map_napi_error(format!("snapshotGamepadHost:{error:?}")))?;
        serialize_ts_json(TsGamepadRuntimeSnapshot::from(snapshot))
    }

    #[napi]
    pub fn pause_sampling_device_json(&self, device_id_json: String) -> NapiResult<String> {
        let device_id = parse_ts_json::<String>(&device_id_json)?;
        let host = self.host()?;
        host.pause_sampling_device(&device_id)
            .map_err(|error| map_napi_error(format!("pauseGamepadSamplingDevice:{error:?}")))?;
        let snapshot = host
            .snapshot()
            .map_err(|error| map_napi_error(format!("snapshotGamepadHost:{error:?}")))?;
        serialize_ts_json(TsGamepadRuntimeSnapshot::from(snapshot))
    }

    #[napi]
    pub fn resume_sampling_device_json(&self, device_id_json: String) -> NapiResult<String> {
        let device_id = parse_ts_json::<String>(&device_id_json)?;
        let host = self.host()?;
        host.resume_sampling_device(&device_id)
            .map_err(|error| map_napi_error(format!("resumeGamepadSamplingDevice:{error:?}")))?;
        let snapshot = host
            .snapshot()
            .map_err(|error| map_napi_error(format!("snapshotGamepadHost:{error:?}")))?;
        serialize_ts_json(TsGamepadRuntimeSnapshot::from(snapshot))
    }

    #[napi]
    pub fn play_rumble_json(&self, request_json: String) -> NapiResult<String> {
        let request = parse_ts_json::<TsGamepadRumbleRequest>(&request_json)?;
        let request =
            OhMyGamepadRumbleRequestDto::try_from(request).map_err(json_parse_error_to_napi)?;
        let host = self.host()?;
        let result = host
            .play_rumble(request)
            .map_err(|error| map_napi_error(format!("playGamepadRumble:{error:?}")))?;
        serialize_ts_json(TsGamepadRumbleResult::from(result))
    }

    #[napi]
    pub fn stop_rumble_json(&self, target_json: String) -> NapiResult<String> {
        let target = parse_ts_json::<TsGamepadRumbleTarget>(&target_json)?;
        let target =
            OhMyGamepadRumbleTargetDto::try_from(target).map_err(json_parse_error_to_napi)?;
        let host = self.host()?;
        let result = host
            .stop_rumble(target)
            .map_err(|error| map_napi_error(format!("stopGamepadRumble:{error:?}")))?;
        serialize_ts_json(TsGamepadRumbleResult::from(result))
    }

    #[napi]
    pub fn press_controller_button_json(&self, request_json: String) -> NapiResult<String> {
        let request = parse_ts_json::<TsGamepadButtonPressPayload>(&request_json)?;
        let host = self.host()?;
        host.submit_simulated_state(
            XBXENGINE_VIRTUAL_DEVICE_ID,
            logical_state_for_button(&request.button, request.duration_ms)
                .map_err(map_napi_error)?,
        )
        .map_err(|error| map_napi_error(format!("pressGamepadControllerButton:{error:?}")))?;
        host.submit_simulated_state(XBXENGINE_VIRTUAL_DEVICE_ID, LogicalPadStateDto::default())
            .map_err(|error| map_napi_error(format!("releaseGamepadControllerButton:{error:?}")))?;
        let snapshot = host
            .snapshot()
            .map_err(|error| map_napi_error(format!("snapshotGamepadHost:{error:?}")))?;
        serialize_ts_json(TsGamepadRuntimeSnapshot::from(snapshot))
    }

    #[napi]
    pub fn subscribe_runtime_snapshot(
        &self,
        snapshot_callback: JsFunction,
        error_callback: Option<JsFunction>,
    ) -> NapiResult<()> {
        self.ensure_initialized()?;
        self.stop_runtime_snapshot_subscription();

        let snapshot_push = snapshot_callback.create_threadsafe_function(
            0,
            |ctx: ThreadSafeCallContext<String>| -> NapiResult<Vec<JsString>> {
                Ok(vec![ctx.env.create_string_from_std(ctx.value)?])
            },
        )?;
        let error_push = error_callback
            .map(create_string_threadsafe_function)
            .transpose()?;
        let snapshot_rx = self.host()?.subscribe_runtime_snapshot();
        let subscription =
            spawn_runtime_snapshot_subscription(snapshot_rx, snapshot_push, error_push);
        *self
            .runtime_snapshot_subscription
            .lock()
            .expect("lock runtime snapshot subscription") = Some(subscription);
        Ok(())
    }

    #[napi]
    pub fn unsubscribe_runtime_snapshot(&self) -> NapiResult<()> {
        self.ensure_initialized()?;
        self.stop_runtime_snapshot_subscription();
        Ok(())
    }

    #[napi]
    pub fn shutdown(&self) -> NapiResult<()> {
        self.ensure_initialized()?;
        self.stop_runtime_snapshot_subscription();
        Ok(())
    }
}

#[cfg(feature = "napi")]
impl XbxEngineGamepadNativeBinding {
    fn host(&self) -> NapiResult<&GamepadRuntimeHost> {
        self.ensure_initialized()?;
        self.host.as_ref().ok_or_else(closed_binding_error)
    }

    fn ensure_initialized(&self) -> NapiResult<()> {
        if let Some(init_error) = &self.init_error {
            return Err(NapiError::new(Status::GenericFailure, init_error.clone()));
        }
        Ok(())
    }

    fn stop_runtime_snapshot_subscription(&self) {
        if let Some(subscription) = self
            .runtime_snapshot_subscription
            .lock()
            .expect("lock runtime snapshot subscription")
            .take()
        {
            subscription.stop();
        }
    }
}

#[cfg(feature = "napi")]
#[napi(js_name = "XbxEngineNativeBinding")]
pub struct XbxEngineNativeBinding {
    worker: Mutex<Option<XbxEngineNativeWorker>>,
    runtime_config: Mutex<XbxEngineRuntimeConfig>,
}

#[cfg(feature = "napi")]
#[napi]
impl XbxEngineNativeBinding {
    #[napi(constructor)]
    pub fn new() -> Self {
        let runtime_config = XbxEngineRuntimeConfig::default();
        Self {
            worker: Mutex::new(Some(XbxEngineNativeWorker::spawn(runtime_config.clone()))),
            runtime_config: Mutex::new(runtime_config),
        }
    }

    #[napi]
    pub fn set_runtime_config_json(&self, runtime_config_json: String) -> NapiResult<()> {
        let config = parse_ts_json::<TsXbxEngineRuntimeConfig>(&runtime_config_json)?;
        let runtime_config = resolve_runtime_config(config);
        *self
            .runtime_config
            .lock()
            .expect("lock xbxengine runtime config") = runtime_config.clone();

        let mut worker = self.worker()?;
        if let Some(worker) = worker.as_mut() {
            worker.shutdown();
        }
        *worker = Some(XbxEngineNativeWorker::spawn(runtime_config));
        Ok(())
    }

    #[napi]
    pub fn send_incoming_message_json(&self, message_json: String) -> NapiResult<()> {
        let message = parse_ts_json::<XbxEngineIncomingMessageDto>(&message_json)?;
        let worker = self.worker()?;
        worker
            .as_ref()
            .ok_or_else(closed_binding_error)?
            .send_incoming_message(message)
            .map_err(map_napi_error)
    }

    #[napi]
    pub fn drain_outgoing_messages_json(&self) -> NapiResult<String> {
        let worker = self.worker()?;
        let messages = worker
            .as_ref()
            .ok_or_else(closed_binding_error)?
            .drain_outgoing_messages();
        serialize_ts_json(messages)
    }

    #[napi]
    pub fn snapshot_stats_json(&self) -> NapiResult<String> {
        let worker = self.worker()?;
        let stats = worker
            .as_ref()
            .ok_or_else(closed_binding_error)?
            .snapshot_stats()
            .map_err(map_napi_error)?;
        serialize_ts_json(stats)
    }

    #[napi]
    pub fn get_last_runtime_event_json(&self) -> NapiResult<String> {
        let worker = self.worker()?;
        let event = worker
            .as_ref()
            .ok_or_else(closed_binding_error)?
            .last_runtime_event();
        serialize_ts_json(event)
    }

    #[napi]
    pub fn shutdown(&self) -> NapiResult<()> {
        let mut worker = self.worker()?;
        if let Some(worker) = worker.as_mut() {
            worker.shutdown();
        }
        *worker = None;
        Ok(())
    }
}

#[cfg(feature = "napi")]
impl XbxEngineNativeBinding {
    fn worker(&self) -> NapiResult<std::sync::MutexGuard<'_, Option<XbxEngineNativeWorker>>> {
        Ok(self.worker.lock().expect("lock xbxengine worker"))
    }
}

#[cfg(feature = "napi")]
fn resolve_runtime_config(input: TsXbxEngineRuntimeConfig) -> XbxEngineRuntimeConfig {
    let mut runtime_config = XbxEngineRuntimeConfig::default();
    if let Some(runtime_name) = input.runtime_name {
        let trimmed = runtime_name.trim();
        if !trimmed.is_empty() {
            runtime_config.runtime_name = trimmed.to_string();
        }
    }

    if let Some(webrtc) = input.webrtc {
        if let Some(forced_remb_kbps) = webrtc.forced_remb_kbps {
            runtime_config.webrtc.forced_remb_kbps = Some(forced_remb_kbps);
        }
        if let Some(adaptive_remb_enabled) = webrtc.adaptive_remb_enabled {
            runtime_config.webrtc.adaptive_remb_enabled = adaptive_remb_enabled;
        }
        if let Some(video_pipeline) = webrtc.video_pipeline {
            runtime_config.webrtc.video_pipeline =
                resolve_video_pipeline_runtime_config(video_pipeline);
        }
        if let Some(rtt_diagnostics) = webrtc.rtt_diagnostics {
            runtime_config.webrtc.rtt_diagnostics =
                resolve_rtt_diagnostics_runtime_config(rtt_diagnostics);
        }
    }
    runtime_config
}

#[cfg(feature = "napi")]
fn resolve_video_pipeline_runtime_config(
    input: TsXbxEngineVideoPipelineRuntimeConfig,
) -> XbxEngineVideoPipelineRuntimeConfig {
    let default = XbxEngineVideoPipelineRuntimeConfig::default();
    XbxEngineVideoPipelineRuntimeConfig {
        nack_window_ms: input
            .nack_window_ms
            .unwrap_or(default.nack_window_ms)
            .max(1),
        nack_retry_interval_ms: input
            .nack_retry_interval_ms
            .unwrap_or(default.nack_retry_interval_ms)
            .max(1),
        nack_max_retry_count: input
            .nack_max_retry_count
            .unwrap_or(default.nack_max_retry_count)
            .max(1),
    }
}

#[cfg(feature = "napi")]
fn resolve_rtt_diagnostics_runtime_config(
    input: TsXbxEngineRttDiagnosticsRuntimeConfig,
) -> XbxEngineRttDiagnosticsRuntimeConfig {
    let default = XbxEngineRttDiagnosticsRuntimeConfig::default();
    XbxEngineRttDiagnosticsRuntimeConfig {
        enabled: input.enabled.unwrap_or(default.enabled),
        log_interval_ms: input
            .log_interval_ms
            .unwrap_or(default.log_interval_ms)
            .max(500),
    }
}

#[cfg(feature = "napi")]
fn parse_ts_json<T>(value: &str) -> NapiResult<T>
where
    T: DeserializeOwned,
{
    serde_json::from_str(value).map_err(|error| {
        NapiError::new(
            Status::InvalidArg,
            format!("failed to parse xbxengine JSON payload: {error}"),
        )
    })
}

#[cfg(feature = "napi")]
fn serialize_ts_json<T>(value: T) -> NapiResult<String>
where
    T: Serialize,
{
    serde_json::to_string(&value).map_err(|error| {
        NapiError::new(
            Status::GenericFailure,
            format!("failed to serialize xbxengine JSON payload: {error}"),
        )
    })
}

#[cfg(feature = "napi")]
fn closed_binding_error() -> NapiError {
    NapiError::new(
        Status::GenericFailure,
        "XbxEngine native binding has already been shut down".to_owned(),
    )
}

#[cfg(feature = "napi")]
fn json_parse_error_to_napi(message: String) -> NapiError {
    NapiError::new(
        Status::InvalidArg,
        format!("failed to parse xbxengine gamepad JSON payload: {message}"),
    )
}

#[cfg(feature = "napi")]
fn map_napi_error(message: String) -> NapiError {
    NapiError::new(
        Status::GenericFailure,
        format!("XbxEngine native operation failed: {message}"),
    )
}

#[cfg(feature = "napi")]
fn create_string_threadsafe_function(
    callback: JsFunction,
) -> NapiResult<ThreadsafeFunction<String, ErrorStrategy::Fatal>> {
    callback.create_threadsafe_function(
        0,
        |ctx: ThreadSafeCallContext<String>| -> NapiResult<Vec<JsString>> {
            Ok(vec![ctx.env.create_string_from_std(ctx.value)?])
        },
    )
}

#[cfg(feature = "napi")]
fn logical_state_for_button(button: &str, _duration_ms: u64) -> Result<LogicalPadStateDto, String> {
    let mut buttons = LogicalButtonsStateDto::default();
    match button {
        "south" | "a" => buttons.south = 1.0,
        "east" | "b" => buttons.east = 1.0,
        "west" | "x" => buttons.west = 1.0,
        "north" | "y" => buttons.north = 1.0,
        "l1" | "lb" => buttons.l1 = 1.0,
        "r1" | "rb" => buttons.r1 = 1.0,
        "l2" | "lt" => buttons.l2 = 1.0,
        "r2" | "rt" => buttons.r2 = 1.0,
        "l3" => buttons.l3 = 1.0,
        "r3" => buttons.r3 = 1.0,
        "view" | "back" => buttons.view = 1.0,
        "menu" | "start" => buttons.menu = 1.0,
        "home" | "nexus" | "guide" => buttons.home = 1.0,
        "dpad-up" => buttons.dpad_up = 1.0,
        "dpad-down" => buttons.dpad_down = 1.0,
        "dpad-left" => buttons.dpad_left = 1.0,
        "dpad-right" => buttons.dpad_right = 1.0,
        _ => return Err(format!("unsupportedXbxEngineControllerButton:{button}")),
    }

    Ok(LogicalPadStateDto {
        buttons,
        ..LogicalPadStateDto::default()
    })
}

#[cfg(feature = "napi")]
fn spawn_runtime_snapshot_subscription(
    snapshot_rx: mpsc::Receiver<ohmygamepad_protocol::OhMyGamepadRuntimeSnapshotDto>,
    snapshot_push: ThreadsafeFunction<String, ErrorStrategy::Fatal>,
    error_push: Option<ThreadsafeFunction<String, ErrorStrategy::Fatal>>,
) -> RuntimeSnapshotSubscription {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let join_handle = thread::spawn(move || loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        match snapshot_rx.recv_timeout(Duration::from_millis(
            RUNTIME_SNAPSHOT_STOP_CHECK_INTERVAL_MS,
        )) {
            Ok(snapshot) => {
                match serde_json::to_string(&TsGamepadRuntimeSnapshot::from(snapshot)) {
                    Ok(snapshot_json) => {
                        let _ = snapshot_push
                            .call(snapshot_json, ThreadsafeFunctionCallMode::NonBlocking);
                    }
                    Err(error) => {
                        if let Some(error_push) = &error_push {
                            let _ = error_push.call(
                                format!("failed to serialize runtime snapshot: {error}"),
                                ThreadsafeFunctionCallMode::NonBlocking,
                            );
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    });

    RuntimeSnapshotSubscription {
        stop_tx,
        join_handle,
    }
}
