use tauri::Manager;

pub fn build_external_link_patch_script() -> &'static str {
    r#"
      (() => {
        if (window.__XBXRC_EXTERNAL_LINK_PATCHED__) return;
        window.__XBXRC_EXTERNAL_LINK_PATCHED__ = true;

        const invoke = window.__TAURI_INTERNALS__?.invoke;
        const openExternal = (url) => {
          if (!invoke || typeof url !== 'string' || url.trim() === '') return;
          invoke('rpc_invoke', {
            payload: {
              namespace: 'system',
              method: 'openExternal',
              params: { url }
            }
          }).catch(() => {});
        };

        const originalOpen = window.open;
        window.open = function(url, target, features) {
          if (typeof url === 'string' && /^https?:\/\//i.test(url)) {
            openExternal(url);
            return null;
          }
          if (typeof originalOpen === 'function') {
            return originalOpen.call(window, url, target, features);
          }
          return null;
        };

        document.addEventListener('click', (event) => {
          const el = event.target instanceof Element ? event.target.closest('a[target=\"_blank\"]') : null;
          if (!el) return;
          const href = el.getAttribute('href');
          if (!href || !/^https?:\/\//i.test(href)) return;
          event.preventDefault();
          openExternal(href);
        }, true);
      })();
    "#
}

#[cfg(target_os = "macos")]
pub fn handle_macos_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    use crate::mods::gamepad::input_gate::{
        clear_shell_main_window_focus_from_shell_action, sync_gamepad_input_gate,
    };
    use crate::AppState;
    use std::sync::atomic::Ordering;

    if window.label() == "main" {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            let app_state = window.app_handle().state::<AppState>();
            if !app_state.is_quitting.load(Ordering::Relaxed) {
                // 对齐 Electron：macOS 关闭窗口时仅隐藏，不退出进程。
                api.prevent_close();
                clear_shell_main_window_focus_from_shell_action();
                sync_gamepad_input_gate(&window.app_handle());
                let _ = window.hide();
            }
        }
    }
}
