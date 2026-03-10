use crate::mods::auth::types::{CoreTokenPayload, JwtKeysPayload, SisuTokenData, UserTokenData};
use crate::settings_store::{ResolvedSettingsStore, SettingsStoreResolver};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::AppHandle;

pub struct AuthStorageRepository {
    settings_store: SettingsStoreResolver,
}

impl AuthStorageRepository {
    pub fn new(app_handle: AppHandle) -> Self {
        Self {
            settings_store: SettingsStoreResolver::new(app_handle),
        }
    }

    fn open_read_store(&self) -> Result<ResolvedSettingsStore, String> {
        self.settings_store.open_read()
    }

    fn open_write_store(&self) -> Result<ResolvedSettingsStore, String> {
        self.settings_store.open_write()
    }

    fn get_payload(&self) -> Result<CoreTokenPayload, String> {
        let store = self.open_read_store()?;

        let val = store.store().get("auth.tokens.core");

        if let Some(v) = val {
            serde_json::from_value(v.clone()).map_err(|e| e.to_string())
        } else {
            Ok(CoreTokenPayload::default())
        }
    }

    fn set_payload(&self, payload: &CoreTokenPayload) -> Result<(), String> {
        let store = self.open_write_store()?;

        let mut p = payload.clone();

        p.token_update_time = Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_millis() as u64,
        );

        let val = serde_json::to_value(p).map_err(|e| e.to_string())?;
        store.store().set("auth.tokens.core", val);
        store.save()?;

        Ok(())
    }

    pub fn get_user_token(&self) -> Result<Option<UserTokenData>, String> {
        let payload = self.get_payload()?;
        Ok(payload.user_token)
    }

    pub fn set_user_token(&self, user_token: UserTokenData) -> Result<(), String> {
        let mut payload = self.get_payload()?;
        payload.user_token = Some(user_token);
        self.set_payload(&payload)
    }

    pub fn get_sisu_token(&self) -> Result<Option<SisuTokenData>, String> {
        let payload = self.get_payload()?;
        Ok(payload.sisu_token)
    }

    pub fn set_sisu_token(&self, sisu_token: SisuTokenData) -> Result<(), String> {
        let mut payload = self.get_payload()?;
        payload.sisu_token = Some(sisu_token);
        self.set_payload(&payload)
    }

    pub fn get_jwt_private_jwk(&self) -> Result<Option<serde_json::Value>, String> {
        let payload = self.get_payload()?;
        Ok(payload.jwt_keys.and_then(|k| k.private_jwk))
    }

    pub fn set_jwt_private_jwk(&self, private_jwk: serde_json::Value) -> Result<(), String> {
        let mut payload = self.get_payload()?;
        payload.jwt_keys = Some(JwtKeysPayload {
            private_jwk: Some(private_jwk),
        });

        let store = self.open_write_store()?;
        let val = serde_json::to_value(payload).map_err(|e| e.to_string())?;
        store.store().set("auth.tokens.core", val);
        store.save()?;
        Ok(())
    }

    pub fn get_stream_tokens(&self) -> Result<Option<serde_json::Value>, String> {
        let store = self.open_read_store()?;
        Ok(store.store().get("auth.tokens.stream"))
    }

    pub fn set_stream_tokens(&self, val: serde_json::Value) -> Result<(), String> {
        let store = self.open_write_store()?;
        store.store().set("auth.tokens.stream", val);
        store.save()?;
        Ok(())
    }

    pub fn get_web_token(&self) -> Result<Option<serde_json::Value>, String> {
        let store = self.open_read_store()?;
        Ok(store.store().get("auth.tokens.web"))
    }

    pub fn set_web_token(&self, val: serde_json::Value) -> Result<(), String> {
        let store = self.open_write_store()?;
        store.store().set("auth.tokens.web", val);
        store.save()?;
        Ok(())
    }

    pub fn has_valid_auth_tokens(&self) -> bool {
        let payload = self.get_payload().unwrap_or(CoreTokenPayload::default());
        payload.user_token.is_some() && payload.sisu_token.is_some()
    }

    pub fn get_token_update_time(&self) -> u64 {
        let payload = self.get_payload().unwrap_or(CoreTokenPayload::default());
        payload.token_update_time.unwrap_or(0)
    }

    pub fn clear_all_tokens(&self) -> Result<(), String> {
        let store = self.open_write_store()?;
        store.store().delete("auth.tokens.core");
        store.store().delete("auth.tokens.stream");
        store.store().delete("auth.tokens.web");
        store.save()?;
        Ok(())
    }

    pub fn clear_ephemeral_tokens(&self) -> Result<(), String> {
        let store = self.open_write_store()?;
        store.store().delete("auth.tokens.stream");
        store.store().delete("auth.tokens.web");
        store.save()?;
        Ok(())
    }
}
