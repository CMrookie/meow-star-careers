//! 日志初始化：基于 tracing / tracing-subscriber，级别由 RUST_LOG 控制。

use std::io::IsTerminal;

use tracing_subscriber::EnvFilter;

/// 初始化全局日志订阅器。应用日志使用 `tracing::info!/warn!/error!`，
/// 每个请求的访问日志由 `tracing_actix_web::TracingLogger` 中间件产生。
pub fn init() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,tracing_actix_web=info,sqlx=warn"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(std::io::stdout().is_terminal())
        .init();
}
