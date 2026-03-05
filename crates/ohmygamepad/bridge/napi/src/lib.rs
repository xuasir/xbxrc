mod json_contract;

use json_contract::{
    TsGamepadRouteTarget, TsGamepadRumbleRequest, TsGamepadRumbleResult, TsGamepadRumbleTarget,
    TsGamepadRuntimeSnapshot, TsGamepadSamplingConfig, TsGamepadSamplingStrategy,
    TsLogicalPadBinding,
};
use napi::{
    threadsafe_function::{
        ErrorStrategy, ThreadSafeCallContext, ThreadsafeFunction, ThreadsafeFunctionCallMode,
    },
    Error as NapiError, JsFunction, JsString, Result as NapiResult, Status,
};
use napi_derive::napi;
use ohmygamepad_gilrs::{OhMyGamepadService, OhMyGamepadServiceConfig};
use ohmygamepad_protocol::{
    LogicalPadBindingDto, MultiControllerSamplingStrategyDto, OhMyGamepadRouteTargetDto,
    OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleTargetDto, OhMyGamepadSamplingConfigDto,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};

const RUNTIME_SNAPSHOT_STOP_CHECK_INTERVAL_MS: u64 = 100;

#[napi(js_name = "OhMyGamepadNativeBinding")]
pub struct OhMyGamepadNativeBinding {
    runtime: Arc<Mutex<Option<OhMyGamepadService>>>,
    init_error: Option<String>,
    runtime_snapshot_subscription: Mutex<Option<RuntimeSnapshotSubscription>>,
}

struct RuntimeSnapshotSubscription {
    stop_tx: mpsc::Sender<()>,
    join_handle: thread::JoinHandle<()>,
}

impl RuntimeSnapshotSubscription {
    fn stop(self) {
        let _ = self.stop_tx.send(());
        let _ = self.join_handle.join();
    }
}

#[napi]
impl OhMyGamepadNativeBinding {
    #[napi(constructor)]
    pub fn new() -> Self {
        match OhMyGamepadService::spawn(OhMyGamepadServiceConfig::default()) {
            Ok(runtime) => Self {
                runtime: Arc::new(Mutex::new(Some(runtime))),
                init_error: None,
                runtime_snapshot_subscription: Mutex::new(None),
            },
            Err(error) => Self {
                runtime: Arc::new(Mutex::new(None)),
                init_error: Some(format!("OhMyGamepad native bootstrap failed: {error}")),
                runtime_snapshot_subscription: Mutex::new(None),
            },
        }
    }

    #[napi]
    pub fn get_runtime_snapshot_json(&self) -> NapiResult<String> {
        let runtime = self.runtime()?;
        let snapshot = runtime
            .as_ref()
            .ok_or_else(closed_binding_error)?
            .snapshot()
            .map_err(map_napi_error)?;
        serialize_ts_json(TsGamepadRuntimeSnapshot::from(snapshot))
    }

    #[napi]
    pub fn set_route_target_json(&self, target_json: String) -> NapiResult<String> {
        let target = parse_ts_json::<TsGamepadRouteTarget>(&target_json)?;
        let route_target =
            OhMyGamepadRouteTargetDto::try_from(target).map_err(json_parse_error_to_napi)?;
        let runtime = self.runtime()?;
        let runtime = runtime.as_ref().ok_or_else(closed_binding_error)?;
        runtime
            .set_route_target(route_target)
            .map_err(map_napi_error)?;
        let snapshot = runtime.snapshot().map_err(map_napi_error)?;
        serialize_ts_json(TsGamepadRuntimeSnapshot::from(snapshot))
    }

    #[napi]
    pub fn update_sampling_json(&self, sampling_json: String) -> NapiResult<String> {
        let sampling = parse_ts_json::<TsGamepadSamplingConfig>(&sampling_json)?;
        let sampling =
            OhMyGamepadSamplingConfigDto::try_from(sampling).map_err(json_parse_error_to_napi)?;
        let runtime = self.runtime()?;
        let runtime = runtime.as_ref().ok_or_else(closed_binding_error)?;
        runtime.set_sampling(sampling).map_err(map_napi_error)?;
        let snapshot = runtime.snapshot().map_err(map_napi_error)?;
        serialize_ts_json(TsGamepadRuntimeSnapshot::from(snapshot))
    }

    #[napi]
    pub fn rebind_logical_pad_json(&self, binding_json: String) -> NapiResult<String> {
        let binding = parse_ts_json::<TsLogicalPadBinding>(&binding_json)?;
        let binding = LogicalPadBindingDto::try_from(binding).map_err(json_parse_error_to_napi)?;
        let runtime = self.runtime()?;
        let runtime = runtime.as_ref().ok_or_else(closed_binding_error)?;
        runtime
            .rebind_logical_pad(binding)
            .map_err(map_napi_error)?;
        let snapshot = runtime.snapshot().map_err(map_napi_error)?;
        serialize_ts_json(TsGamepadRuntimeSnapshot::from(snapshot))
    }

    #[napi]
    pub fn set_sampling_strategy_json(&self, strategy_json: String) -> NapiResult<String> {
        let strategy = parse_ts_json::<TsGamepadSamplingStrategy>(&strategy_json)?;
        let strategy = MultiControllerSamplingStrategyDto::try_from(strategy)
            .map_err(json_parse_error_to_napi)?;
        let runtime = self.runtime()?;
        let runtime = runtime.as_ref().ok_or_else(closed_binding_error)?;
        runtime
            .set_sampling_strategy(strategy)
            .map_err(map_napi_error)?;
        let snapshot = runtime.snapshot().map_err(map_napi_error)?;
        serialize_ts_json(TsGamepadRuntimeSnapshot::from(snapshot))
    }

    #[napi]
    pub fn set_primary_sampling_device_json(&self, device_id_json: String) -> NapiResult<String> {
        let device_id = parse_ts_json::<Option<String>>(&device_id_json)?;
        let runtime = self.runtime()?;
        let runtime = runtime.as_ref().ok_or_else(closed_binding_error)?;
        runtime
            .set_primary_sampling_device(device_id)
            .map_err(map_napi_error)?;
        let snapshot = runtime.snapshot().map_err(map_napi_error)?;
        serialize_ts_json(TsGamepadRuntimeSnapshot::from(snapshot))
    }

    #[napi]
    pub fn pause_sampling_device_json(&self, device_id_json: String) -> NapiResult<String> {
        let device_id = parse_ts_json::<String>(&device_id_json)?;
        let runtime = self.runtime()?;
        let runtime = runtime.as_ref().ok_or_else(closed_binding_error)?;
        runtime
            .pause_sampling_device(&device_id)
            .map_err(map_napi_error)?;
        let snapshot = runtime.snapshot().map_err(map_napi_error)?;
        serialize_ts_json(TsGamepadRuntimeSnapshot::from(snapshot))
    }

    #[napi]
    pub fn resume_sampling_device_json(&self, device_id_json: String) -> NapiResult<String> {
        let device_id = parse_ts_json::<String>(&device_id_json)?;
        let runtime = self.runtime()?;
        let runtime = runtime.as_ref().ok_or_else(closed_binding_error)?;
        runtime
            .resume_sampling_device(&device_id)
            .map_err(map_napi_error)?;
        let snapshot = runtime.snapshot().map_err(map_napi_error)?;
        serialize_ts_json(TsGamepadRuntimeSnapshot::from(snapshot))
    }

    #[napi]
    pub fn play_rumble_json(&self, request_json: String) -> NapiResult<String> {
        let request = parse_ts_json::<TsGamepadRumbleRequest>(&request_json)?;
        let request =
            OhMyGamepadRumbleRequestDto::try_from(request).map_err(json_parse_error_to_napi)?;
        let runtime = self.runtime()?;
        let runtime = runtime.as_ref().ok_or_else(closed_binding_error)?;
        let result = runtime.play_rumble(request).map_err(map_napi_error)?;
        serialize_ts_json(TsGamepadRumbleResult::from(result))
    }

    #[napi]
    pub fn stop_rumble_json(&self, target_json: String) -> NapiResult<String> {
        let target = parse_ts_json::<TsGamepadRumbleTarget>(&target_json)?;
        let target =
            OhMyGamepadRumbleTargetDto::try_from(target).map_err(json_parse_error_to_napi)?;
        let runtime = self.runtime()?;
        let runtime = runtime.as_ref().ok_or_else(closed_binding_error)?;
        let result = runtime.stop_rumble(target).map_err(map_napi_error)?;
        serialize_ts_json(TsGamepadRumbleResult::from(result))
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
        let snapshot_rx = {
            let runtime = self.runtime()?;
            let runtime = runtime.as_ref().ok_or_else(closed_binding_error)?;
            runtime.subscribe_runtime_snapshot()
        };
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
        let runtime = self.runtime.lock().expect("lock napi runtime").take();
        if let Some(runtime) = runtime {
            runtime.shutdown().map_err(map_napi_error)?;
        }
        Ok(())
    }

    fn runtime(&self) -> NapiResult<std::sync::MutexGuard<'_, Option<OhMyGamepadService>>> {
        self.ensure_initialized()?;
        Ok(self.runtime.lock().expect("lock napi runtime"))
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

fn parse_ts_json<T>(value: &str) -> NapiResult<T>
where
    T: DeserializeOwned,
{
    serde_json::from_str(value).map_err(|error| {
        NapiError::new(
            Status::InvalidArg,
            format!("failed to parse OhMyGamepad JSON payload: {error}"),
        )
    })
}

fn serialize_ts_json<T>(value: T) -> NapiResult<String>
where
    T: Serialize,
{
    serde_json::to_string(&value).map_err(|error| {
        NapiError::new(
            Status::GenericFailure,
            format!("failed to serialize OhMyGamepad JSON payload: {error}"),
        )
    })
}

fn closed_binding_error() -> NapiError {
    NapiError::new(
        Status::GenericFailure,
        "OhMyGamepad native binding has already been shut down".to_owned(),
    )
}

fn map_napi_error(error: ohmygamepad_core::InputRuntimeError) -> NapiError {
    NapiError::new(
        Status::GenericFailure,
        format!("OhMyGamepad native operation failed: {error:?}"),
    )
}

fn json_parse_error_to_napi(error: String) -> NapiError {
    NapiError::new(
        Status::InvalidArg,
        format!("invalid OhMyGamepad payload: {error}"),
    )
}

fn create_string_threadsafe_function(
    callback: JsFunction,
) -> NapiResult<ThreadsafeFunction<String, ErrorStrategy::CalleeHandled>> {
    callback.create_threadsafe_function(
        0,
        |ctx: ThreadSafeCallContext<String>| -> NapiResult<Vec<JsString>> {
            Ok(vec![ctx.env.create_string_from_std(ctx.value)?])
        },
    )
}

fn spawn_runtime_snapshot_subscription(
    snapshot_rx: mpsc::Receiver<ohmygamepad_protocol::OhMyGamepadRuntimeSnapshotDto>,
    snapshot_push: ThreadsafeFunction<String, ErrorStrategy::CalleeHandled>,
    error_push: Option<ThreadsafeFunction<String, ErrorStrategy::CalleeHandled>>,
) -> RuntimeSnapshotSubscription {
    let (stop_tx, stop_rx) = mpsc::channel();
    let join_handle = thread::spawn(move || loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        match snapshot_rx.recv_timeout(Duration::from_millis(
            RUNTIME_SNAPSHOT_STOP_CHECK_INTERVAL_MS,
        )) {
            Ok(snapshot) => {
                let snapshot_json =
                    match serialize_ts_json(TsGamepadRuntimeSnapshot::from(snapshot)) {
                        Ok(snapshot_json) => snapshot_json,
                        Err(error) => {
                            if let Some(error_push) = &error_push {
                                let _ = error_push.call(
                                    Ok(error.to_string()),
                                    ThreadsafeFunctionCallMode::NonBlocking,
                                );
                            }
                            break;
                        }
                    };
                if snapshot_push.call(Ok(snapshot_json), ThreadsafeFunctionCallMode::NonBlocking)
                    != Status::Ok
                {
                    break;
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
