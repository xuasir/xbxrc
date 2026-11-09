#!/usr/bin/env python3
"""复跑 XHOME 静默恢复的新旧代码对照；本次基线为 17 通过、1 个目标行为失败。

排查基线 20348f4c 的真实 token_policy/runtime_state 源码直接参加编译；存储和 PendingOAuthLogin 使用离线桩。
该脚本固定历史故障证据，使用合成凭据在临时目录构建。
"""
from pathlib import Path
import os
import subprocess
import tempfile

MANIFEST = r'''
[package]
name = "xbxrc-xhome-auth-audit"
version = "0.0.0"
edition = "2021"

[dependencies]
base64 = "=0.22.1"
chrono = { version = "=0.4.44", features = ["serde"] }
log = "=0.4.29"
serde = { version = "=1.0.228", features = ["derive"] }
serde_json = "=1.0.149"
tokio = { version = "=1.50.0", features = ["macros", "sync", "rt", "time"] }

[workspace]
'''

LIB_SOURCE = r'''
#![allow(dead_code)]

extern crate self as xbox_auth_flow;

#[derive(Clone)]
pub struct PendingOAuthLogin;

pub mod error {
    #[derive(Debug)]
    pub enum AppError {
        Auth(String),
        Data(String),
        Internal(String),
    }
    pub type AppResult<T> = Result<T, AppError>;
}

pub mod mods {
    pub mod auth {
        #[path = "__REPO_ROOT__/src-tauri/src/mods/auth/types.rs"]
        pub mod types;

        pub mod storage_repository {
            use serde_json::Value;
            use std::sync::{Arc, Mutex};

            #[derive(Clone)]
            pub struct AuthStorageRepository {
                values: Arc<Mutex<(Option<Value>, Option<Value>)>>,
            }

            impl AuthStorageRepository {
                pub fn fixture(stream: Value, web: Value) -> Self {
                    Self {
                        values: Arc::new(Mutex::new((Some(stream), Some(web)))),
                    }
                }
                pub fn get_stream_tokens(&self) -> Result<Option<Value>, String> {
                    Ok(self.values.lock().unwrap().0.clone())
                }
                pub fn set_stream_tokens(&self, value: Value) -> Result<(), String> {
                    self.values.lock().unwrap().0 = Some(value);
                    Ok(())
                }
                pub fn get_web_token(&self) -> Result<Option<Value>, String> {
                    Ok(self.values.lock().unwrap().1.clone())
                }
                pub fn set_web_token(&self, value: Value) -> Result<(), String> {
                    self.values.lock().unwrap().1 = Some(value);
                    Ok(())
                }
                pub fn clear_ephemeral_tokens(&self) -> Result<(), String> {
                    *self.values.lock().unwrap() = (None, None);
                    Ok(())
                }
            }
        }

        #[path = "__LEGACY_PATH__"]
        pub mod legacy_runtime_state;
        #[path = "__REPO_ROOT__/src-tauri/src/mods/auth/runtime_state.rs"]
        pub mod runtime_state;
        #[path = "__REPO_ROOT__/src-tauri/src/mods/auth/token_policy.rs"]
        pub mod token_policy;
    }
}

#[cfg(test)]
mod audit {
    use super::mods::auth::{
        legacy_runtime_state::AuthRuntimeState as LegacyState, runtime_state::AuthRuntimeState,
        storage_repository::AuthStorageRepository, token_policy::AuthTokenPolicy,
    };
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use serde_json::{json, Value};

    fn valid_web(now: i64) -> Value {
        json!({"data": {
            "Token": "fixture-web-token",
            "DisplayClaims": {"xui": [{"uhs": "fixture-account"}]},
            "NotAfter": chrono::DateTime::from_timestamp_millis(now + 16 * 3_600_000)
                .unwrap().to_rfc3339(),
        }})
    }

    fn home_token(created: i64) -> Value {
        json!({
            "_objectCreateTime": created,
            "data": {"durationInSeconds": 14_400, "gsToken": "fixture-home-token"},
        })
    }

    fn expired_home_policy() -> AuthTokenPolicy {
        let now = chrono::Utc::now().timestamp_millis();
        let stream = json!({"xHomeToken": home_token(now - 5 * 3_600_000)});
        AuthTokenPolicy::new(AuthStorageRepository::fixture(stream, valid_web(now)))
    }

    #[test]
    fn historical_startup_attempts_silent_restore_for_expired_home() {
        let policy = expired_home_policy();
        let snapshot = policy.get_valid_session_snapshot().unwrap();
        assert!(snapshot.is_none());
        let runtime = LegacyState::new("xal".into(), snapshot.is_some(), 1);
        let state = runtime.sync_state_from_snapshot(snapshot.as_ref());
        assert!(
            !state.is_authenticated && !state.is_authenticating,
            "shell startup must enter check_authentication for expired XHOME",
        );
    }

    #[test]
    fn current_startup_should_attempt_silent_restore_for_expired_home() {
        let policy = expired_home_policy();
        let snapshot = policy.get_valid_session_snapshot().unwrap();
        let web = policy.get_web_token().unwrap();
        assert!(snapshot.is_none());
        assert!(policy.is_web_token_valid(web.as_ref()));
        let runtime = AuthRuntimeState::new("xal".into(), snapshot.is_some(), 1);
        let state = runtime
            .sync_state_from_snapshot(snapshot.as_ref(), policy.is_web_token_valid(web.as_ref()));
        assert!(
            !state.is_authenticated && !state.is_authenticating,
            "actual shell gate skips check_authentication: authenticated={}, appLevel={}",
            state.is_authenticated,
            state.app_level,
        );
    }

    #[test]
    fn four_hour_home_expiry_keeps_web_identity_usable() {
        let policy = expired_home_policy();
        let stream = policy.get_stream_tokens().unwrap().unwrap();
        let web = policy.get_web_token().unwrap();
        assert!(!policy.is_stream_token_valid(stream.get("xHomeToken")));
        assert!(policy.is_web_token_valid(web.as_ref()));
        assert!(policy.get_valid_session_snapshot().unwrap().is_none());
    }

    #[test]
    fn stream_validity_uses_milliseconds_and_reserves_sixty_seconds() {
        let policy = expired_home_policy();
        let now = chrono::Utc::now().timestamp_millis();
        assert!(policy.is_stream_token_valid(Some(&home_token(now))));
        assert!(!policy.is_stream_token_valid(Some(&home_token(now - 14_400_000 + 59_000,))));
        assert!(policy.is_stream_token_valid(Some(&home_token(now - 14_400_000 + 121_000,))));
    }

    #[test]
    fn jwt_expiry_fallback_uses_seconds() {
        let policy = expired_home_policy();
        let now = chrono::Utc::now().timestamp();
        for (exp, expected) in [(now + 1_200, true), (now - 120, false)] {
            let segment = URL_SAFE_NO_PAD.encode(json!({"exp": exp}).to_string());
            let token = json!({"data": {"gsToken": format!("fixture.{segment}.fixture")}});
            assert_eq!(policy.is_stream_token_valid(Some(&token)), expected);
        }
    }

    #[test]
    fn generic_snapshot_accepts_cloud_while_home_is_expired() {
        let now = chrono::Utc::now().timestamp_millis();
        let stream = json!({
            "xHomeToken": home_token(now - 5 * 3_600_000),
            "xCloudToken": home_token(now),
        });
        let policy = AuthTokenPolicy::new(AuthStorageRepository::fixture(stream, valid_web(now)));
        let snapshot = policy.get_valid_session_snapshot().unwrap().unwrap();
        assert_eq!(snapshot.app_level, 2);
        assert!(!policy.is_stream_token_valid(snapshot.streaming_tokens.get("xHomeToken")));
    }
}
'''

def main():
    repo = Path(__file__).resolve().parents[2]
    output = Path(tempfile.mkdtemp(prefix="xbxrc-xhome-auth-audit-"))
    (output / "src").mkdir()
    baseline = output / "baseline"
    auth_dir = baseline / "src-tauri/src/mods/auth"
    auth_dir.mkdir(parents=True)
    for filename in ["types.rs", "runtime_state.rs", "runtime_state.test.rs", "token_policy.rs", "token_policy.test.rs"]:
        historical = subprocess.run(
            ["git", "show", f"20348f4c:src-tauri/src/mods/auth/{filename}"],
            cwd=repo, check=True, capture_output=True, text=True,
        ).stdout
        (auth_dir / filename).write_text(historical)
    legacy = output / "legacy-runtime-state.rs"
    previous = subprocess.run(
        ["git", "show", "21fd04f0^:src-tauri/src/mods/auth/runtime_state.rs"],
        cwd=repo, check=True, capture_output=True, text=True,
    ).stdout
    legacy.write_text(previous)
    source = LIB_SOURCE.replace("__REPO_ROOT__", baseline.as_posix()).replace("__LEGACY_PATH__", legacy.as_posix())
    (output / "src/lib.rs").write_text(source)
    (output / "Cargo.toml").write_text(MANIFEST)
    environment = dict(os.environ)
    environment["CARGO_TARGET_DIR"] = str(output / "target")
    print("Offline audit output:", output, flush=True)
    result = subprocess.run(
        ["cargo", "test", "--manifest-path", str(output / "Cargo.toml"), "--offline", "--", "--nocapture"],
        cwd=repo, env=environment, capture_output=True, text=True,
    )
    (output / "test-output.txt").write_text(result.stdout + result.stderr)
    print(result.stdout, end="")
    print(result.stderr, end="")
    print("Audit output:", output, flush=True)
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
