use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameProgress {
    pub unlocked_count: Option<u32>,
    pub total_count: Option<u32>,
    pub earned_gamerscore: Option<u32>,
    pub total_gamerscore: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryGame {
    pub description: Option<String>,
    pub xbox_title_id: String,
    pub scid: Option<String>,
    pub product_id: Option<String>,
    pub name: String,
    pub artwork_url: Option<String>,
    pub last_played_at: Option<String>,
    pub progress: Option<GameProgress>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AchievementRequirement {
    pub id: String,
    pub current: Option<String>,
    pub target: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryAchievement {
    pub id: String,
    pub scid: Option<String>,
    pub name: String,
    pub description: String,
    pub locked_description: String,
    pub image_url: Option<String>,
    pub is_secret: bool,
    pub is_unlocked: bool,
    pub gamerscore: Option<u32>,
    pub progress_percentage: Option<f64>,
    pub requirements: Vec<AchievementRequirement>,
    pub unlocked_at: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryPerson {
    pub xuid: String,
    pub display_name: String,
    pub gamertag: String,
    pub avatar_url: Option<String>,
    pub gamerscore: Option<String>,
    pub presence_state: Option<String>,
    pub presence_text: Option<String>,
    pub current_title_id: Option<String>,
    pub is_followed_by_caller: Option<bool>,
    pub is_following_caller: Option<bool>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledGame {
    pub id: String,
    pub xbox_title_id: Option<String>,
    pub product_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub artwork_url: Option<String>,
    pub storage_device_id: Option<String>,
    pub size_bytes: Option<u64>,
    pub installed_at: Option<String>,
    pub last_played_at: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleStorage {
    pub id: String,
    pub name: String,
    pub free_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    Screenshot,
    Clip,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryMedia {
    pub id: String,
    pub kind: MediaKind,
    pub xbox_title_id: Option<String>,
    pub title_name: String,
    pub captured_at: Option<String>,
    pub duration_seconds: Option<u64>,
    pub thumbnail_url: Option<String>,
    pub url: Option<String>,
    pub expires_at: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryPage<T> {
    pub items: Vec<T>,
    pub continuation: Option<String>,
}
