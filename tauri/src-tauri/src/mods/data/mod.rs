pub mod application;
pub mod client;
pub mod domain;
pub mod infrastructure;
pub mod rpc;
pub mod types;

pub use application::DataService;
pub use types::*;

use async_trait::async_trait;
use std::sync::Arc;

#[async_trait]
pub trait DataProvider: Send + Sync {
    async fn get_user_profile(&self) -> Result<DataUserProfile, String>;
    async fn get_hosts(&self) -> Result<Vec<DataHostSummary>, String>;
    async fn get_remote_consoles(&self) -> Result<Vec<DataHostSummary>, String>;
    async fn get_streaming_title_input_config(
        &self,
        xbox_title_id: &str,
    ) -> Result<DataStreamingTitleInputConfig, String>;
    async fn power_on_console(&self, console_id: &str) -> Result<DataConsolePowerResult, String>;
    async fn power_off_console(&self, console_id: &str) -> Result<DataConsolePowerResult, String>;
    async fn send_text_to_console(
        &self,
        console_id: &str,
        text: &str,
    ) -> Result<DataSendTextResult, String>;
    async fn get_xcloud_titles(&self) -> Result<Vec<DataXcloudTitleSummary>, String>;
}

pub type DataProviderRef = Arc<dyn DataProvider>;
