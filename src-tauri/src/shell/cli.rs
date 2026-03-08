use super::state::StartupFlagsState;

pub fn parse_startup_flags() -> StartupFlagsState {
    let mut fullscreen = false;
    let mut auto_connect = String::new();

    for arg in std::env::args() {
        if arg.contains("--fullscreen") {
            fullscreen = true;
        }

        if let Some(value) = arg.strip_prefix("--auto-connect=") {
            auto_connect = value.trim().to_string();
        }
    }

    StartupFlagsState {
        fullscreen,
        auto_connect,
    }
}
