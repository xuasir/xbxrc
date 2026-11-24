# reqwest / hyper-util 平台代理与 TLS 证据

本轮只读下载 crates.io 发布包，不执行其中代码；SHA-256 与当前 [Cargo.lock](/Users/guo.xu/Documents/code/games/xbxrc/Cargo.lock) 的 package checksum 一致。以下是依赖源码事实，不是手机实际网络路径或故障原因证明。

## 来源与完整性

| 依赖 | 发布包 SHA-256 | 关键源码 SHA-256 |
| --- | --- | --- |
| hyper-util 0.1.20 | 96547c2556ec9d12fb1578c4eaf448b04993e7fb79cbaad930a656880a6bdfa0 | `src/client/proxy/matcher.rs`：3869f863fbba4714ffd828d3dcd35e68fceeaf33169aa502e6b0d74a48f1fb1a |
| reqwest 0.13.2 | ab3f43e3283ab1488b624b44b0e988d0acea0b3214e694730a055cb6b2efa801 | `src/proxy.rs`：7d5d3ed0fe95b0c82f1081e0907f4691d53c929dbe0f2c6134dceaca304839ba；`src/async_impl/client.rs`：4bdbedf8f10460af26a099e5bf5648ae0a21a5993218ea2b7ba5bfd67d335e21 |

下载来源：`https://static.crates.io/crates/hyper-util/hyper-util-0.1.20.crate`、`https://static.crates.io/crates/reqwest/reqwest-0.13.2.crate`。外部源码路径不是仓库业务文件，保留以上固定版本、校验值与必要摘录供复现，不保留完整临时解包目录。

## 1. 环境变量与系统代理

`hyper-util 0.1.20 / src/client/proxy/matcher.rs`，从第 228 行开始的关键代码：

```rust
fn from_env() -> Self {
    Builder {
        is_cgi: std::env::var_os("REQUEST_METHOD").is_some(),
        all: get_first_env(&["ALL_PROXY", "all_proxy"]),
        http: get_first_env(&["HTTP_PROXY", "http_proxy"]),
        https: get_first_env(&["HTTPS_PROXY", "https_proxy"]),
        no: get_first_env(&["NO_PROXY", "no_proxy"]),
    }
}

fn from_system() -> Self {
    #[allow(unused_mut)]
    let mut builder = Self::from_env();

    #[cfg(all(feature = "client-proxy-system", target_os = "macos"))]
    mac::with_system(&mut builder);

    #[cfg(all(feature = "client-proxy-system", windows))]
    win::with_system(&mut builder);

    builder
}
```

该版本的系统配置读取只装配 macOS/Windows，**没有 iOS 分支**。macOS 模块从第 575 行开始，使用 SystemConfiguration/SCDynamicStore 读取 HTTP/HTTPS 设置；环境变量已有值时优先保留环境设置。

iOS 并不是“不支持任何代理”：显式 reqwest proxy 或上述环境变量仍可生效。准确结论是，启用 system-proxy feature 并不会自动读取 iOS 的 Wi-Fi 系统代理配置。TUN/VPN 属于更低层路由，不等同于 HTTP 系统代理。

## 2. reqwest 默认装配

`reqwest 0.13.2 / src/async_impl/client.rs`：第 305 行将 `auto_sys_proxy` 默认设为 true；第 410 行开始：

```rust
let mut proxies = config.proxies;
if config.auto_sys_proxy {
    proxies.push(ProxyMatcher::system());
}
```

`src/proxy.rs` 第 515 行开始的 `ProxyMatcher::system()` 使用 `matcher::Matcher::from_system()`。因此，这不是项目手动关闭 iOS 代理，而是默认依赖的平台分支能力不同。

该 client 默认 resolver 在第 420 行开始选择 `GaiResolver::new()`；项目没有为业务生产请求装配自定义 DNS resolver、强制 HTTP/1 或额外 proxy。测试的 no_proxy 不计入生产策略。

## 3. TLS 不应被误判为平台断层

同一 client 第 714 行优先使用已安装的 Rustls CryptoProvider，否则使用 reqwest 的推荐默认 provider；第 751 行开始，普通配置使用 `rustls_platform_verifier::Verifier::new(provider.clone())`。

两目标离线依赖树均包含 Rustls、HTTP/2、system-proxy、rustls-platform-verifier 与 Apple Security Framework。PC 额外出现 stream、rustls-no-provider 等特征，不代表其业务 API 改走另一套 HTTP 实现。

[PC 进程初始化](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/lib.rs:131) 明确选定 aws-lc-rs；iOS reqwest 使用自身默认装配。没有证据表明本次失败来自关闭证书校验、缺少 Rustls 或缺少 HTTP/2。两平台实际信任配置、DNS 结果与连接路径仍需手机/PC 同环境运行证据。
