use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tauri::{AppHandle, Manager};
use windows::core::{Interface, PCSTR};
use windows::Win32::Foundation::{HANDLE, HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_SRV_DIMENSION_TEXTURE2DARRAY,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Buffer, ID3D11DepthStencilView, ID3D11Device, ID3D11Device3,
    ID3D11DeviceContext, ID3D11PixelShader, ID3D11RenderTargetView, ID3D11SamplerState,
    ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader, D3D11_BIND_CONSTANT_BUFFER,
    D3D11_COMPARISON_NEVER, D3D11_CPU_ACCESS_FLAG, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_SAMPLER_DESC, D3D11_SDK_VERSION,
    D3D11_SHADER_RESOURCE_VIEW_DESC1, D3D11_SHADER_RESOURCE_VIEW_DESC1_0, D3D11_SUBRESOURCE_DATA,
    D3D11_TEX2D_ARRAY_SRV1, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_USAGE_DEFAULT, D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8_UNORM,
    DXGI_FORMAT_R8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory2, IDXGIOutput, IDXGISwapChain1,
    DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use xbxengine::{
    MacOsVideoChromaLocation, MacOsVideoColorMatrix, MacOsVideoColorRange,
    WindowsD3d11TextureDescriptor, XbxEngineRenderFrame, XbxEngineRenderPixelData,
};

use crate::mods::runtime_trace::RuntimeTraceRecorderRef;

use super::presenters::{NativeVideoPresenter, NativeVideoPresenterKind};
use super::scheduling::{
    submitted_frame_is_stale_for_host_mailbox, HostCadenceTelemetry, ScheduledFrameSlot,
    ScheduledFrameSubmitOutcome,
};
use super::{
    clear_host_present_tick_dispatch, finish_host_present_tick_guard_and_maybe_rerun, now_ms_f64,
    record_host_frame_presented, record_host_mailbox_idle, record_host_mailbox_rejected_stale,
    record_host_mailbox_retained_displayed, record_host_mailbox_retained_displayed_stale,
    record_host_mailbox_take_decision, record_native_video_timing_event_lazy,
    request_host_present_tick_dispatch, HostFramePresentedFacts, HostPresentTickGuard,
    NativeVideoDisplayState, NativeVideoViewportState, HOST_TIMING_QUEUE_WARN_MS,
    HOST_TIMING_TICK_WARN_MS,
};

use super::presenters::apply_host_mailbox_viewport_diagnostics;
use super::scheduling::ScheduledFrameTakeOutcome;

const D3D11_NATIVE_PIPELINE: &str = "d3d11-native";
const DXGI_FORMAT_NV12_VALUE: u32 = 103;

const D3D11_VERTEX_SHADER: &[u8] = br#"
struct VertexOutput {
  float4 position : SV_POSITION;
  float2 uv : TEXCOORD0;
};

VertexOutput vs_main(uint vertex_id : SV_VertexID) {
  float2 positions[3] = {
    float2(-1.0, -1.0),
    float2(3.0, -1.0),
    float2(-1.0, 3.0)
  };
  float2 uvs[3] = {
    float2(0.0, 1.0),
    float2(2.0, 1.0),
    float2(0.0, -1.0)
  };
  VertexOutput output;
  output.position = float4(positions[vertex_id], 0.0, 1.0);
  output.uv = uvs[vertex_id];
  return output;
}
"#;

const D3D11_PIXEL_SHADER: &[u8] = br#"
Texture2D y_texture : register(t0);
Texture2D uv_texture : register(t1);
SamplerState frame_sampler : register(s0);

cbuffer Nv12Params : register(b0) {
  float4 row0;
  float4 row1;
  float4 row2;
  float4 uv_offset;
};

float4 ps_main(float4 position : SV_POSITION, float2 uv : TEXCOORD0) : SV_TARGET {
  float y = y_texture.SampleLevel(frame_sampler, uv, 0.0).r;
  float2 uv_sample = uv_texture.SampleLevel(frame_sampler, uv + uv_offset.xy, 0.0).rg;
  float4 yuv = float4(y, uv_sample.x, uv_sample.y, 1.0);
  float3 rgb = float3(dot(row0, yuv), dot(row1, yuv), dot(row2, yuv));
  return float4(saturate(rgb), 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct Nv12ColorParams {
    row0: [f32; 4],
    row1: [f32; 4],
    row2: [f32; 4],
    uv_offset: [f32; 4],
}

#[derive(Default)]
struct WindowsD3d11State {
    renderer: Option<D3d11FrameRenderer>,
    latest_frame: Option<XbxEngineRenderFrame>,
    view_generation: u64,
    latest_view_created_at_ms: Option<f64>,
    last_surface_size: Option<(u32, u32)>,
    descriptor_upload_mode: Option<String>,
    descriptor_cpu_upload_count_total: u64,
    render_loop_started: bool,
    init_failed_logged: bool,
}

unsafe impl Send for WindowsD3d11State {}

pub(super) struct WindowsD3d11Presenter {
    viewport_id: String,
    window_label: String,
    surface_id: Option<String>,
    app_handle: AppHandle,
    renderer_state: Arc<Mutex<WindowsD3d11State>>,
    frame_slot: Arc<Mutex<ScheduledFrameSlot>>,
    telemetry: Arc<Mutex<HostCadenceTelemetry>>,
    render_loop_stop: Arc<AtomicBool>,
    render_loop_pending: Arc<AtomicBool>,
    render_loop_rerun_requested: Arc<AtomicBool>,
    runtime_trace: Option<RuntimeTraceRecorderRef>,
    display_state: NativeVideoDisplayState,
}

impl WindowsD3d11Presenter {
    pub(super) fn new(
        viewport_id: &str,
        window_label: &str,
        app_handle: AppHandle,
        runtime_trace: Option<RuntimeTraceRecorderRef>,
    ) -> Self {
        Self {
            viewport_id: viewport_id.to_string(),
            window_label: window_label.to_string(),
            surface_id: None,
            app_handle,
            renderer_state: Arc::new(Mutex::new(WindowsD3d11State::default())),
            frame_slot: Arc::new(Mutex::new(ScheduledFrameSlot::default())),
            telemetry: Arc::new(Mutex::new(HostCadenceTelemetry::default())),
            render_loop_stop: Arc::new(AtomicBool::new(false)),
            render_loop_pending: Arc::new(AtomicBool::new(false)),
            render_loop_rerun_requested: Arc::new(AtomicBool::new(false)),
            runtime_trace,
            display_state: NativeVideoDisplayState::default(),
        }
    }

    fn ensure_render_loop(&mut self) -> bool {
        let Ok(mut state) = self.renderer_state.try_lock() else {
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                D3D11_NATIVE_PIPELINE,
                "hostMailboxUpdateFailed",
                &self.viewport_id,
                &self.window_label,
                || serde_json::json!({ "reason": "rendererStateBusy" }),
            );
            return false;
        };
        if state.render_loop_started {
            return true;
        }
        state.render_loop_started = true;
        self.render_loop_stop.store(false, Ordering::Relaxed);

        let viewport_id = self.viewport_id.clone();
        let window_label = self.window_label.clone();
        let app_handle = self.app_handle.clone();
        let renderer_state = self.renderer_state.clone();
        let frame_slot = self.frame_slot.clone();
        let telemetry = self.telemetry.clone();
        let render_loop_stop = self.render_loop_stop.clone();
        let render_loop_pending = self.render_loop_pending.clone();
        let render_loop_rerun_requested = self.render_loop_rerun_requested.clone();
        let runtime_trace = self.runtime_trace.clone();
        thread::Builder::new()
            .name(format!("XbxWindowsD3d11RenderLoop-{viewport_id}"))
            .spawn(move || {
                while !render_loop_stop.load(Ordering::Relaxed) {
                    let tick = telemetry
                        .lock()
                        .map(|state| state.present_tick_sleep_duration())
                        .unwrap_or(Duration::from_millis(16));
                    thread::sleep(tick);
                    if !request_host_present_tick_dispatch(
                        &render_loop_pending,
                        &render_loop_rerun_requested,
                    ) {
                        continue;
                    }
                    run_windows_d3d11_render_tick(
                        &app_handle,
                        &window_label,
                        &viewport_id,
                        &renderer_state,
                        &frame_slot,
                        &telemetry,
                        &render_loop_pending,
                        &render_loop_rerun_requested,
                        Some(now_ms_f64()),
                        runtime_trace.clone(),
                    );
                }
            })
            .expect("Failed to spawn Windows D3D11 render loop");
        true
    }

    fn request_immediate_render_tick(&self) {
        if !request_host_present_tick_dispatch(
            &self.render_loop_pending,
            &self.render_loop_rerun_requested,
        ) {
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                D3D11_NATIVE_PIPELINE,
                "present_tick_dispatch_coalesced",
                &self.viewport_id,
                &self.window_label,
                || {
                    serde_json::json!({
                        "source": "immediateSubmit",
                    })
                },
            );
            return;
        }
        record_native_video_timing_event_lazy(
            self.runtime_trace.as_ref(),
            D3D11_NATIVE_PIPELINE,
            "present_tick_immediate_requested",
            &self.viewport_id,
            &self.window_label,
            || {
                serde_json::json!({
                    "source": "immediateSubmit",
                    "target": "renderTick",
                })
            },
        );
        run_windows_d3d11_render_tick(
            &self.app_handle,
            &self.window_label,
            &self.viewport_id,
            &self.renderer_state,
            &self.frame_slot,
            &self.telemetry,
            &self.render_loop_pending,
            &self.render_loop_rerun_requested,
            Some(now_ms_f64()),
            self.runtime_trace.clone(),
        );
    }

    fn should_drop_submitted_frame(&self, frame: &XbxEngineRenderFrame, now_ms: f64) -> bool {
        let Ok(telemetry) = self.telemetry.try_lock() else {
            return false;
        };
        submitted_frame_is_stale_for_host_mailbox(&telemetry, frame, now_ms)
    }

    fn present_d3d11_descriptor(
        &mut self,
        surface_id: Option<&str>,
        frame: &XbxEngineRenderFrame,
    ) -> bool {
        self.surface_id = surface_id
            .map(str::to_string)
            .or_else(|| self.surface_id.clone());
        if !self.ensure_render_loop() {
            return false;
        }
        let now_ms = now_ms_f64();
        if self.should_drop_submitted_frame(frame, now_ms) {
            if let Ok(mut telemetry) = self.telemetry.try_lock() {
                telemetry.record_stale_frame_drop(frame, now_ms, "submittedFrameStale", 0);
            }
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                D3D11_NATIVE_PIPELINE,
                "hostMailboxRejected",
                &self.viewport_id,
                &self.window_label,
                || {
                    serde_json::json!({
                        "outcome": "stale",
                        "frameSeq": frame.frame_seq,
                        "frameAgeMs": (now_ms - frame.rendered_at_ms).max(0.0),
                    })
                },
            );
            return false;
        }
        let Ok(mut telemetry) = self.telemetry.try_lock() else {
            self.record_mailbox_update_failed("telemetryLockFailed", frame.frame_seq);
            return false;
        };
        let submit_gap_ms = telemetry.submit_gap_ms(now_ms);
        let no_pending_streak_before_submit = telemetry.no_pending_streak;
        let should_warn_submit_gap =
            submit_gap_ms.is_some_and(|gap_ms| telemetry.should_warn_submit_gap(gap_ms));
        let telemetry_diag = telemetry.diagnostics_snapshot();
        let Ok(mut frame_slot) = self.frame_slot.try_lock() else {
            self.record_mailbox_update_failed("frameSlotLockFailed", frame.frame_seq);
            return false;
        };
        match frame_slot.submit_frame(frame, now_ms, &mut telemetry) {
            ScheduledFrameSubmitOutcome::Accepted {
                frame_seq,
                overwrote_pending,
                replaced_frame_seq,
                frame_age_ms,
                frame_age_budget_ms,
            } => {
                let slot_diag = frame_slot.diagnostics_snapshot();
                record_native_video_timing_event_lazy(
                    self.runtime_trace.as_ref(),
                    D3D11_NATIVE_PIPELINE,
                    "hostMailboxAccepted",
                    &self.viewport_id,
                    &self.window_label,
                    || {
                        serde_json::json!({
                            "outcome": "accepted",
                            "frameSeq": frame_seq,
                            "frameRtpTimestamp": frame.rtp_timestamp,
                            "frameRecoveryEpoch": frame.recovery_epoch_tag,
                            "frameRecoveryOwnerRtpTimestamp": frame.recovery_owner_rtp_timestamp,
                            "frameRecoveryDisposition": frame.frame_recovery_disposition,
                            "framePresentationValueRole": frame.presentation_value_role,
                            "frameIsKeyframe": frame.is_keyframe,
                            "frameAgeMs": frame_age_ms,
                            "frameAgeBudgetMs": frame_age_budget_ms,
                            "submitGapMs": submit_gap_ms,
                            "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                            "overwrotePending": overwrote_pending,
                            "replacedFrameSeq": replaced_frame_seq,
                            "displayedFrameSeq": slot_diag.displayed_frame_seq,
                            "pendingFrameSeq": slot_diag.pending_frame_seqs.first().copied(),
                            "hasPendingFrame": slot_diag.pending_queue_depth > 0,
                            "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                            "queueDepth": slot_diag.queue_depth,
                            "pendingQueueDepth": slot_diag.pending_queue_depth,
                            "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                            "hostFramePresentEpoch": telemetry_diag.present_epoch,
                            "hostCadencePhase": telemetry_diag.cadence_phase.as_str(),
                        })
                    },
                );
                if let Some(gap_ms) = submit_gap_ms.filter(|_| should_warn_submit_gap) {
                    record_native_video_timing_event_lazy(
                        self.runtime_trace.as_ref(),
                        D3D11_NATIVE_PIPELINE,
                        "hostMailboxSubmitGap",
                        &self.viewport_id,
                        &self.window_label,
                        || {
                            serde_json::json!({
                                "frameSeq": frame_seq,
                                "submitGapMs": gap_ms,
                                "frameAgeMs": frame_age_ms,
                                "frameAgeBudgetMs": frame_age_budget_ms,
                                "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                                "overwrotePending": overwrote_pending,
                                "replacedFrameSeq": replaced_frame_seq,
                                "displayedFrameSeq": slot_diag.displayed_frame_seq,
                                "pendingFrameSeq": slot_diag.pending_frame_seqs.first().copied(),
                                "hasPendingFrame": slot_diag.pending_queue_depth > 0,
                                "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                                "queueDepth": slot_diag.queue_depth,
                                "pendingQueueDepth": slot_diag.pending_queue_depth,
                                "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                                "hostFramePresentEpoch": telemetry_diag.present_epoch,
                            })
                        },
                    );
                }
                drop(frame_slot);
                drop(telemetry);
                self.request_immediate_render_tick();
                true
            }
            ScheduledFrameSubmitOutcome::DroppedStale {
                frame_seq,
                frame_age_ms,
                frame_age_budget_ms,
            } => {
                let slot_diag = frame_slot.diagnostics_snapshot();
                record_native_video_timing_event_lazy(
                    self.runtime_trace.as_ref(),
                    D3D11_NATIVE_PIPELINE,
                    "hostMailboxRejected",
                    &self.viewport_id,
                    &self.window_label,
                    || {
                        serde_json::json!({
                            "outcome": "stale",
                            "frameSeq": frame_seq,
                            "frameRtpTimestamp": frame.rtp_timestamp,
                            "frameRecoveryEpoch": frame.recovery_epoch_tag,
                            "frameRecoveryOwnerRtpTimestamp": frame.recovery_owner_rtp_timestamp,
                            "frameRecoveryDisposition": frame.frame_recovery_disposition,
                            "framePresentationValueRole": frame.presentation_value_role,
                            "frameIsKeyframe": frame.is_keyframe,
                            "frameAgeMs": frame_age_ms,
                            "frameAgeBudgetMs": frame_age_budget_ms,
                            "submitGapMs": submit_gap_ms,
                            "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                            "displayedFrameSeq": slot_diag.displayed_frame_seq,
                            "pendingFrameSeq": slot_diag.pending_frame_seqs.first().copied(),
                            "hasPendingFrame": slot_diag.pending_queue_depth > 0,
                            "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                            "queueDepth": slot_diag.queue_depth,
                            "pendingQueueDepth": slot_diag.pending_queue_depth,
                            "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                            "hostFramePresentEpoch": telemetry_diag.present_epoch,
                            "hostCadencePhase": telemetry_diag.cadence_phase.as_str(),
                        })
                    },
                );
                false
            }
            ScheduledFrameSubmitOutcome::RejectedAlreadyPresented {
                frame_seq,
                last_presented_frame_seq,
            } => {
                let slot_diag = frame_slot.diagnostics_snapshot();
                record_native_video_timing_event_lazy(
                    self.runtime_trace.as_ref(),
                    D3D11_NATIVE_PIPELINE,
                    "hostMailboxRejected",
                    &self.viewport_id,
                    &self.window_label,
                    || {
                        serde_json::json!({
                            "outcome": "already_presented",
                            "frameSeq": frame_seq,
                            "frameRtpTimestamp": frame.rtp_timestamp,
                            "frameRecoveryEpoch": frame.recovery_epoch_tag,
                            "frameRecoveryOwnerRtpTimestamp": frame.recovery_owner_rtp_timestamp,
                            "frameRecoveryDisposition": frame.frame_recovery_disposition,
                            "framePresentationValueRole": frame.presentation_value_role,
                            "frameIsKeyframe": frame.is_keyframe,
                            "lastPresentedFrameSeq": last_presented_frame_seq,
                            "submitGapMs": submit_gap_ms,
                            "noPendingStreakBeforeSubmit": no_pending_streak_before_submit,
                            "displayedFrameSeq": slot_diag.displayed_frame_seq,
                            "pendingFrameSeq": slot_diag.pending_frame_seqs.first().copied(),
                            "hasPendingFrame": slot_diag.pending_queue_depth > 0,
                            "pendingFrameSeqs": slot_diag.pending_frame_seqs,
                            "queueDepth": slot_diag.queue_depth,
                            "pendingQueueDepth": slot_diag.pending_queue_depth,
                            "hostDisplayTickEpoch": telemetry_diag.display_tick_epoch,
                            "hostFramePresentEpoch": telemetry_diag.present_epoch,
                            "hostCadencePhase": telemetry_diag.cadence_phase.as_str(),
                        })
                    },
                );
                false
            }
        }
    }

    fn record_mailbox_update_failed(&self, reason: &str, frame_seq: u64) {
        record_native_video_timing_event_lazy(
            self.runtime_trace.as_ref(),
            D3D11_NATIVE_PIPELINE,
            "hostMailboxUpdateFailed",
            &self.viewport_id,
            &self.window_label,
            || serde_json::json!({ "reason": reason, "frameSeq": frame_seq }),
        );
    }
}

impl NativeVideoPresenter for WindowsD3d11Presenter {
    fn kind(&self) -> NativeVideoPresenterKind {
        NativeVideoPresenterKind::PlatformNative
    }

    fn attach(&mut self, surface_id: Option<&str>) {
        self.begin_media_epoch();
        self.surface_id = surface_id.map(str::to_string);
        let _ = self.ensure_render_loop();
    }

    fn begin_media_epoch(&mut self) {
        if let Ok(mut state) = self.renderer_state.lock() {
            state.latest_frame = None;
            state.last_surface_size = None;
            state.descriptor_upload_mode = None;
            state.descriptor_cpu_upload_count_total = 0;
        }
        if let Ok(mut frame_slot) = self.frame_slot.lock() {
            frame_slot.begin_media_epoch();
        }
        if let Ok(mut telemetry) = self.telemetry.lock() {
            telemetry.reset_frame_slot();
        }
        clear_host_present_tick_dispatch(
            &self.render_loop_pending,
            &self.render_loop_rerun_requested,
        );
    }

    fn reset_mailbox_for_host_stall_recovery(&mut self) -> bool {
        self.begin_media_epoch();
        self.render_loop_stop.store(false, Ordering::Relaxed);
        let _ = self.ensure_render_loop();
        true
    }

    fn apply_display_state(&mut self, state: &NativeVideoDisplayState) {
        self.display_state = state.clone();
    }

    fn present(&mut self, surface_id: Option<&str>, frame: &XbxEngineRenderFrame) -> bool {
        let XbxEngineRenderPixelData::Descriptor { handle } = &frame.pixel_data else {
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                D3D11_NATIVE_PIPELINE,
                "hostMailboxRejected",
                &self.viewport_id,
                &self.window_label,
                || {
                    serde_json::json!({
                        "outcome": "unsupportedSurface",
                        "reason": "cpuSurfaceRejectedForZeroCopyPresenter",
                        "frameSeq": frame.frame_seq,
                    })
                },
            );
            return false;
        };
        let Some(descriptor) = handle.downcast_ref::<WindowsD3d11TextureDescriptor>() else {
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                D3D11_NATIVE_PIPELINE,
                "hostMailboxRejected",
                &self.viewport_id,
                &self.window_label,
                || {
                    serde_json::json!({
                        "outcome": "unsupportedDescriptor",
                        "reason": "expectedWindowsD3d11TextureDescriptor",
                        "frameSeq": frame.frame_seq,
                    })
                },
            );
            return false;
        };
        if descriptor.dxgi_format != DXGI_FORMAT_NV12_VALUE {
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                D3D11_NATIVE_PIPELINE,
                "hostMailboxRejected",
                &self.viewport_id,
                &self.window_label,
                || {
                    serde_json::json!({
                        "outcome": "unsupportedDescriptor",
                        "reason": "unsupportedDxgiFormat",
                        "dxgiFormat": descriptor.dxgi_format,
                        "frameSeq": frame.frame_seq,
                    })
                },
            );
            return false;
        }
        if descriptor.shared_handle.is_null() {
            record_native_video_timing_event_lazy(
                self.runtime_trace.as_ref(),
                D3D11_NATIVE_PIPELINE,
                "hostMailboxRejected",
                &self.viewport_id,
                &self.window_label,
                || {
                    serde_json::json!({
                        "outcome": "unsupportedDescriptor",
                        "reason": "missingSharedHandle",
                        "frameSeq": frame.frame_seq,
                    })
                },
            );
            return false;
        }
        self.present_d3d11_descriptor(surface_id, frame)
    }

    fn detach(&mut self) {
        self.surface_id = None;
        self.render_loop_stop.store(true, Ordering::Relaxed);
        if let Ok(mut frame_slot) = self.frame_slot.lock() {
            frame_slot.reset();
        }
        if let Ok(mut telemetry) = self.telemetry.lock() {
            telemetry.reset_frame_slot();
        }
        if let Ok(mut state) = self.renderer_state.lock() {
            state.renderer = None;
            state.latest_frame = None;
            state.last_surface_size = None;
            state.descriptor_upload_mode = None;
            state.descriptor_cpu_upload_count_total = 0;
            state.render_loop_started = false;
        }
    }

    fn apply_viewport_diagnostics(&self, viewport: &mut NativeVideoViewportState) {
        let Ok(telemetry) = self.telemetry.lock() else {
            return;
        };
        let slot_diag = self
            .frame_slot
            .lock()
            .ok()
            .map(|frame_slot| frame_slot.diagnostics_snapshot());
        let renderer_state = self.renderer_state.lock().ok();
        apply_host_mailbox_viewport_diagnostics(
            viewport,
            &telemetry,
            slot_diag.as_ref(),
            renderer_state
                .as_ref()
                .and_then(|state| state.latest_view_created_at_ms),
            renderer_state
                .as_ref()
                .and_then(|state| state.descriptor_upload_mode.clone()),
            0,
            renderer_state
                .as_ref()
                .map(|state| state.descriptor_cpu_upload_count_total)
                .unwrap_or_default(),
        );
    }

    fn take_pending_frame_drops(&mut self) -> Vec<xbxengine::XbxEngineHostVideoFrameDropEvent> {
        self.telemetry
            .lock()
            .map(|mut telemetry| telemetry.take_pending_frame_drops())
            .unwrap_or_default()
    }
}

fn run_windows_d3d11_render_tick(
    app_handle: &AppHandle,
    window_label: &str,
    viewport_id: &str,
    renderer_state: &Arc<Mutex<WindowsD3d11State>>,
    frame_slot: &Arc<Mutex<ScheduledFrameSlot>>,
    telemetry: &Arc<Mutex<HostCadenceTelemetry>>,
    render_loop_pending: &Arc<AtomicBool>,
    render_loop_rerun_requested: &Arc<AtomicBool>,
    dispatch_requested_at_ms: Option<f64>,
    runtime_trace: Option<RuntimeTraceRecorderRef>,
) {
    let tick_started_at_ms = now_ms_f64();
    let mut tick_dispatch_guard = HostPresentTickGuard::new(
        render_loop_pending.clone(),
        render_loop_rerun_requested.clone(),
    );
    if let Some(dispatch_ms) = dispatch_requested_at_ms {
        let queue_delay_ms = (tick_started_at_ms - dispatch_ms).max(0.0);
        if queue_delay_ms >= HOST_TIMING_QUEUE_WARN_MS {
            record_native_video_timing_event_lazy(
                runtime_trace.as_ref(),
                D3D11_NATIVE_PIPELINE,
                "run_on_main_thread_delay",
                viewport_id,
                window_label,
                || serde_json::json!({ "queueDelayMs": queue_delay_ms }),
            );
        }
    }

    let run_tick = || {
        let Some(window) = app_handle.get_window(window_label) else {
            return;
        };
        let Ok(surface_size) = window.inner_size() else {
            return;
        };
        let Ok(hwnd) = extract_hwnd(&window) else {
            return;
        };
        let surface_width = surface_size.width.max(1);
        let surface_height = surface_size.height.max(1);

        let Ok(mut state) = renderer_state.lock() else {
            return;
        };
        let view_generation_before_tick = state.view_generation;
        let size_changed = state.last_surface_size != Some((surface_width, surface_height));
        if state.renderer.is_none() {
            match D3d11FrameRenderer::new(hwnd, surface_width, surface_height) {
                Ok(renderer) => {
                    state.renderer = Some(renderer);
                    state.view_generation = state.view_generation.saturating_add(1);
                    state.latest_view_created_at_ms = Some(now_ms_f64());
                    state.last_surface_size = Some((surface_width, surface_height));
                    state.descriptor_upload_mode = Some(D3D11_NATIVE_PIPELINE.to_string());
                    state.descriptor_cpu_upload_count_total = 0;
                }
                Err(error) => {
                    if !state.init_failed_logged {
                        state.init_failed_logged = true;
                        log::warn!(
                            "[native_video][windows][d3d11] failed to create renderer for viewport={} window={} error={}",
                            viewport_id,
                            window_label,
                            error
                        );
                    }
                    record_native_video_timing_event_lazy(
                        runtime_trace.as_ref(),
                        D3D11_NATIVE_PIPELINE,
                        "present_tick_failed",
                        viewport_id,
                        window_label,
                        || serde_json::json!({ "reason": "rendererInitFailed", "error": error }),
                    );
                    return;
                }
            }
        }

        let view_generation_changed = state.view_generation != view_generation_before_tick;
        let now_ms = now_ms_f64();
        let Some(take_result) =
            take_d3d11_scheduled_frame(frame_slot, telemetry, view_generation_changed, now_ms)
        else {
            return;
        };
        if size_changed {
            state.last_surface_size = Some((surface_width, surface_height));
        }
        let Some(renderer) = state.renderer.as_mut() else {
            return;
        };
        if size_changed {
            if let Err(error) = renderer.update_surface_size(surface_width, surface_height) {
                record_native_video_timing_event_lazy(
                    runtime_trace.as_ref(),
                    D3D11_NATIVE_PIPELINE,
                    "present_tick_failed",
                    viewport_id,
                    window_label,
                    || serde_json::json!({ "reason": "resizeFailed", "error": error }),
                );
                state.renderer = None;
                return;
            }
        }
        let has_cached_frame = state.latest_frame.is_some();
        let presented_frame = process_d3d11_render_take_outcome(
            renderer,
            take_result.outcome,
            has_cached_frame,
            size_changed,
            telemetry,
            runtime_trace.as_ref(),
            viewport_id,
            window_label,
            &take_result.slot_diag,
            &take_result.telemetry_diag,
            now_ms,
        );
        if let Some(frame) = presented_frame {
            state.latest_frame = Some(frame);
            state.descriptor_upload_mode = Some(D3D11_NATIVE_PIPELINE.to_string());
            state.descriptor_cpu_upload_count_total = 0;
        }
        let tick_total_ms = (now_ms_f64() - tick_started_at_ms).max(0.0);
        if tick_total_ms >= HOST_TIMING_TICK_WARN_MS {
            record_native_video_timing_event_lazy(
                runtime_trace.as_ref(),
                D3D11_NATIVE_PIPELINE,
                "tick_total",
                viewport_id,
                window_label,
                || serde_json::json!({ "totalMs": tick_total_ms }),
            );
        }
    };

    run_tick();
    finish_host_present_tick_guard_and_maybe_rerun(&mut tick_dispatch_guard, || {
        run_windows_d3d11_render_tick(
            app_handle,
            window_label,
            viewport_id,
            renderer_state,
            frame_slot,
            telemetry,
            render_loop_pending,
            render_loop_rerun_requested,
            dispatch_requested_at_ms,
            runtime_trace,
        );
    });
}

struct D3d11ScheduledFrameTake {
    outcome: ScheduledFrameTakeOutcome,
    slot_diag: super::scheduling::ScheduledFrameSlotDiagnostics,
    telemetry_diag: super::scheduling::HostCadenceTelemetryDiagnostics,
}

fn take_d3d11_scheduled_frame(
    frame_slot: &Arc<Mutex<ScheduledFrameSlot>>,
    telemetry: &Arc<Mutex<HostCadenceTelemetry>>,
    view_generation_changed: bool,
    now_ms: f64,
) -> Option<D3d11ScheduledFrameTake> {
    let Ok(mut telemetry_state) = telemetry.lock() else {
        return None;
    };
    telemetry_state.record_display_tick(now_ms);
    let Ok(mut frame_slot_state) = frame_slot.lock() else {
        return None;
    };
    if view_generation_changed {
        frame_slot_state.begin_view_epoch();
    }
    let outcome = frame_slot_state.take_ready_frame(now_ms, &mut telemetry_state);
    let slot_diag = frame_slot_state.diagnostics_snapshot();
    let telemetry_diag = telemetry_state.diagnostics_snapshot();
    Some(D3d11ScheduledFrameTake {
        outcome,
        slot_diag,
        telemetry_diag,
    })
}

fn process_d3d11_render_take_outcome(
    renderer: &mut D3d11FrameRenderer,
    take_outcome: ScheduledFrameTakeOutcome,
    has_cached_frame: bool,
    size_changed: bool,
    telemetry: &Arc<Mutex<HostCadenceTelemetry>>,
    runtime_trace: Option<&RuntimeTraceRecorderRef>,
    viewport_id: &str,
    window_label: &str,
    take_slot_diag: &super::scheduling::ScheduledFrameSlotDiagnostics,
    take_telemetry_diag: &super::scheduling::HostCadenceTelemetryDiagnostics,
    now_ms: f64,
) -> Option<XbxEngineRenderFrame> {
    record_host_mailbox_take_decision(
        runtime_trace,
        D3D11_NATIVE_PIPELINE,
        viewport_id,
        window_label,
        &take_outcome,
        take_slot_diag,
        take_telemetry_diag,
    );
    match take_outcome {
        ScheduledFrameTakeOutcome::Ready(frame) => {
            let presented_frame = frame.clone();
            match renderer.render_frame(&frame) {
                Ok(()) => {
                    if let Ok(mut telemetry) = telemetry.lock() {
                        telemetry.record_present(now_ms);
                    }
                    let telemetry_diag = telemetry
                        .lock()
                        .ok()
                        .map(|telemetry_state| telemetry_state.diagnostics_snapshot());
                    record_host_frame_presented(
                        runtime_trace,
                        D3D11_NATIVE_PIPELINE,
                        viewport_id,
                        window_label,
                        HostFramePresentedFacts::from_render_frame(&presented_frame, "present"),
                        Some(take_slot_diag),
                        telemetry_diag.as_ref(),
                        now_ms,
                    );
                    Some(frame)
                }
                Err(error) => {
                    log::warn!(
                        "[native_video][windows][d3d11] render failed for viewport={} window={} error={}",
                        viewport_id,
                        window_label,
                        error
                    );
                    record_native_video_timing_event_lazy(
                        runtime_trace,
                        D3D11_NATIVE_PIPELINE,
                        "present_tick_failed",
                        viewport_id,
                        window_label,
                        || {
                            serde_json::json!({
                                "reason": "renderFailed",
                                "error": error,
                                "frameSeq": presented_frame.frame_seq,
                            })
                        },
                    );
                    None
                }
            }
        }
        ScheduledFrameTakeOutcome::RetainedDisplayedFrame(frame) => {
            record_host_mailbox_retained_displayed(
                runtime_trace,
                D3D11_NATIVE_PIPELINE,
                viewport_id,
                window_label,
                take_slot_diag,
                take_telemetry_diag,
                now_ms,
            );
            let presented_frame = frame.clone();
            if let Err(error) = renderer.render_frame(&frame) {
                record_native_video_timing_event_lazy(
                    runtime_trace,
                    D3D11_NATIVE_PIPELINE,
                    "present_tick_failed",
                    viewport_id,
                    window_label,
                    || {
                        serde_json::json!({
                            "reason": "retainedRenderFailed",
                            "error": error,
                            "frameSeq": presented_frame.frame_seq,
                        })
                    },
                );
            } else if let Ok(mut telemetry) = telemetry.lock() {
                telemetry.record_present_refresh(now_ms);
                let telemetry_diag = telemetry.diagnostics_snapshot();
                drop(telemetry);
                record_host_frame_presented(
                    runtime_trace,
                    D3D11_NATIVE_PIPELINE,
                    viewport_id,
                    window_label,
                    HostFramePresentedFacts::from_render_frame(&presented_frame, "refresh"),
                    Some(take_slot_diag),
                    Some(&telemetry_diag),
                    now_ms,
                );
            }
            None
        }
        ScheduledFrameTakeOutcome::RetainedDisplayedFrameStale {
            frame,
            frame_age_ms,
            frame_age_budget_ms,
        } => {
            record_host_mailbox_retained_displayed_stale(
                runtime_trace,
                D3D11_NATIVE_PIPELINE,
                viewport_id,
                window_label,
                &frame,
                frame_age_ms,
                frame_age_budget_ms,
                take_slot_diag,
                take_telemetry_diag,
            );
            None
        }
        ScheduledFrameTakeOutcome::NoPendingFrame => {
            record_host_mailbox_idle(
                runtime_trace,
                D3D11_NATIVE_PIPELINE,
                viewport_id,
                window_label,
                take_slot_diag,
                take_telemetry_diag,
                now_ms,
            );
            if !has_cached_frame && size_changed {
                let _ = renderer.clear();
            }
            None
        }
        ScheduledFrameTakeOutcome::DroppedStale {
            frame,
            frame_age_ms,
            frame_age_budget_ms,
        } => {
            record_host_mailbox_rejected_stale(
                runtime_trace,
                D3D11_NATIVE_PIPELINE,
                viewport_id,
                window_label,
                &frame,
                frame_age_ms,
                frame_age_budget_ms,
                take_slot_diag,
                take_telemetry_diag,
            );
            None
        }
    }
}

struct D3d11FrameRenderer {
    device: ID3D11Device,
    device3: ID3D11Device3,
    context: ID3D11DeviceContext,
    swap_chain: IDXGISwapChain1,
    render_target_view: Option<ID3D11RenderTargetView>,
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    params_buffer: ID3D11Buffer,
    width: u32,
    height: u32,
}

unsafe impl Send for D3d11FrameRenderer {}

impl D3d11FrameRenderer {
    fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self, String> {
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                None::<&IDXGIAdapter>,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None::<*mut D3D_FEATURE_LEVEL>,
                Some(&mut context),
            )
            .map_err(|error| format!("d3d11CreateDeviceFailed:{error}"))?;
        }
        let device = device.ok_or_else(|| "d3d11CreateDeviceReturnedNoDevice".to_string())?;
        let device3 = device
            .cast::<ID3D11Device3>()
            .map_err(|error| format!("d3d11Device3Unavailable:{error}"))?;
        let context = context.ok_or_else(|| "d3d11CreateDeviceReturnedNoContext".to_string())?;
        let factory: IDXGIFactory2 = unsafe { CreateDXGIFactory1() }
            .map_err(|error| format!("dxgiCreateFactoryFailed:{error}"))?;
        let swap_desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width.max(1),
            Height: height.max(1),
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: 0,
        };
        let swap_chain = unsafe {
            factory.CreateSwapChainForHwnd(&device, hwnd, &swap_desc, None, None::<&IDXGIOutput>)
        }
        .map_err(|error| format!("dxgiCreateSwapChainForHwndFailed:{error}"))?;
        let render_target_view = create_render_target_view(&device, &swap_chain)?;
        let vertex_shader = compile_vertex_shader(&device)?;
        let pixel_shader = compile_pixel_shader(&device)?;
        let sampler = create_sampler(&device)?;
        let params_buffer = create_params_buffer(&device)?;
        Ok(Self {
            device,
            device3,
            context,
            swap_chain,
            render_target_view: Some(render_target_view),
            vertex_shader,
            pixel_shader,
            sampler,
            params_buffer,
            width: width.max(1),
            height: height.max(1),
        })
    }

    fn update_surface_size(&mut self, width: u32, height: u32) -> Result<(), String> {
        let width = width.max(1);
        let height = height.max(1);
        if self.width == width && self.height == height {
            return Ok(());
        }
        self.render_target_view = None;
        unsafe {
            self.swap_chain
                .ResizeBuffers(
                    0,
                    width,
                    height,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )
                .map_err(|error| format!("dxgiResizeBuffersFailed:{error}"))?;
        }
        self.render_target_view = Some(create_render_target_view(&self.device, &self.swap_chain)?);
        self.width = width;
        self.height = height;
        Ok(())
    }

    fn render_frame(&mut self, frame: &XbxEngineRenderFrame) -> Result<(), String> {
        let XbxEngineRenderPixelData::Descriptor { handle } = &frame.pixel_data else {
            return Err("d3d11NativeRejectedCpuSurface".to_string());
        };
        let descriptor = handle
            .downcast_ref::<WindowsD3d11TextureDescriptor>()
            .ok_or_else(|| "d3d11NativeUnsupportedDescriptor".to_string())?;
        if descriptor.dxgi_format != DXGI_FORMAT_NV12_VALUE {
            return Err(format!(
                "d3d11NativeUnsupportedDxgiFormat:{}",
                descriptor.dxgi_format
            ));
        }
        let texture = self.open_shared_texture(descriptor)?;
        let y_view = self.create_nv12_srv(&texture, descriptor.array_slice, 0)?;
        let uv_view = self.create_nv12_srv(&texture, descriptor.array_slice, 1)?;
        let params = build_nv12_color_params(
            descriptor.color_matrix,
            descriptor.color_range,
            descriptor.chroma_location,
            frame.width,
            frame.height,
        );
        self.draw_nv12(&y_view, &uv_view, &params)?;
        unsafe {
            self.swap_chain
                .Present(1, 0)
                .ok()
                .map_err(|error| format!("dxgiPresentFailed:{error}"))?;
        }
        Ok(())
    }

    fn clear(&mut self) -> Result<(), String> {
        let render_target_view = self
            .render_target_view
            .as_ref()
            .ok_or_else(|| "d3d11RenderTargetViewMissing".to_string())?;
        unsafe {
            self.context
                .ClearRenderTargetView(render_target_view, &[0.0_f32, 0.0_f32, 0.0_f32, 1.0_f32]);
            self.swap_chain
                .Present(1, 0)
                .ok()
                .map_err(|error| format!("dxgiPresentClearFailed:{error}"))?;
        }
        Ok(())
    }

    fn open_shared_texture(
        &self,
        descriptor: &WindowsD3d11TextureDescriptor,
    ) -> Result<ID3D11Texture2D, String> {
        unsafe {
            self.device3
                .OpenSharedResource1::<ID3D11Texture2D>(HANDLE(descriptor.shared_handle))
                .map_err(|error| format!("d3d11OpenSharedResource1Failed:{error}"))
        }
    }

    fn create_nv12_srv(
        &self,
        texture: &ID3D11Texture2D,
        array_slice: u32,
        plane_slice: u32,
    ) -> Result<ID3D11ShaderResourceView, String> {
        let format = if plane_slice == 0 {
            DXGI_FORMAT_R8_UNORM
        } else {
            DXGI_FORMAT_R8G8_UNORM
        };
        let srv_desc = D3D11_SHADER_RESOURCE_VIEW_DESC1 {
            Format: format,
            ViewDimension: D3D_SRV_DIMENSION_TEXTURE2DARRAY,
            Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC1_0 {
                Texture2DArray: D3D11_TEX2D_ARRAY_SRV1 {
                    MostDetailedMip: 0,
                    MipLevels: 1,
                    FirstArraySlice: array_slice,
                    ArraySize: 1,
                    PlaneSlice: plane_slice,
                },
            },
        };
        let mut srv1 = None;
        unsafe {
            self.device3
                .CreateShaderResourceView1(texture, Some(&srv_desc), Some(&mut srv1))
                .map_err(|error| format!("d3d11CreateNv12PlaneSrvFailed:{error}"))?;
        }
        let srv1 = srv1.ok_or_else(|| "d3d11CreateNv12PlaneSrvReturnedNone".to_string())?;
        srv1.cast::<ID3D11ShaderResourceView>()
            .map_err(|error| format!("d3d11CastSrv1Failed:{error}"))
    }

    fn draw_nv12(
        &self,
        y_view: &ID3D11ShaderResourceView,
        uv_view: &ID3D11ShaderResourceView,
        params: &Nv12ColorParams,
    ) -> Result<(), String> {
        unsafe {
            self.context.UpdateSubresource(
                &self.params_buffer,
                0,
                None,
                (params as *const Nv12ColorParams).cast(),
                std::mem::size_of::<Nv12ColorParams>() as u32,
                0,
            );
            self.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: self.width as f32,
                Height: self.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            self.context.OMSetRenderTargets(
                Some(&[Some(render_target_view.clone())]),
                None::<&ID3D11DepthStencilView>,
            );
            self.context.VSSetShader(&self.vertex_shader, None);
            self.context.PSSetShader(&self.pixel_shader, None);
            self.context
                .PSSetShaderResources(0, Some(&[Some(y_view.clone()), Some(uv_view.clone())]));
            self.context
                .PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            self.context
                .PSSetConstantBuffers(0, Some(&[Some(self.params_buffer.clone())]));
            self.context.Draw(3, 0);
            self.context.PSSetShaderResources(0, Some(&[None, None]));
        }
        Ok(())
    }
}

fn create_render_target_view(
    device: &ID3D11Device,
    swap_chain: &IDXGISwapChain1,
) -> Result<ID3D11RenderTargetView, String> {
    let back_buffer: ID3D11Texture2D = unsafe { swap_chain.GetBuffer(0) }
        .map_err(|error| format!("dxgiGetBackBufferFailed:{error}"))?;
    let mut render_target_view = None;
    unsafe {
        device
            .CreateRenderTargetView(&back_buffer, None, Some(&mut render_target_view))
            .map_err(|error| format!("d3d11CreateRenderTargetViewFailed:{error}"))?;
    }
    render_target_view.ok_or_else(|| "d3d11CreateRenderTargetViewReturnedNone".to_string())
}

fn compile_vertex_shader(device: &ID3D11Device) -> Result<ID3D11VertexShader, String> {
    let bytecode = compile_shader(D3D11_VERTEX_SHADER, b"vs_main\0", b"vs_5_0\0")?;
    let mut shader = None;
    unsafe {
        device
            .CreateVertexShader(bytecode.as_slice(), None, Some(&mut shader))
            .map_err(|error| format!("d3d11CreateVertexShaderFailed:{error}"))?;
    }
    shader.ok_or_else(|| "d3d11CreateVertexShaderReturnedNone".to_string())
}

fn compile_pixel_shader(device: &ID3D11Device) -> Result<ID3D11PixelShader, String> {
    let bytecode = compile_shader(D3D11_PIXEL_SHADER, b"ps_main\0", b"ps_5_0\0")?;
    let mut shader = None;
    unsafe {
        device
            .CreatePixelShader(bytecode.as_slice(), None, Some(&mut shader))
            .map_err(|error| format!("d3d11CreatePixelShaderFailed:{error}"))?;
    }
    shader.ok_or_else(|| "d3d11CreatePixelShaderReturnedNone".to_string())
}

fn compile_shader(
    source: &[u8],
    entry: &'static [u8],
    target: &'static [u8],
) -> Result<Vec<u8>, String> {
    let mut code = None;
    let mut errors = None;
    unsafe {
        D3DCompile(
            source.as_ptr().cast(),
            source.len(),
            PCSTR::null(),
            None,
            None::<windows::Win32::Graphics::Direct3D::ID3DInclude>,
            PCSTR(entry.as_ptr()),
            PCSTR(target.as_ptr()),
            0,
            0,
            &mut code,
            Some(&mut errors),
        )
        .map_err(|error| {
            let message = errors
                .as_ref()
                .map(blob_to_string)
                .unwrap_or_else(|| error.to_string());
            format!("d3dCompileFailed:{message}")
        })?;
    }
    let code = code.ok_or_else(|| "d3dCompileReturnedNoCode".to_string())?;
    let bytes = unsafe {
        std::slice::from_raw_parts(code.GetBufferPointer().cast::<u8>(), code.GetBufferSize())
    };
    Ok(bytes.to_vec())
}

fn blob_to_string(blob: &ID3DBlob) -> String {
    let bytes = unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize())
    };
    String::from_utf8_lossy(bytes).trim().to_string()
}

fn create_sampler(device: &ID3D11Device) -> Result<ID3D11SamplerState, String> {
    let desc = D3D11_SAMPLER_DESC {
        Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
        MipLODBias: 0.0,
        MaxAnisotropy: 1,
        ComparisonFunc: D3D11_COMPARISON_NEVER,
        BorderColor: [0.0, 0.0, 0.0, 0.0],
        MinLOD: 0.0,
        MaxLOD: f32::MAX,
    };
    let mut sampler = None;
    unsafe {
        device
            .CreateSamplerState(&desc, Some(&mut sampler))
            .map_err(|error| format!("d3d11CreateSamplerFailed:{error}"))?;
    }
    sampler.ok_or_else(|| "d3d11CreateSamplerReturnedNone".to_string())
}

fn create_params_buffer(device: &ID3D11Device) -> Result<ID3D11Buffer, String> {
    let initial = build_nv12_color_params(
        MacOsVideoColorMatrix::Bt709,
        MacOsVideoColorRange::Video,
        MacOsVideoChromaLocation::Center,
        1920,
        1080,
    );
    let desc = windows::Win32::Graphics::Direct3D11::D3D11_BUFFER_DESC {
        ByteWidth: std::mem::size_of::<Nv12ColorParams>() as u32,
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_FLAG(0).0 as u32,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: (&initial as *const Nv12ColorParams).cast(),
        SysMemPitch: 0,
        SysMemSlicePitch: 0,
    };
    let mut buffer = None;
    unsafe {
        device
            .CreateBuffer(&desc, Some(&data), Some(&mut buffer))
            .map_err(|error| format!("d3d11CreateParamsBufferFailed:{error}"))?;
    }
    buffer.ok_or_else(|| "d3d11CreateParamsBufferReturnedNone".to_string())
}

fn build_nv12_color_params(
    matrix: MacOsVideoColorMatrix,
    range: MacOsVideoColorRange,
    chroma_location: MacOsVideoChromaLocation,
    width: u32,
    height: u32,
) -> Nv12ColorParams {
    let (row0, row1, row2) = match (matrix, range) {
        (MacOsVideoColorMatrix::Bt601, MacOsVideoColorRange::Full) => (
            [1.0, 0.0, 1.402, -0.701],
            [1.0, -0.344_136, -0.714_136, 0.529_136],
            [1.0, 1.772, 0.0, -0.886],
        ),
        (MacOsVideoColorMatrix::Bt601, MacOsVideoColorRange::Video) => (
            [1.164_384, 0.0, 1.596_027, -0.874_202],
            [1.164_384, -0.391_762, -0.812_968, 0.531_668],
            [1.164_384, 2.017_232, 0.0, -1.085_631],
        ),
        (_, MacOsVideoColorRange::Full) => (
            [1.0, 0.0, 1.574_8, -0.787_4],
            [1.0, -0.187_324, -0.468_124, 0.327_724],
            [1.0, 1.855_6, 0.0, -0.927_8],
        ),
        _ => (
            [1.164_384, 0.0, 1.792_741, -0.972_945],
            [1.164_384, -0.213_249, -0.532_909, 0.301_483],
            [1.164_384, 2.112_402, 0.0, -1.133_402],
        ),
    };
    let uv_offset_x = match chroma_location {
        MacOsVideoChromaLocation::Left | MacOsVideoChromaLocation::TopLeft => {
            -0.25 / width.max(1) as f32
        }
        _ => 0.0,
    };
    let uv_offset_y = match chroma_location {
        MacOsVideoChromaLocation::TopLeft => -0.25 / height.max(1) as f32,
        _ => 0.0,
    };
    Nv12ColorParams {
        row0,
        row1,
        row2,
        uv_offset: [uv_offset_x, uv_offset_y, 0.0, 0.0],
    }
}

fn extract_hwnd(window: &tauri::Window) -> Result<HWND, String> {
    let handle = window
        .window_handle()
        .map_err(|error| format!("windowHandleUnavailable:{error}"))?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Ok(HWND(handle.hwnd.get() as *mut _)),
        other => Err(format!("unsupportedWindowHandle:{other:?}")),
    }
}
