use super::channel::UpdateChannel;
pub const GITHUB_OWNER: &str = "xuasir";
pub const GITHUB_REPO: &str = "xbxrc";
pub fn endpoint_for(_: UpdateChannel) -> String {
    format!(
        "https://github.com/{GITHUB_OWNER}/{GITHUB_REPO}/releases/download/native-wip/latest.json"
    )
}
