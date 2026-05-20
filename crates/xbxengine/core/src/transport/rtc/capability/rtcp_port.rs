use std::sync::{Arc, Mutex};

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::connection::RtcConnectionService;
use crate::transport::rtc::stream::sink::RtcRtcpSendPort;
use crate::XbxEngineMediaRuntimeStats;

/// 经 connection service 写出视频 RTCP（NACK 等）。
pub struct ConnectionRtcpCapability {
    connection: Arc<Mutex<RtcConnectionService>>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
}

impl ConnectionRtcpCapability {
    pub fn new(
        connection: Arc<Mutex<RtcConnectionService>>,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Self {
        Self {
            connection,
            runtime_stats,
        }
    }
}

impl RtcRtcpSendPort for ConnectionRtcpCapability {
    fn send_rtcp(&self, buf: &[u8]) -> Result<(), String> {
        let mut connection = self.connection.lock().map_err(|_| {
            crate::xbx_log_warn!(
                "[xbxengine][rtc][rtcp] drop rtcp payload because connection lock failed"
            );
            "connection lock failed".to_string()
        })?;

        connection
            .send_video_rtcp_payload(buf)
            .map_err(|error| {
                crate::xbx_log_warn!(
                    "[xbxengine][rtc][rtcp] failed to send video rtcp payload: {error}"
                );
                RuntimeStatsSink::new(self.runtime_stats.clone()).record_video_rtcp_send_failure(
                    crate::transport::rtc::stats::now_ms_f64(),
                    &error.to_string(),
                );
                error.to_string()
            })
            .map(|_| {
                RuntimeStatsSink::new(self.runtime_stats.clone())
                    .record_feedback_target_availability(
                        crate::transport::rtc::stats::now_ms_f64(),
                        "videoRtcpFeedback",
                        "ready",
                        "rtcpSendSucceeded",
                    );
            })
    }
}
