use crate::{
    media::video::render::renderer::XbxRenderFrame, media::video::types::EncodedFrame,
    XbxEngineRuntimeError,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum XbxVideoDecoderBackendKind {
    Hardware,
    Software,
    Placeholder,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum XbxVideoBackendFailureReason {
    NotSupportedOnPlatform,
    InitializationFailed,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct XbxVideoBackendCandidate {
    pub(crate) kind: XbxVideoDecoderBackendKind,
    pub(crate) backend_name: &'static str,
}

pub(crate) enum XbxVideoBackendProbeResult {
    Selected(Box<dyn XbxVideoDecoderBackend>),
    Rejected {
        candidate: XbxVideoBackendCandidate,
        reason: XbxVideoBackendFailureReason,
        error: Option<XbxEngineRuntimeError>,
    },
}

pub(crate) trait XbxVideoDecoderBackend: Send {
    fn backend_name(&self) -> &'static str;
    fn decode(
        &mut self,
        encoded_frame: EncodedFrame,
        now_ms: f64,
    ) -> Result<Option<XbxRenderFrame>, XbxEngineRuntimeError>;
    fn reset(&mut self) -> Result<(), XbxEngineRuntimeError>;
}

#[derive(Default)]
struct NoopXbxVideoDecoderBackend;

impl XbxVideoDecoderBackend for NoopXbxVideoDecoderBackend {
    fn backend_name(&self) -> &'static str {
        "noop"
    }

    fn decode(
        &mut self,
        _encoded_frame: EncodedFrame,
        _now_ms: f64,
    ) -> Result<Option<XbxRenderFrame>, XbxEngineRuntimeError> {
        Ok(None)
    }

    fn reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        Ok(())
    }
}

pub(crate) fn create_video_decoder_backend() -> Box<dyn XbxVideoDecoderBackend> {
    for probe in probe_video_decoder_backends() {
        match probe {
            XbxVideoBackendProbeResult::Selected(decoder) => return decoder,
            XbxVideoBackendProbeResult::Rejected {
                candidate,
                reason,
                error,
            } => match error {
                Some(error) => crate::xbx_log_info!(
                    "[xbxengine][rtc] skip decoder backend={} kind={:?} reason={reason:?} error={error}",
                    candidate.backend_name,
                    candidate.kind
                ),
                None => crate::xbx_log_info!(
                    "[xbxengine][rtc] skip decoder backend={} kind={:?} reason={reason:?}",
                    candidate.backend_name,
                    candidate.kind
                ),
            },
        }
    }

    Box::<NoopXbxVideoDecoderBackend>::default()
}

fn probe_video_decoder_backends() -> Vec<XbxVideoBackendProbeResult> {
    let mut probes = Vec::new();

    #[cfg(not(target_os = "macos"))]
    probes.push(XbxVideoBackendProbeResult::Rejected {
        candidate: XbxVideoBackendCandidate {
            kind: XbxVideoDecoderBackendKind::Hardware,
            backend_name: "videotoolbox",
        },
        reason: XbxVideoBackendFailureReason::NotSupportedOnPlatform,
        error: None,
    });

    #[cfg(target_os = "macos")]
    {
        let candidate = XbxVideoBackendCandidate {
            kind: XbxVideoDecoderBackendKind::Hardware,
            backend_name: "videotoolbox",
        };
        match super::backend_macos_videotoolbox::try_create_macos_videotoolbox_backend() {
            Ok(decoder) => probes.push(XbxVideoBackendProbeResult::Selected(decoder)),
            Err(error) => probes.push(XbxVideoBackendProbeResult::Rejected {
                candidate,
                reason: XbxVideoBackendFailureReason::InitializationFailed,
                error: Some(error),
            }),
        }
    }

    probes.push(XbxVideoBackendProbeResult::Rejected {
        candidate: XbxVideoBackendCandidate {
            kind: XbxVideoDecoderBackendKind::Software,
            backend_name: "software-placeholder",
        },
        reason: XbxVideoBackendFailureReason::Unavailable,
        error: None,
    });

    if probes.is_empty() {
        probes.push(XbxVideoBackendProbeResult::Rejected {
            candidate: XbxVideoBackendCandidate {
                kind: XbxVideoDecoderBackendKind::Placeholder,
                backend_name: "noop",
            },
            reason: XbxVideoBackendFailureReason::Unavailable,
            error: None,
        });
    }

    probes
}
