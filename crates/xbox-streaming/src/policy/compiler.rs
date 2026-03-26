use serde::{Deserialize, Serialize};

use crate::policy::config::Config;
use crate::policy::context::Context;
use crate::policy::plan::Plan;
use crate::policy::session::ResolvedSessionAccess;
use crate::policy::types::CompileError;

/// compiler 输入固定由“偏好配置 + 运行上下文”组成。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompilerInput {
    pub config: Config,
    pub context: Context,
}

/// compiler 首期只先固定输入输出契约，后续再补真实编译实现。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompilerOutput {
    pub plan: Plan,
}

/// 会话接入解析与 plan 编译拆开，避免把敏感 token 混进 public plan。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionAccessOutput {
    pub access: ResolvedSessionAccess,
}

/// 入口只做编排：先解会话接入，再分别编译 session/runtime/negotiation/input。
pub fn compile(input: CompilerInput) -> Result<CompilerOutput, CompileError> {
    let access = resolve_session_access(&input.config, &input.context)?;
    let runtime = crate::policy::runtime::compiler::compile_runtime(&input.config, &input.context)?;
    let session = crate::policy::session::compiler::compile_session(
        &input.config,
        &input.context,
        &access,
        runtime.mode,
    );
    let negotiation = crate::policy::negotiation::compiler::compile_negotiation(
        &input.config,
        &input.context,
        runtime.mode,
    );
    let input_plan =
        crate::policy::input::compiler::compile_input(&input.config, &input.context, &runtime)?;
    let render = crate::policy::render::compiler::compile_render(&input.config);

    Ok(CompilerOutput {
        plan: Plan {
            session,
            negotiation,
            input: input_plan,
            runtime,
            render,
        },
    })
}

/// 会话接入解析和 plan 编译拆开，便于 tauri 侧先复用 base_url/region 选择逻辑。
pub fn resolve_session_access(
    config: &Config,
    context: &Context,
) -> Result<ResolvedSessionAccess, CompileError> {
    crate::policy::session::compiler::resolve_session_access(config, context)
}

/// 单独暴露 `SessionAccessOutput`，让 tauri 侧可以只复用会话入口解析逻辑。
pub fn resolve_session_access_output(
    config: &Config,
    context: &Context,
) -> Result<SessionAccessOutput, CompileError> {
    let access = resolve_session_access(config, context)?;
    Ok(SessionAccessOutput { access })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::config::Config;
    use crate::policy::context::Context;
    use crate::policy::input::InputMode;
    use crate::policy::input::InputPreference;
    use crate::policy::negotiation::CodecPreference;
    use crate::policy::runtime::RuntimeMode;
    use crate::policy::runtime::RuntimePreference;
    use crate::policy::session::ResolutionPreference;
    use crate::policy::types::{HostAddr, Region, Switch, Target, TurnServer, TurnSource};

    fn sample_region(name: &str, base_uri: &str, is_default: bool) -> Region {
        Region {
            name: name.to_string(),
            base_uri: base_uri.to_string(),
            is_default,
            short_name: None,
            display_name: None,
            continent: None,
        }
    }

    #[test]
    fn pick_region_prefers_force_region_ip() {
        let mut config = Config::default();
        config.session.force_region_ip = Some("10.0.0.1".to_string());
        let regions = vec![
            sample_region("WESTUS", "https://westus.example.com", true),
            sample_region("HOME", "https://10.0.0.1/stream", false),
        ];

        let selected = crate::policy::session::compiler::pick_region(&regions, &config).unwrap();
        assert_eq!(selected.name, "HOME");
    }

    #[test]
    fn runtime_auto_prefers_browser_then_rust_owned() {
        let config = Config::default();
        let context = Context::default();
        let runtime_plan =
            crate::policy::runtime::compiler::compile_runtime(&config, &context).unwrap();
        assert_eq!(runtime_plan.mode, RuntimeMode::WebRtcDirect);

        let mut fallback_context = Context::default();
        fallback_context.runtime.browser_webrtc = false;
        fallback_context.runtime.rust_owned = true;
        let runtime_plan =
            crate::policy::runtime::compiler::compile_runtime(&config, &fallback_context).unwrap();
        assert_eq!(runtime_plan.mode, RuntimeMode::RustOwned);
    }

    #[test]
    fn runtime_unavailable_returns_error() {
        let mut config = Config::default();
        config.runtime.mode = RuntimePreference::RustOwned;

        let context = Context::default();
        let error =
            crate::policy::runtime::compiler::compile_runtime(&config, &context).unwrap_err();
        assert_eq!(error, CompileError::RuntimeUnavailable);
    }

    #[test]
    fn codec_preference_maps_h264_profiles() {
        let low =
            crate::policy::negotiation::compiler::compile_codec(CodecPreference::H264Low).unwrap();
        assert_eq!(low.mime_type, "video/H264");
        assert_eq!(low.profiles, vec!["420".to_string()]);

        let normal =
            crate::policy::negotiation::compiler::compile_codec(CodecPreference::H264Normal)
                .unwrap();
        assert_eq!(normal.profiles, vec!["42e".to_string()]);

        let main =
            crate::policy::negotiation::compiler::compile_codec(CodecPreference::H264Main).unwrap();
        assert_eq!(main.profiles, vec!["4d".to_string()]);

        let high =
            crate::policy::negotiation::compiler::compile_codec(CodecPreference::H264High).unwrap();
        assert_eq!(high.profiles, vec!["4d".to_string()]);
    }

    #[test]
    fn auto_input_prefers_native_mkb_when_supported() {
        let mut config = Config::default();
        config.input.mode = InputPreference::Auto;
        config.input.native_mkb = Switch::On;

        let mut context = Context::default();
        context.input.has_mkb = true;
        context.runtime.native_mkb = true;
        let capabilities = crate::policy::input::compiler::interpret_input_capabilities(&context);

        let mode =
            crate::policy::input::compiler::resolve_input_mode(&config, &context, &capabilities)
                .unwrap();
        assert_eq!(mode, InputMode::NativeMkb);
    }

    #[test]
    fn explicit_native_mkb_errors_when_unavailable() {
        let mut config = Config::default();
        config.input.mode = InputPreference::NativeMkb;

        let context = Context::default();
        let capabilities = crate::policy::input::compiler::interpret_input_capabilities(&context);
        let error =
            crate::policy::input::compiler::resolve_input_mode(&config, &context, &capabilities)
                .unwrap_err();
        assert_eq!(error, CompileError::NativeMkbUnavailable);
    }

    #[test]
    fn input_config_capabilities_override_fallback_touch_fact() {
        let mut context = Context::default();
        context.input.has_touch = true;
        context.input.has_native_touch = true;
        context.input_capability.input_config_resolved = true;
        context.input_capability.input_config_supports_touch = false;
        context.input_capability.input_config_supports_native_touch = false;

        let effective = crate::policy::input::compiler::interpret_input_capabilities(&context);
        assert!(!effective.title_supports_touch);
        assert!(!effective.title_supports_native_touch);
    }

    #[test]
    fn compile_builds_home_plan_with_console_addrs_and_fallback_turn() {
        let mut config = Config::default();
        config.session.home_resolution = ResolutionPreference::P1080;
        config.runtime.mode = RuntimePreference::WebRtcDirect;
        config.runtime.home_fallback_turn = true;
        config.input.virtual_mkb = true;
        config.negotiation.video_codec = CodecPreference::H264Normal;

        let mut context = Context::default();
        context.target = Target::Home;
        context.target_id = "console-1".to_string();
        context.session.gs_token = Some("token".to_string());
        context.session.regions = vec![sample_region("HOME", "home.example.com", true)];
        context.remote_play.console_addrs.push(HostAddr {
            ip: "10.0.0.10".to_string(),
            port: 9002,
        });
        context.turn.fallback = Some(TurnServer {
            url: "turn:example.com".to_string(),
            username: "u".to_string(),
            credential: "c".to_string(),
        });

        let output = compile(CompilerInput { config, context }).unwrap();

        assert_eq!(output.plan.session.base_url, "https://home.example.com");
        assert!(output.plan.negotiation.inject_console_addrs);
        assert_eq!(output.plan.runtime.turn.source, TurnSource::Fallback);
        assert_eq!(output.plan.runtime.mode, RuntimeMode::WebRtcDirect);
    }

    #[test]
    fn compile_builds_cloud_plan_with_fallback_turn_when_enabled() {
        let mut config = Config::default();
        config.runtime.home_fallback_turn = true;

        let mut context = Context::default();
        context.target = Target::Cloud;
        context.session.gs_token = Some("token".to_string());
        context.session.regions = vec![sample_region("WESTUS", "https://westus.example.com", true)];
        context.turn.fallback = Some(TurnServer {
            url: "turn:example.com".to_string(),
            username: "u".to_string(),
            credential: "c".to_string(),
        });

        let output = compile(CompilerInput { config, context }).unwrap();

        assert_eq!(output.plan.runtime.turn.source, TurnSource::Fallback);
        assert_eq!(
            output
                .plan
                .runtime
                .turn
                .resolved
                .as_ref()
                .map(|turn| turn.url.as_str()),
            Some("turn:example.com")
        );
    }

    #[test]
    fn configuration_facts_override_remote_play_fallback_capability() {
        let mut config = Config::default();
        config.session.home_resolution = ResolutionPreference::P1080;

        let mut context = Context::default();
        context.target = Target::Home;
        context.session.gs_token = Some("token".to_string());
        context.session.regions = vec![sample_region("HOME", "home.example.com", true)];
        context.remote_play.configuration_resolved = true;
        context.remote_play.console_streaming_enabled = Some(false);
        context.remote_play.console_addrs.push(HostAddr {
            ip: "10.0.0.10".to_string(),
            port: 9002,
        });

        let output = compile(CompilerInput {
            config,
            context: context.clone(),
        })
        .unwrap();
        let projection =
            crate::policy::projection::project_session_capabilities(&context, &output.plan);

        assert_eq!(
            projection.effective_remote_play_capability_source,
            "configuration"
        );
        assert!(!projection.effective_remote_play_allows_streaming);
    }

    #[test]
    fn resolve_session_access_output_wraps_access() {
        let config = Config::default();
        let mut context = Context::default();
        context.session.gs_token = Some("token".to_string());
        context.session.regions = vec![sample_region("HOME", "home.example.com", true)];

        let output = resolve_session_access_output(&config, &context).unwrap();
        assert_eq!(output.access.gs_token, "token");
        assert_eq!(output.access.base_url, "https://home.example.com");
    }

    #[test]
    fn render_compile_trims_empty_video_format() {
        let mut config = Config::default();
        config.render.video_format = Some("   ".to_string());

        let render_plan = crate::policy::render::compiler::compile_render(&config);
        assert_eq!(render_plan.video_format, None);
    }
}
