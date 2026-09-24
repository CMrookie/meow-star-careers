//! 审核专用账号管理（**仅平台管理员**）：
//!
//! - `POST   /api/v1/reviewers`                 创建审核账号（手机号 + 初始密码）
//! - `GET    /api/v1/reviewers`                 审核账号列表
//! - `POST   /api/v1/reviewers/{id}/active`     启用 / 禁用（禁用即踢下线）
//! - `POST   /api/v1/reviewers/{id}/password`   重置密码（同时撤销该账号全部会话）
//! - `DELETE /api/v1/reviewers/{id}`            删除
//!
//! 「多个审核同时在线」不需要额外开关：令牌按账号、多条并存（登录不互相踢），
//! 因此多个审核账号可同时登录并行审核，审核留痕见 `complaints.reviewed_by(_name)`。
//! 本模块只操作 `role=reviewer` 的账号，其它角色一律 404，避免误伤普通用户。

use actix_web::{web, HttpResponse};
use uuid::Uuid;

use crate::auth::AuthenticatedUser;
use crate::error::{ApiResult, AppError, ErrorResponse};
use crate::models::user::{NewReviewer, ResetPassword, SetActiveRequest, UpdateUser, User};
use crate::repositories::{auth as auth_repo, user as user_repo};
use crate::security;
use crate::state::AppState;

/// 审核账号的角色取值（与 0015_reviewer_role.sql 的 CHECK 一致）
pub const REVIEWER_ROLE: &str = "reviewer";

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/reviewers")
            .route("", web::get().to(list_reviewers))
            .route("", web::post().to(create_reviewer))
            .route("/{id}/active", web::post().to(set_reviewer_active))
            .route("/{id}/password", web::post().to(reset_reviewer_password))
            .route("/{id}", web::delete().to(delete_reviewer)),
    );
}

fn validate_phone(phone: &str) -> Result<(), AppError> {
    if !security::is_valid_cn_phone(phone.trim()) {
        return Err(AppError::bad_request("phone 必须为 11 位手机号（1 开头）"));
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), AppError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(AppError::bad_request("name 必填且不超过 64 字符"));
    }
    Ok(())
}

fn validate_password(password: &str) -> Result<(), AppError> {
    if !(8..=128).contains(&password.len()) {
        return Err(AppError::bad_request("password 长度须在 8-128 之间"));
    }
    Ok(())
}

/// 取出目标账号并确认它确实是审核账号（否则 404：既防探测也防误操作其它角色）
async fn reviewer_target(state: &AppState, id: &Uuid) -> ApiResult<User> {
    let target = user_repo::get(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("reviewer `{id}`")))?;
    if target.role != REVIEWER_ROLE {
        return Err(AppError::not_found(format!("reviewer `{id}`")));
    }
    Ok(target)
}

/// 审核账号列表
#[utoipa::path(
    get,
    path = "/reviewers",
    tag = "reviewer",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "审核账号列表（仅平台管理员）", body = [User]),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非平台管理员", body = ErrorResponse),
    )
)]
pub async fn list_reviewers(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
) -> ApiResult<HttpResponse> {
    user_repo::require_admin(&state.pool, &auth.user_id).await?;
    let reviewers = user_repo::list_by_role(&state.pool, REVIEWER_ROLE).await?;
    Ok(HttpResponse::Ok().json(reviewers))
}

/// 创建审核账号（平台管理员）
#[utoipa::path(
    post,
    path = "/reviewers",
    tag = "reviewer",
    security(("bearerAuth" = [])),
    responses(
        (status = 201, description = "创建成功（role=reviewer，可直接登录审核）", body = User),
        (status = 400, description = "手机号/姓名/密码不合法", body = ErrorResponse),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非平台管理员", body = ErrorResponse),
        (status = 409, description = "手机号已被占用", body = ErrorResponse),
    )
)]
pub async fn create_reviewer(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    payload: web::Json<NewReviewer>,
) -> ApiResult<HttpResponse> {
    let admin = user_repo::require_admin(&state.pool, &auth.user_id).await?;
    let payload = payload.into_inner();
    validate_phone(&payload.phone)?;
    validate_name(&payload.name)?;
    validate_password(&payload.password)?;

    let password_hash = security::hash_password(&payload.password)
        .map_err(|err| AppError::internal(err.to_string()))?;
    let reviewer = auth_repo::create_reviewer(
        &state.pool,
        payload.phone.trim(),
        payload.name.trim(),
        &password_hash,
    )
    .await?;

    tracing::info!(admin_id = %admin.id, reviewer_id = %reviewer.id, "已创建审核账号");
    Ok(HttpResponse::Created().json(reviewer))
}

/// 启用 / 禁用审核账号（禁用会立即撤销其全部会话）
#[utoipa::path(
    post,
    path = "/reviewers/{id}/active",
    tag = "reviewer",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "审核账号 ID")),
    responses(
        (status = 200, description = "更新后的审核账号", body = User),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非平台管理员", body = ErrorResponse),
        (status = 404, description = "审核账号不存在", body = ErrorResponse),
    )
)]
pub async fn set_reviewer_active(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
    payload: web::Json<SetActiveRequest>,
) -> ApiResult<HttpResponse> {
    let admin = user_repo::require_admin(&state.pool, &auth.user_id).await?;
    let id = path.into_inner();
    let target = reviewer_target(&state, &id).await?;
    let active = payload.into_inner().is_active;

    let updated = user_repo::update(
        &state.pool,
        &id,
        &UpdateUser {
            name: None,
            is_active: Some(active),
        },
    )
    .await?
    .ok_or_else(|| AppError::not_found(format!("reviewer `{id}`")))?;

    let revoked = if active {
        0
    } else {
        // 禁用即刻踢下线：与「禁用账号令牌立即失效」的约定保持一致
        auth_repo::revoke_all_tokens(&state.pool, &id).await?
    };
    tracing::info!(
        admin_id = %admin.id, reviewer_id = %id, target_name = %target.name,
        is_active = active, revoked_sessions = revoked, "审核账号状态已更新"
    );
    Ok(HttpResponse::Ok().json(updated))
}

/// 重置审核账号密码（会撤销该账号全部会话，需重新登录）
#[utoipa::path(
    post,
    path = "/reviewers/{id}/password",
    tag = "reviewer",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "审核账号 ID")),
    responses(
        (status = 204, description = "已重置（该账号原有会话全部失效）"),
        (status = 400, description = "密码长度不合法", body = ErrorResponse),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非平台管理员", body = ErrorResponse),
        (status = 404, description = "审核账号不存在", body = ErrorResponse),
    )
)]
pub async fn reset_reviewer_password(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
    payload: web::Json<ResetPassword>,
) -> ApiResult<HttpResponse> {
    let admin = user_repo::require_admin(&state.pool, &auth.user_id).await?;
    let id = path.into_inner();
    let target = reviewer_target(&state, &id).await?;
    let payload = payload.into_inner();
    validate_password(&payload.password)?;

    let password_hash = security::hash_password(&payload.password)
        .map_err(|err| AppError::internal(err.to_string()))?;
    auth_repo::update_password(&state.pool, &id, &password_hash)
        .await?
        .then_some(())
        .ok_or_else(|| AppError::not_found(format!("reviewer `{id}`")))?;
    let revoked = auth_repo::revoke_all_tokens(&state.pool, &id).await?;

    tracing::info!(
        admin_id = %admin.id, reviewer_id = %id, target_name = %target.name,
        revoked_sessions = revoked, "审核账号密码已重置，旧会话全部失效"
    );
    Ok(HttpResponse::NoContent().finish())
}

/// 删除审核账号
#[utoipa::path(
    delete,
    path = "/reviewers/{id}",
    tag = "reviewer",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "审核账号 ID")),
    responses(
        (status = 204, description = "删除成功（其令牌等数据级联清理）"),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非平台管理员", body = ErrorResponse),
        (status = 404, description = "审核账号不存在", body = ErrorResponse),
    )
)]
pub async fn delete_reviewer(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
) -> ApiResult<HttpResponse> {
    let admin = user_repo::require_admin(&state.pool, &auth.user_id).await?;
    let id = path.into_inner();
    let target = reviewer_target(&state, &id).await?;

    user_repo::delete(&state.pool, &id).await?;
    tracing::info!(admin_id = %admin.id, reviewer_id = %id, target_name = %target.name, "审核账号已删除");
    Ok(HttpResponse::NoContent().finish())
}
