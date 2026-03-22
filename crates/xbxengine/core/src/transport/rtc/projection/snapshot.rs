use super::{
    BweProjection, ConnectionProjection, DiagnosticsProjection, MediaProjection, RecoveryProjection,
};

#[derive(Clone, Debug, PartialEq)]
pub struct TransportSnapshot {
    pub version: u64,
    pub now_ms: f64,
    pub connection: ConnectionProjection,
    pub media: MediaProjection,
    pub recovery: RecoveryProjection,
    pub bwe: BweProjection,
    pub diagnostics: DiagnosticsProjection,
}

impl TransportSnapshot {
    pub fn new(
        version: u64,
        now_ms: f64,
        connection: ConnectionProjection,
        media: MediaProjection,
        recovery: RecoveryProjection,
        bwe: BweProjection,
        diagnostics: DiagnosticsProjection,
    ) -> Self {
        Self {
            version,
            now_ms,
            connection,
            media,
            recovery,
            bwe,
            diagnostics,
        }
    }
}
