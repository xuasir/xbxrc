use std::collections::VecDeque;
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::thread;

use ohmygamepad_protocol::{
    OhMyGamepadRumbleEffectDto, OhMyGamepadRumbleRequestDto, OhMyGamepadRumbleResultDto,
    OhMyGamepadRumbleTargetDto,
};
use tauri::{AppHandle, Manager};

use crate::error::AppError;
use crate::mods::runtime_trace::RuntimeTraceRecorderRef;
use crate::mods::xbxengine::runtime_state::map_app_error;
use crate::AppState;
use xbxengine::XbxEngineRuntimeError;

enum GamepadRumbleWorkerCommand {
    Submit(OhMyGamepadRumbleRequestDto),
    Clear,
    Shutdown,
}

#[derive(Clone)]
pub(super) struct GamepadRumbleWorkerHandle {
    sender: Sender<GamepadRumbleWorkerCommand>,
}

impl GamepadRumbleWorkerHandle {
    pub(super) fn new(app_handle: AppHandle, runtime_trace: RuntimeTraceRecorderRef) -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("xbxengine-rumble-worker".to_string())
            .spawn(move || {
                let mut pending_requests = VecDeque::new();
                let mut active_targets = Vec::new();
                let mut active_effects = Vec::new();
                let mut shutting_down = false;

                loop {
                    if shutting_down && pending_requests.is_empty() {
                        break;
                    }

                    if let Some(request) = pending_requests.pop_front() {
                        dispatch_request(
                            &app_handle,
                            &runtime_trace,
                            &request,
                            &mut active_targets,
                            &mut active_effects,
                        );
                        drain_incoming_commands(
                            &receiver,
                            &mut pending_requests,
                            &mut active_targets,
                            &mut shutting_down,
                        );
                        continue;
                    }

                    match receiver.recv() {
                        Ok(command) => {
                            apply_command(
                                command,
                                &mut pending_requests,
                                &mut active_targets,
                                &mut shutting_down,
                            );
                        }
                        Err(_) => break,
                    }
                }
            })
            .expect("spawn xbxengine rumble worker");

        Self { sender }
    }

    pub(super) fn submit_request(
        &self,
        request: OhMyGamepadRumbleRequestDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.sender
            .send(GamepadRumbleWorkerCommand::Submit(request))
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRumbleWorkerUnavailable"))
    }

    pub(super) fn clear_pending_requests(&self) -> Result<(), XbxEngineRuntimeError> {
        self.sender
            .send(GamepadRumbleWorkerCommand::Clear)
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRumbleWorkerUnavailable"))
    }

    pub(super) fn shutdown(&self) -> Result<(), XbxEngineRuntimeError> {
        self.sender
            .send(GamepadRumbleWorkerCommand::Shutdown)
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRumbleWorkerUnavailable"))
    }
}

fn drain_incoming_commands(
    receiver: &mpsc::Receiver<GamepadRumbleWorkerCommand>,
    pending_requests: &mut VecDeque<OhMyGamepadRumbleRequestDto>,
    active_targets: &mut Vec<OhMyGamepadRumbleTargetDto>,
    shutting_down: &mut bool,
) {
    loop {
        match receiver.try_recv() {
            Ok(command) => {
                apply_command(
                    command,
                    pending_requests,
                    active_targets,
                    shutting_down,
                );
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                *shutting_down = true;
                break;
            }
        }
    }
}

fn apply_command(
    command: GamepadRumbleWorkerCommand,
    pending_requests: &mut VecDeque<OhMyGamepadRumbleRequestDto>,
    active_targets: &mut Vec<OhMyGamepadRumbleTargetDto>,
    shutting_down: &mut bool,
) {
    match command {
        GamepadRumbleWorkerCommand::Submit(request) => {
            if *shutting_down {
                return;
            }
            upsert_pending_request(pending_requests, request);
        }
        GamepadRumbleWorkerCommand::Clear => {
            clear_pending_requests(pending_requests, active_targets);
        }
        GamepadRumbleWorkerCommand::Shutdown => {
            *shutting_down = true;
            clear_pending_requests(pending_requests, active_targets);
        }
    }
}

fn clear_pending_requests(
    pending_requests: &mut VecDeque<OhMyGamepadRumbleRequestDto>,
    active_targets: &mut Vec<OhMyGamepadRumbleTargetDto>,
) {
    pending_requests.clear();
    for target in active_targets.drain(..) {
        pending_requests.push_back(stop_rumble_request(target));
    }
}

fn upsert_pending_request(
    pending_requests: &mut VecDeque<OhMyGamepadRumbleRequestDto>,
    request: OhMyGamepadRumbleRequestDto,
) {
    if let Some(index) = pending_requests
        .iter()
        .position(|pending| pending.target == request.target)
    {
        pending_requests.remove(index);
    }
    pending_requests.push_back(request);
}

fn dispatch_request(
    app_handle: &AppHandle,
    runtime_trace: &RuntimeTraceRecorderRef,
    request: &OhMyGamepadRumbleRequestDto,
    active_targets: &mut Vec<OhMyGamepadRumbleTargetDto>,
    active_effects: &mut Vec<(OhMyGamepadRumbleTargetDto, OhMyGamepadRumbleEffectDto)>,
) {
    if should_skip_redundant_play(request, active_effects) {
        return;
    }

    let effective_request = if is_silent_rumble_effect(&request.effect) {
        stop_rumble_request(request.target.clone())
    } else {
        request.clone()
    };
    let request_summary = serde_json::to_value(&effective_request).unwrap_or(serde_json::Value::Null);
    let event_name = if is_stop_gamepad_rumble_request(&effective_request.effect) {
        "stopGamepadRumbleResult"
    } else {
        "playGamepadRumbleResult"
    };
    let result = execute_request(app_handle, &effective_request);

    match result {
        Ok(result) => {
            record_gamepad_rumble_result(runtime_trace, event_name, request_summary, &result, None);
            if result.accepted {
                track_active_state(&effective_request, active_targets, active_effects);
            }
        }
        Err(error) => {
            record_gamepad_rumble_error(
                runtime_trace,
                event_name,
                request_summary,
                &error.to_string(),
            );
            log::warn!("[xbxengine][host] gamepad rumble host call failed: {}", error);
        }
    }
}

fn execute_request(
    app_handle: &AppHandle,
    request: &OhMyGamepadRumbleRequestDto,
) -> Result<OhMyGamepadRumbleResultDto, XbxEngineRuntimeError> {
    let state = app_handle
        .try_state::<AppState>()
        .ok_or_else(|| {
            AppError::XbxEngine("AppState unavailable for xbxengine host bridge".to_string())
        })
        .map_err(map_app_error(if is_stop_gamepad_rumble_request(&request.effect) {
            "stopGamepadRumble"
        } else {
            "playGamepadRumble"
        }))?;
    let vibration_config = state.config.get_streaming_config();
    if !vibration_config.vibration {
        return Ok(OhMyGamepadRumbleResultDto {
            accepted: false,
            reason: Some(
                ohmygamepad_protocol::OhMyGamepadRumbleRejectionReasonDto::Unsupported,
            ),
            resolved_device_ids: Vec::new(),
        });
    }
    if is_stop_gamepad_rumble_request(&request.effect) {
        state
            .gamepad
            .stop_rumble(request.target.clone())
            .map_err(|error| map_app_error("stopGamepadRumble")(AppError::Gamepad(error)))
    } else {
        state
            .gamepad
            .play_rumble(request.clone())
            .map_err(|error| map_app_error("playGamepadRumble")(AppError::Gamepad(error)))
    }
}

fn track_active_target(
    request: &OhMyGamepadRumbleRequestDto,
    active_targets: &mut Vec<OhMyGamepadRumbleTargetDto>,
) {
    if is_stop_gamepad_rumble_request(&request.effect) {
        active_targets.retain(|target| *target != request.target);
        return;
    }
    if !active_targets.iter().any(|target| *target == request.target) {
        active_targets.push(request.target.clone());
    }
}

fn track_active_state(
    request: &OhMyGamepadRumbleRequestDto,
    active_targets: &mut Vec<OhMyGamepadRumbleTargetDto>,
    active_effects: &mut Vec<(OhMyGamepadRumbleTargetDto, OhMyGamepadRumbleEffectDto)>,
) {
    track_active_target(request, active_targets);
    if is_stop_gamepad_rumble_request(&request.effect) {
        active_effects.retain(|(target, _)| *target != request.target);
        return;
    }
    if let Some((_, effect)) = active_effects
        .iter_mut()
        .find(|(target, _)| *target == request.target)
    {
        *effect = request.effect.clone();
        return;
    }
    active_effects.push((request.target.clone(), request.effect.clone()));
}

fn should_skip_redundant_play(
    request: &OhMyGamepadRumbleRequestDto,
    active_effects: &[(OhMyGamepadRumbleTargetDto, OhMyGamepadRumbleEffectDto)],
) -> bool {
    if is_stop_gamepad_rumble_request(&request.effect) || is_silent_rumble_effect(&request.effect) {
        return false;
    }
    active_effects
        .iter()
        .find(|(target, _)| *target == request.target)
        .is_some_and(|(_, effect)| effects_equivalent(effect, &request.effect))
}

fn effects_equivalent(
    left: &OhMyGamepadRumbleEffectDto,
    right: &OhMyGamepadRumbleEffectDto,
) -> bool {
    left.start_delay_ms == right.start_delay_ms
        && left.duration_ms == right.duration_ms
        && left.repeat == right.repeat
        && left.strong_magnitude == right.strong_magnitude
        && left.weak_magnitude == right.weak_magnitude
        && left.left_trigger == right.left_trigger
        && left.right_trigger == right.right_trigger
}

fn record_gamepad_rumble_result(
    runtime_trace: &RuntimeTraceRecorderRef,
    event_name: &'static str,
    request_summary: serde_json::Value,
    result: &OhMyGamepadRumbleResultDto,
    error: Option<&str>,
) {
    runtime_trace.record_event(
        "xbxengine-host",
        event_name,
        None,
        serde_json::json!({
            "request": request_summary,
            "accepted": result.accepted,
            "reason": result.reason,
            "resolvedDeviceIds": result.resolved_device_ids,
            "error": error,
        }),
    );
    if !result.accepted {
        log::warn!(
            "[xbxengine][host] gamepad rumble rejected reason={:?} resolved_device_ids={:?}",
            result.reason,
            result.resolved_device_ids
        );
    }
}

fn record_gamepad_rumble_error(
    runtime_trace: &RuntimeTraceRecorderRef,
    event_name: &'static str,
    request_summary: serde_json::Value,
    error: &str,
) {
    runtime_trace.record_event(
        "xbxengine-host",
        event_name,
        None,
        serde_json::json!({
            "request": request_summary,
            "accepted": false,
            "error": error,
        }),
    );
}

fn stop_rumble_request(target: OhMyGamepadRumbleTargetDto) -> OhMyGamepadRumbleRequestDto {
    OhMyGamepadRumbleRequestDto {
        target,
        effect: OhMyGamepadRumbleEffectDto {
            duration_ms: 0,
            start_delay_ms: 0,
            strong_magnitude: 0.0,
            weak_magnitude: 0.0,
            left_trigger: 0.0,
            right_trigger: 0.0,
            repeat: 0,
        },
    }
}

fn is_stop_gamepad_rumble_request(effect: &OhMyGamepadRumbleEffectDto) -> bool {
    effect.duration_ms == 0
        && effect.start_delay_ms == 0
        && effect.strong_magnitude <= 0.0
        && effect.weak_magnitude <= 0.0
        && effect.left_trigger <= 0.0
        && effect.right_trigger <= 0.0
        && effect.repeat == 0
}

fn is_silent_rumble_effect(effect: &OhMyGamepadRumbleEffectDto) -> bool {
    effect.strong_magnitude <= 0.0
        && effect.weak_magnitude <= 0.0
        && effect.left_trigger <= 0.0
        && effect.right_trigger <= 0.0
}
