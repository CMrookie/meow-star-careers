//! 设备推送令牌接口（预留）：客户端登录后可注册/注销
//! POST   /api/v1/push-tokens            {platform, token}
//! POST   /api/v1/push-tokens/remove     {platform, token}

use actix_web::{web, HttpResponse};
use serde::Deserialize;

use crate::auth::AuthenticatedUser;
use crate::error::{ApiResult, AppError};
use crate::repositories::push as push_repo;
use crate::state::AppState;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushTokenPayload {
    /// android / ios / web
    pub platform: String,
    pub token: String,
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/push-tokens")
            .route("", web::post().to(register_token))
            .route("/remove", web::post().to(remove_token)),
    );
}

fn validate(payload: &PushTokenPayload) -> Result<(), AppError> {
    if !["android", "ios", "web"].contains(&payload.platform.as_str()) {
        return Err(AppError::bad_request("platform 取值: android / ios / web"));
    }
    let t = payload.token.trim();
    if t.is_empty() || t.len() > 512 {
        return Err(AppError::bad_request("token 不能为空且不超过 512 字符"));
    }
    Ok(())
}

pub async fn register_token(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    payload: web::Json<PushTokenPayload>,
) -> ApiResult<HttpResponse> {
    let p = payload.into_inner();
    validate(&p)?;
    push_repo::register(&state.pool, &auth.user_id, &p.platform, p.token.trim()).await?;
    Ok(HttpResponse::NoContent().finish())
}

pub async fn remove_token(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    payload: web::Json<PushTokenPayload>,
) -> ApiResult<HttpResponse> {
    let p = payload.into_inner();
    validate(&p)?;
    push_repo::remove(&state.pool, &auth.user_id, &p.platform, p.token.trim()).await?;
    Ok(HttpResponse::NoContent().finish())
}
