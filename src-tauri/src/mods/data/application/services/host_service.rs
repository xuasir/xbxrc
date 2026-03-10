use crate::mods::data::client::XboxWebApiClient;
use crate::mods::data::domain::DataSessionContext;
use crate::mods::data::types::DataHostSummary;

pub struct HostService;

impl HostService {
    pub fn new() -> Self {
        Self
    }

    // 与 Electron 语义一致：主机列表来自 smartglass provider。
    pub async fn get_hosts(
        &self,
        _session: &DataSessionContext,
        web_api: &XboxWebApiClient,
    ) -> Result<Vec<DataHostSummary>, String> {
        match tokio::time::timeout(
            std::time::Duration::from_secs(8),
            web_api.get_consoles_list(),
        )
        .await
        {
            Ok(Ok(hosts)) => Ok(hosts),
            Ok(Err(error)) => {
                // 与迁移前 JS 行为保持一致：网络波动时降级为空数组，避免把 hosts 查询变成致命错误。
                log::warn!(
                    "[Data] load hosts failed, fallback to empty list: {}",
                    error
                );
                Ok(Vec::new())
            }
            Err(_) => {
                log::warn!("[Data] load hosts timeout, fallback to empty list");
                Ok(Vec::new())
            }
        }
    }
}
