//! 认证端点：注册（求职者/招聘者+企业）/ 登录 / 登出 / 当前用户。
//! 账号体系为「手机号 + 密码」；历史邮箱账号不受影响（users.email 可空）。

use actix_web::{web, HttpResponse};

use crate::auth::AuthenticatedUser;
use crate::error::{ApiResult, AppError, ErrorResponse};
use crate::models::auth::{AuthResponse, LoginRequest, RegisterRequest, Role};
use crate::models::user::User;
use crate::repositories;
use crate::repositories::auth as auth_repo;
use crate::state::AppState;
use crate::security;

/// 注册 /auth 资源路由（父 scope 为 /api/v1）
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/auth")
            .route("/register", web::post().to(register))
            .route("/login", web::post().to(login))
            .route("/logout", web::post().to(logout))
            .route("/me", web::get().to(me)),
    );
}

fn validate_register(payload: &RegisterRequest) -> Result<(), AppError> {
    // 管理员只能由服务端引导创建：注册接口显式拒绝，避免通过公开接口提权
    if payload.role == Role::Admin {
        return Err(AppError::bad_request(
            "admin 账号不可自助注册（由服务端 ADMIN_PHONE / ADMIN_PASSWORD 引导创建）",
        ));
    }
    let phone = payload.phone.trim();
    if !security::is_valid_cn_phone(phone) {
        return Err(AppError::bad_request("手机号必须为 11 位数字（1 开头）"));
    }
    let name = payload.name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(AppError::bad_request("name 必填且不超过 64 字符"));
    }
    if !(8..=128).contains(&payload.password.len()) {
        return Err(AppError::bad_request("password 长度须在 8-128 之间"));
    }
    if payload.role == Role::Recruiter {
        // 注意：这里必须用 400 而不是 expect —— 注册是公开接口，缺 company 的请求
        // 以前会 panic（500），属于可被构造触发的缺陷（回归测试已覆盖）。
        let Some(company) = payload.company.as_ref() else {
            return Err(AppError::bad_request("招聘者注册必须提供 company"));
        };
        if company.name.trim().is_empty() || company.name.trim().chars().count() > 128 {
            return Err(AppError::bad_request("招聘者注册必须提供 company.name（≤128 字符）"));
        }
        if company.industry.as_ref().is_some_and(|v| v.trim().chars().count() > 64)
            || company.location.as_ref().is_some_and(|v| v.trim().chars().count() > 128)
            || company.description.as_ref().is_some_and(|v| v.trim().chars().count() > 2000)
            || company.website.as_ref().is_some_and(|v| v.trim().chars().count() > 200)
            || company.logo_url.as_ref().is_some_and(|v| v.trim().chars().count() > 500)
        {
            return Err(AppError::bad_request("企业字段超长（industry≤64 / location≤128 / description≤2000 / website≤200 / logoUrl≤500）"));
        }
    }
    Ok(())
}

/// 注册：求职者，或招聘者（同事务创建企业）并签发令牌
#[utoipa::path(
    post,
    path = "/auth/register",
    tag = "auth",
    responses(
        (status = 201, description = "注册成功，返回令牌与用户", body = AuthResponse),
        (status = 400, description = "参数不合法（手机号/密码/招聘者缺 company.name 等）", body = ErrorResponse),
        (status = 409, description = "手机号已被注册", body = ErrorResponse),
    )
)]
pub async fn register(
    state: web::Data<AppState>,
    payload: web::Json<RegisterRequest>,
) -> ApiResult<HttpResponse> {
    let payload = payload.into_inner();
    validate_register(&payload)?;

    let password_hash = security::hash_password(&payload.password)
        .map_err(|err| AppError::internal(err.to_string()))?;
    let phone = payload.phone.trim().to_string();
    let name = payload.name.trim().to_string();

    let user = match payload.role {
        Role::Recruiter => {
            let company = payload
                .company
                .ok_or_else(|| AppError::bad_request("招聘者注册必须提供 company"))?;
            auth_repo::register_recruiter(&state.pool, &phone, &name, &password_hash, &company)
                .await?
                .0
        }
        Role::Seeker => auth_repo::register_seeker(&state.pool, &phone, &name, &password_hash).await?,
        // validate_register 已拒绝；此处仅保证 match 穷尽，避免将来漏改
        Role::Admin => {
            return Err(AppError::bad_request(
                "admin 账号不可自助注册（由服务端 ADMIN_PHONE / ADMIN_PASSWORD 引导创建）",
            ));
        }
    };

    let token = auth_repo::issue_token(&state.pool, user.id).await?;
    Ok(HttpResponse::Created().json(AuthResponse { token, user }))
}

/// 登录：校验手机号密码并签发新令牌（内置失败限流：5 次/15 分钟 -> 429）
#[utoipa::path(
    post,
    path = "/auth/login",
    tag = "auth",
    responses(
        (status = 200, description = "登录成功，返回令牌与用户", body = AuthResponse),
        (status = 400, description = "参数不合法", body = ErrorResponse),
        (status = 401, description = "手机号或密码错误", body = ErrorResponse),
        (status = 403, description = "账号已禁用", body = ErrorResponse),
        (status = 429, description = "失败次数过多，请稍后再试", body = ErrorResponse),
    )
)]
pub async fn login(
    state: web::Data<AppState>,
    req: actix_web::HttpRequest,
    payload: web::Json<LoginRequest>,
) -> ApiResult<HttpResponse> {
    let payload = payload.into_inner();
    let phone = payload.phone.trim();
    if !security::is_valid_cn_phone(phone) || payload.password.is_empty() || payload.password.len() > 128 {
        return Err(AppError::bad_request("手机号（11 位，1 开头）与 password 必填且长度合法"));
    }

    // 防爆破：按「手机号|IP」限流
    let ip = req
        .peer_addr()
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let limiter_key = format!("{phone}|{ip}");
    if state.login_limiter.blocked(&limiter_key) {
        return Err(AppError::TooManyRequests(
            "登录失败次数过多，请 15 分钟后再试".to_string(),
        ));
    }

    let fail = |code: &'static str, message: &str| -> AppError {
        state.login_limiter.record_failure(&limiter_key);
        match code {
            "forbidden" => AppError::forbidden(message),
            _ => AppError::unauthorized(message),
        }
    };

    let stored = match auth_repo::find_by_phone(&state.pool, phone).await {
        Ok(Some(stored)) => stored,
        Ok(None) => return Err(fail("unauthorized", "手机号或密码错误")),
        Err(err) => return Err(err),
    };

    if !stored.user.is_active {
        return Err(fail("forbidden", "账号已被禁用"));
    }
    let hash_ok = stored
        .password_hash
        .as_deref()
        .map(|hash| security::verify_password(&payload.password, hash))
        .unwrap_or(false);
    if !hash_ok {
        return Err(fail("unauthorized", "手机号或密码错误"));
    }

    state.login_limiter.clear(&limiter_key);
    let token = auth_repo::issue_token(&state.pool, stored.user.id).await?;
    Ok(HttpResponse::Ok().json(AuthResponse {
        token,
        user: stored.user,
    }))
}

/// 登出：撤销当前令牌
#[utoipa::path(
    post,
    path = "/auth/logout",
    tag = "auth",
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "登出成功"),
        (status = 401, description = "未认证或令牌无效", body = ErrorResponse),
    )
)]
pub async fn logout(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
) -> ApiResult<HttpResponse> {
    auth_repo::revoke_token(&state.pool, &auth.token).await?;
    Ok(HttpResponse::NoContent().finish())
}

/// 当前用户信息（可用于前端校验令牌有效性）
#[utoipa::path(
    get,
    path = "/auth/me",
    tag = "auth",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "当前用户", body = User),
        (status = 401, description = "未认证或令牌无效", body = ErrorResponse),
    )
)]
pub async fn me(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
) -> ApiResult<HttpResponse> {
    let user = repositories::user::get(&state.pool, &auth.user_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("user `{}`", auth.user_id)))?;
    Ok(HttpResponse::Ok().json(user))
}
