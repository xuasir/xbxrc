use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateChannel {
    #[serde(rename = "native-wip")]
    NativeWip,
}
impl UpdateChannel {
    pub fn as_str(self) -> &'static str {
        "native-wip"
    }
    pub fn parse(value: &str) -> Option<Self> {
        (value == "native-wip").then_some(Self::NativeWip)
    }
    pub fn default_channel() -> Self {
        Self::NativeWip
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_wip_is_the_only_channel() {
        assert_eq!(
            UpdateChannel::parse("native-wip"),
            Some(UpdateChannel::NativeWip)
        );
        assert!(UpdateChannel::parse("stable").is_none());
        assert!(UpdateChannel::parse("beta").is_none());
    }
}
