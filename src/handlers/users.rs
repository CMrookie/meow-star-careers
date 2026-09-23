//! 用户 CRUD 端点（全部需要 Bearer 鉴权）。每个处理器都标注 `#[utoipa::path]`
//! 用于生成 OpenAPI 文档，路由在 `configure` 中集中注册（挂在 /api/v1/users 下）。

use actix_web::{web, HttpResponse};
use uuid::Uuid;

use crate::auth::AuthenticatedUser;
use crate::error::{ApiResult, AppError, ErrorResponse};
use crate::models::user::{NewUser, UpdateUser, User};
use crate::repositories::user as user_repo;
use crate::state::AppState;

/// 注册 /users 资源路由（父 scope 为 /api/v1）
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/users")
            .route("", web::get().to(list_users))
            .route("", web::post().to(create_user))
            .route("/{id}", web::get().to(get_user))
            .route("/{id}", web::put().to(update_user))
            .route("/{id}", web::delete().to(delete_user)),
    );
}

/// 列出全部用户（仅平台管理员；含手机号等敏感字段）
#[utoipa::path(
    get,
    path = "/users",
    tag = "users",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "用户列表（仅平台管理员）", body = [User]),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非平台管理员", body = ErrorResponse),
    )
)]
pub async fn list_users(
    auth: AuthenticatedUser,
    state: web::Data<AppState>,
) -> ApiResult<HttpResponse> {
    user_repo::require_admin(&state.pool, &auth.user_id).await?;
    let users = user_repo::list(&state.pool).await?;
    Ok(HttpResponse::Ok().json(users))
}

/// 创建用户（仅平台管理员；管理端历史邮箱账号，无密码）
#[utoipa::path(
    post,
    path = "/users",
    tag = "users",
    security(("bearerAuth" = [])),
    responses(
        (status = 201, description = "创建成功", body = User),
        (status = 400, description = "请求体不合法", body = ErrorResponse),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非平台管理员", body = ErrorResponse),
        (status = 409, description = "邮箱已被占用", body = ErrorResponse),
    )
)]
pub async fn create_user(
    auth: AuthenticatedUser,
    state: web::Data<AppState>,
    payload: web::Json<NewUser>,
) -> ApiResult<HttpResponse> {
    user_repo::require_admin(&state.pool, &auth.user_id).await?;
    let payload = payload.into_inner();
    let email = payload.email.trim().to_lowercase();
    let name = payload.name.trim().to_string();

    if email.is_empty() || !email.contains('@') {
        return Err(AppError::bad_request("email 必须为合法邮箱地址"));
    }
    if name.is_empty() {
        return Err(AppError::bad_request("name 不能为空"));
    }

    let user = user_repo::create(&state.pool, &NewUser { email, name }).await?;
    Ok(HttpResponse::Created().json(user))
}

/// 按 id 获取用户（本人或平台管理员；其余一律 404，不泄露账号是否存在）
#[utoipa::path(
    get,
    path = "/users/{id}",
    tag = "users",
    security(("bearerAuth" = [])),
    params(
        ("id" = Uuid, Path, description = "用户 ID"),
    ),
    responses(
        (status = 200, description = "用户详情（本人或平台管理员）", body = User),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 404, description = "用户不存在，或无权查看（非本人且非管理员）", body = ErrorResponse),
    )
)]
pub async fn get_user(
    auth: AuthenticatedUser,
    state: web::Data<AppState>,
    path: web::Path<Uuid>,
) -> ApiResult<HttpResponse> {
    let id = path.into_inner();

    let me = user_repo::get(&state.pool, &auth.user_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("user `{}`", auth.user_id)))?;
    if me.role != "admin" && me.id != id {
        // 与简历一致：非本人且非管理员按 404 处理，不泄露账号存在性
        return Err(AppError::not_found(format!("user `{id}`")));
    }

    let user = user_repo::get(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("user `{id}`")))?;

    Ok(HttpResponse::Ok().json(user))
}

/// 更新用户（仅本人或平台管理员；仅更新请求体中提供的字段）
#[utoipa::path(
    put,
    path = "/users/{id}",
    tag = "users",
    security(("bearerAuth" = [])),
    params(
        ("id" = Uuid, Path, description = "用户 ID"),
    ),
    responses(
        (status = 200, description = "更新成功", body = User),
        (status = 400, description = "请求体不合法", body = ErrorResponse),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非本人且非平台管理员", body = ErrorResponse),
        (status = 404, description = "用户不存在", body = ErrorResponse),
    )
)]
pub async fn update_user(
    auth: AuthenticatedUser,
    state: web::Data<AppState>,
    path: web::Path<Uuid>,
    payload: web::Json<UpdateUser>,
) -> ApiResult<HttpResponse> {
    let id = path.into_inner();
    let mut payload = payload.into_inner();

    // 仅本人或平台管理员可修改（管理端禁用/改名走同一入口）
    let me = user_repo::get(&state.pool, &auth.user_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("user `{}`", auth.user_id)))?;
    if me.role != "admin" && me.id != id {
        return Err(AppError::forbidden("仅本人或平台管理员可修改用户资料"));
    }

    if payload.name.is_none() && payload.is_active.is_none() {
        return Err(AppError::bad_request("至少提供 name 或 isActive 之一"));
    }
    if let Some(name) = payload.name.as_mut() {
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() {
            return Err(AppError::bad_request("name 不能为空"));
        }
        *name = trimmed;
    }

    let user = user_repo::update(&state.pool, &id, &payload)
        .await?
        .ok_or_else(|| AppError::not_found(format!("user `{id}`")))?;

    Ok(HttpResponse::Ok().json(user))
}

/// 删除用户（仅平台管理员；级联清理其令牌/简历/投递等）
#[utoipa::path(
    delete,
    path = "/users/{id}",
    tag = "users",
    security(("bearerAuth" = [])),
    params(
        ("id" = Uuid, Path, description = "用户 ID"),
    ),
    responses(
        (status = 204, description = "删除成功"),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非平台管理员", body = ErrorResponse),
        (status = 404, description = "用户不存在", body = ErrorResponse),
    )
)]
pub async fn delete_user(
    auth: AuthenticatedUser,
    state: web::Data<AppState>,
    path: web::Path<Uuid>,
) -> ApiResult<HttpResponse> {
    user_repo::require_admin(&state.pool, &auth.user_id).await?;
    let id = path.into_inner();

    let deleted = user_repo::delete(&state.pool, &id).await?;
    if !deleted {
        return Err(AppError::not_found(format!("user `{id}`")));
    }

    Ok(HttpResponse::NoContent().finish())
}
