//! 入口：初始化日志 -> 加载配置 -> 连接数据库并迁移 -> 启动 WebSocket Hub -> 启动 HTTP 服务器。
//! 模块分层：config(配置) / logging(日志) / db(连接池+迁移) / auth(鉴权提取器)
//!           / security(哈希与令牌) / ws(WebSocket 实时通道) / state(共享状态)
//!           / models(模型) / repositories(数据访问) / handlers(HTTP)
//!           / error(统一错误) / openapi(文档)

mod app;
mod auth;
mod config;
mod db;
mod error;
mod handlers;
mod limiter;
mod logging;
mod models;
mod openapi;
mod repositories;
mod security;
mod state;
#[cfg(test)]
mod tests;
mod ws;

use actix_web::{web, HttpServer};
use tracing_actix_web::TracingLogger;

use crate::state::AppState;

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    logging::init();

    let cfg = config::Config::from_env()?;
    let pool = db::connect(&cfg).await?;
    db::migrate(&pool).await?;

    // 平台管理员引导（可选）：配置了 ADMIN_PHONE / ADMIN_PASSWORD 时幂等创建 admin 账号，
    // 否则投诉审核没有可用的账号（admin 不可自助注册）。
    if let (Some(phone), Some(password)) = (cfg.admin_phone.as_deref(), cfg.admin_password.as_deref())
    {
        match repositories::auth::ensure_admin(&pool, phone, &cfg.admin_name, password).await? {
            repositories::auth::AdminBootstrap::Created => {
                tracing::info!(admin_phone = %phone, "已引导创建平台管理员账号")
            }
            repositories::auth::AdminBootstrap::AlreadyExists => {
                tracing::info!(admin_phone = %phone, "平台管理员账号已存在（密码保持不变）")
            }
        }
    }

    // WebSocket 在线会话中枢（所有 worker 共享）
    let hub = std::sync::Arc::new(ws::RealtimeHub::default());

    let state = web::Data::new(AppState::new(pool, hub));
    let addr = cfg.bind_addr()?;
    let docs_url = format!("http://{addr}/swagger-ui/");

    tracing::info!(%addr, %docs_url, "API 服务启动");
    HttpServer::new(move || {
        app::create_app(web::Data::clone(&state))
            .wrap(TracingLogger::default())
            .wrap(app::build_cors(&cfg.cors_origins))
    })
    .bind(addr)?
    .run()
    .await?;

    Ok(())
}
