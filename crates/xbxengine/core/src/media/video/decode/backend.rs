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

impl XbxVideoDecoderBackendKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Hardware => "hardware",
            Self::Software => "software",
            Self::Placeholder => "placeholder",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum XbxVideoBackendFailureReason {
    NotSupportedOnPlatform,
    InitializationFailed,
    Unavailable,
}

impl XbxVideoBackendFailureReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NotSupportedOnPlatform => "not-supported-on-platform",
            Self::InitializationFailed => "initialization-failed",
            Self::Unavailable => "unavailable",
        }
    }
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

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct XbxVideoDecoderProbeSummary {
    pub(crate) selected_backend_name: String,
    pub(crate) selected_backend_kind: String,
    pub(crate) fallback_count: u32,
    pub(crate) fallback_summary: Option<String>,
}

pub(crate) trait XbxVideoDecoderBackend: Send {
    fn backend_name(&self) -> &'static str;
    fn decode(
        &mut self,
        encoded_frame: EncodedFrame,
        now_ms: f64,
    ) -> Result<Option<XbxRenderFrame>, XbxEngineRuntimeError>;
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
}

pub(crate) fn create_video_decoder_backend_with_probe(
) -> (Box<dyn XbxVideoDecoderBackend>, XbxVideoDecoderProbeSummary) {
    let mut fallback_details = Vec::new();
    for probe in probe_video_decoder_backends() {
        match probe {
            XbxVideoBackendProbeResult::Selected(decoder) => {
                let summary = XbxVideoDecoderProbeSummary {
                    selected_backend_name: decoder.backend_name().to_string(),
                    selected_backend_kind: resolve_selected_backend_kind(decoder.backend_name())
                        .as_str()
                        .to_string(),
                    fallback_count: fallback_details.len() as u32,
                    fallback_summary: (!fallback_details.is_empty())
                        .then(|| fallback_details.join(" -> ")),
                };
                return (decoder, summary);
            }
            XbxVideoBackendProbeResult::Rejected {
                candidate,
                reason,
                error,
            } => match error {
                Some(error) => {
                    fallback_details.push(format!(
                        "{}({}/{}):{}",
                        candidate.backend_name,
                        candidate.kind.as_str(),
                        reason.as_str(),
                        error
                    ));
                }
                None => {
                    fallback_details.push(format!(
                        "{}({}/{})",
                        candidate.backend_name,
                        candidate.kind.as_str(),
                        reason.as_str()
                    ));
                }
            },
        }
    }

    let decoder = Box::<NoopXbxVideoDecoderBackend>::default();
    let summary = XbxVideoDecoderProbeSummary {
        selected_backend_name: decoder.backend_name().to_string(),
        selected_backend_kind: XbxVideoDecoderBackendKind::Placeholder.as_str().to_string(),
        fallback_count: fallback_details.len() as u32,
        fallback_summary: (!fallback_details.is_empty()).then(|| fallback_details.join(" -> ")),
    };
    (decoder, summary)
}

fn resolve_selected_backend_kind(backend_name: &str) -> XbxVideoDecoderBackendKind {
    match backend_name {
        "ffmpeg-software" => XbxVideoDecoderBackendKind::Software,
        "noop" => XbxVideoDecoderBackendKind::Placeholder,
        _ => XbxVideoDecoderBackendKind::Hardware,
    }
}

fn probe_video_decoder_backends() -> Vec<XbxVideoBackendProbeResult> {
    let mut probes = Vec::new();

    #[cfg(target_os = "windows")]
    {
        let candidate = XbxVideoBackendCandidate {
            kind: XbxVideoDecoderBackendKind::Hardware,
            backend_name: "ffmpeg-d3d11va",
        };
        match super::backend_ffmpeg_windows_d3d11va::try_create_ffmpeg_windows_d3d11va_backend() {
            Ok(decoder) => probes.push(XbxVideoBackendProbeResult::Selected(decoder)),
            Err(error) => probes.push(XbxVideoBackendProbeResult::Rejected {
                candidate,
                reason: XbxVideoBackendFailureReason::InitializationFailed,
                error: Some(error),
            }),
        }
    }

    #[cfg(not(target_os = "windows"))]
    probes.push(XbxVideoBackendProbeResult::Rejected {
        candidate: XbxVideoBackendCandidate {
            kind: XbxVideoDecoderBackendKind::Hardware,
            backend_name: "ffmpeg-d3d11va",
        },
        reason: XbxVideoBackendFailureReason::NotSupportedOnPlatform,
        error: None,
    });

    #[cfg(target_os = "macos")]
    {
        let candidate = XbxVideoBackendCandidate {
            kind: XbxVideoDecoderBackendKind::Hardware,
            backend_name: "ffmpeg-videotoolbox",
        };
        match super::backend_ffmpeg_macos_videotoolbox::try_create_ffmpeg_macos_videotoolbox_backend(
        ) {
            Ok(decoder) => probes.push(XbxVideoBackendProbeResult::Selected(decoder)),
            Err(error) => probes.push(XbxVideoBackendProbeResult::Rejected {
                candidate,
                reason: XbxVideoBackendFailureReason::InitializationFailed,
                error: Some(error),
            }),
        }
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    probes.push(XbxVideoBackendProbeResult::Rejected {
        candidate: XbxVideoBackendCandidate {
            kind: XbxVideoDecoderBackendKind::Hardware,
            backend_name: "ffmpeg-videotoolbox",
        },
        reason: XbxVideoBackendFailureReason::NotSupportedOnPlatform,
        error: None,
    });

    {
        let candidate = XbxVideoBackendCandidate {
            kind: XbxVideoDecoderBackendKind::Software,
            backend_name: "ffmpeg-software",
        };
        match super::backend_ffmpeg_software::try_create_ffmpeg_software_backend() {
            Ok(decoder) => probes.push(XbxVideoBackendProbeResult::Selected(decoder)),
            Err(error) => probes.push(XbxVideoBackendProbeResult::Rejected {
                candidate,
                reason: XbxVideoBackendFailureReason::InitializationFailed,
                error: Some(error),
            }),
        }
    }

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
