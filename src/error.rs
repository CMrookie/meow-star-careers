//! 统一错误处理：
//! - `AppError`：全应用唯一错误类型（thiserror），实现 actix `ResponseError`；
//! - 所有处理器返回 `ApiResult<T>`，任何错误在此处统一转成 JSON 响应并记录日志；
//! - `ErrorResponse`：统一的错误响应体，同时也作为 OpenAPI 组件导出。

use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse, ResponseError};
use serde::Serialize;
use utoipa::ToSchema;

/// 全局便捷别名：处理器/仓储层统一返回该结果类型
pub type ApiResult<T> = Result<T, AppError>;

/// 统一 JSON 错误响应体（code 为机器可读错误码，message 为人类可读描述）
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(description = "统一错误响应")]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
}

/// 应用统一错误类型
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)] // Unauthorized / Forbidden / Internal 为预留错误位，供后续鉴权等场景使用
pub enum AppError {
    /// 400 请求参数/请求体不合法
    #[error("{0}")]
    BadRequest(String),
    /// 401 未认证
    #[error("{0}")]
    Unauthorized(String),
    /// 403 无权限
    #[error("{0}")]
    Forbidden(String),
    /// 404 资源不存在
    #[error("{0}")]
    NotFound(String),
    /// 409 冲突（如唯一约束）
    #[error("{0}")]
    Conflict(String),
    /// 503 服务暂不可用（如依赖故障）
    #[error("{0}")]
    ServiceUnavailable(String),
    /// 429 触发频率限制（如登录连续失败）
    #[error("{0}")]
    TooManyRequests(String),
    /// 500 内部错误（不向客户端泄露细节）
    #[error("internal error: {0}")]
    Internal(String),
    /// 500 数据库错误（细节仅记录日志）
    #[error("database error: {0}")]
    Database(sqlx::Error),
}

impl AppError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    #[allow(dead_code)] // 预留：鉴权场景使用
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::Unauthorized(message.into())
    }

    #[allow(dead_code)] // 预留：鉴权场景使用
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Forbidden(message.into())
    }

    /// 形如 `AppError::not_found(format!("user `{id}`"))` -> “user `xxx` not found”
    pub fn not_found(resource: impl std::fmt::Display) -> Self {
        Self::NotFound(format!("{resource} not found"))
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self::ServiceUnavailable(message.into())
    }

    #[allow(dead_code)] // 预留：通用内部错误场景使用
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    /// 机器可读错误码
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::ServiceUnavailable(_) => "service_unavailable",
            Self::TooManyRequests(_) => "too_many_requests",
            Self::Internal(_) | Self::Database(_) => "internal_error",
        }
    }

    /// 面向客户端的安全消息：5xx 一律脱敏，避免泄露内部细节
    fn client_message(&self) -> String {
        match self.status_code() {
            StatusCode::INTERNAL_SERVER_ERROR => "internal server error".to_string(),
            StatusCode::SERVICE_UNAVAILABLE => "service unavailable".to_string(),
            _ => self.to_string(),
        }
    }
}

/// sqlx 错误 -> AppError：唯一约束冲突转为 409，其余作为数据库错误（500）
impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
                Self::conflict("a record with the same unique key already exists")
            }
            sqlx::Error::RowNotFound => Self::not_found("record"),
            other => Self::Database(other),
        }
    }
}

impl ResponseError for AppError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal(_) | Self::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// 唯一的“出网”出口：集中记录日志并输出统一 JSON 错误体
    fn error_response(&self) -> HttpResponse {
        let status = self.status_code();

        if status.is_server_error() {
            tracing::error!(status = status.as_u16(), code = %self.code(), error = %self, "request failed");
        } else {
            tracing::warn!(status = status.as_u16(), code = %self.code(), error = %self, "request rejected");
        }

        HttpResponse::build(status).json(ErrorResponse {
            code: self.code().to_string(),
            message: self.client_message(),
        })
    }
}

/// web::Json 提取失败（非法 JSON / 字段类型错误 / 过大等）统一转 400，
/// 从而与业务校验走同一错误通道。
pub fn json_error_handler(err: actix_web::Error, _req: &HttpRequest) -> actix_web::Error {
    tracing::debug!(error = %err, "json payload rejected");
    AppError::bad_request("malformed or invalid JSON request body").into()
}
