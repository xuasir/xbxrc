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
        web_api.get_consoles_list().await
    }
}
