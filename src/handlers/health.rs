//! 系统级端点：服务索引与健康检查。

use actix_web::{web, HttpResponse};
use serde_json::json;

use crate::error::{ApiResult, AppError};
use crate::state::AppState;

/// 服务索引：方便快速定位文档与健康检查
#[utoipa::path(
    get,
    path = "/",
    tag = "system",
    responses(
        (status = 200, description = "服务信息"),
    )
)]
pub async fn index() -> ApiResult<HttpResponse> {
    Ok(HttpResponse::Ok().json(json!({
        "service": env!("CARGO_PKG_NAME"),
        "version": env!("CARGO_PKG_VERSION"),
        "docs": "/swagger-ui/",
        "openapi": "/api-docs/openapi.json",
        "health": "/healthz",
    })))
}

/// 健康检查：对数据库做一次探活
#[utoipa::path(
    get,
    path = "/healthz",
    tag = "system",
    responses(
        (status = 200, description = "服务与数据库均正常"),
        (status = 503, description = "数据库不可用", body = crate::error::ErrorResponse),
    )
)]
pub async fn health_check(state: web::Data<AppState>) -> ApiResult<HttpResponse> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.pool)
        .await
        .map_err(|err| {
            tracing::error!(error = %err, "health check: database ping failed");
            AppError::service_unavailable("database unreachable")
        })?;

    Ok(HttpResponse::Ok().json(json!({ "status": "ok", "database": "up" })))
}
