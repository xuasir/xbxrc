use super::channel::UpdateChannel;

pub const GITHUB_OWNER: &str = "xuasir";
pub const GITHUB_REPO: &str = "xbxrc";

pub fn endpoint_for(channel: UpdateChannel) -> String {
    match channel {
        UpdateChannel::Stable => format!(
            "https://github.com/{GITHUB_OWNER}/{GITHUB_REPO}/releases/latest/download/latest.json"
        ),
        UpdateChannel::Beta => format!(
            "https://github.com/{GITHUB_OWNER}/{GITHUB_REPO}/releases/download/beta/latest.json"
        ),
    }
}
