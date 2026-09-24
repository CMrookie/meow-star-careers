//! HTTP 处理器层：薄封装，负责参数校验与 HTTP 语义，业务/数据逻辑下沉到仓储层。

pub mod applications;
pub mod auth;
pub mod chat;
pub mod complaints;
pub mod companies;
pub mod health;
pub mod interviews;
pub mod jobs;
pub mod push;
pub mod reviewers;
pub mod resumes;
pub mod users;

use actix_web::HttpResponse;

use crate::error::{ApiResult, AppError};

/// 兜底 404：未匹配任何路由时也返回统一错误 JSON
pub async fn not_found_route() -> ApiResult<HttpResponse> {
    Err(AppError::not_found("route"))
}
