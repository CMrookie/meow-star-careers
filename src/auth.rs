//! 认证请求提取器：`AuthenticatedUser` 从 `Authorization: Bearer <token>`
//! 中解析并校验令牌（查库），失败时返回统一的 401 JSON。

use std::future::{ready, Future};
use std::pin::Pin;

use actix_web::http::header;
use actix_web::web::Data;
use actix_web::{FromRequest, HttpRequest};
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::repositories;
use crate::state::AppState;

/// 已认证用户（由提取器注入处理器）
#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub user_id: Uuid,
    /// 原始令牌（登出时使用）
    pub token: String,
}

impl FromRequest for AuthenticatedUser {
    type Error = AppError;
    type Future = Pin<Box<dyn Future<Output = ApiResult<Self>>>>;

    fn from_request(req: &HttpRequest, _payload: &mut actix_web::dev::Payload) -> Self::Future {
        // 1) 同步解析 Bearer 头
        let token = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let Some(token) = token else {
            return Box::pin(ready(Err(AppError::unauthorized("missing bearer token"))));
        };

        // 2) 获取连接池（理论上 app_data 总已注入）
        let pool = match req.app_data::<Data<AppState>>() {
            Some(state) => state.pool.clone(),
            None => {
                return Box::pin(ready(Err(AppError::internal("app state not available"))));
            }
        };

        // 3) 异步查库校验
        Box::pin(async move {
            let user = repositories::auth::find_user_by_token(&pool, &token)
                .await?
                .ok_or_else(|| AppError::unauthorized("invalid or expired token"))?;
            Ok(Self {
                user_id: user.id,
                token,
            })
        })
    }
}
