//! 跨请求共享的应用状态（通过 actix `web::Data<AppState>` 注入）。

use std::sync::Arc;

use sqlx::PgPool;

use crate::limiter::LoginLimiter;
use crate::ws::RealtimeHub;

#[derive(Clone)]
pub struct AppState {
    /// PostgreSQL 连接池
    pub pool: PgPool,
    /// WebSocket 实时转发中枢（在线连接表）
    pub hub: Arc<RealtimeHub>,
    /// 登录失败限流
    pub login_limiter: LoginLimiter,
}

impl AppState {
    pub fn new(pool: PgPool, hub: Arc<RealtimeHub>) -> Self {
        Self {
            pool,
            hub,
            login_limiter: LoginLimiter::default(),
        }
    }
}
