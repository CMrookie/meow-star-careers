//! 投递端点：列表（求职者=我的投递 / 招聘者=企业收件箱）、详情、状态流转。

use actix_web::{web, HttpResponse};
use uuid::Uuid;

use crate::auth::AuthenticatedUser;
use crate::error::{ApiResult, AppError, ErrorResponse};
use crate::models::application::{
    APPLICATION_STATUSES, RECRUITER_TRANSITIONS, SEEKER_WITHDRAW, ApplicationQuery,
    UpdateApplicationStatus,
};
use crate::repositories::{application as app_repo, user as user_repo};
use crate::state::AppState;
use crate::ws;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/applications")
            .route("", web::get().to(list_applications))
            .route("/{id}", web::get().to(get_application))
            .route("/{id}/status", web::post().to(update_application_status)),
    );
}

fn ensure_status_valid(status: &str) -> Result<(), AppError> {
    if !APPLICATION_STATUSES.contains(&status) {
        return Err(AppError::bad_request(format!(
            "status 取值: {}",
            APPLICATION_STATUSES.join(" / ")
        )));
    }
    Ok(())
}

/// 投递列表：求职者看自己的；招聘者看本企业的
#[utoipa::path(
    get,
    path = "/applications",
    tag = "application",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "投递列表（角色相关）", body = crate::models::application::ApplicationPage),
        (status = 401, description = "未认证", body = ErrorResponse),
    )
)]
pub async fn list_applications(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    query: web::Query<ApplicationQuery>,
) -> ApiResult<HttpResponse> {
    let user = user_repo::get(&state.pool, &auth.user_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("user `{}`", auth.user_id)))?;
    let query = query.into_inner();

    let page = if user.role == "recruiter" {
        let company_id = user
            .company_id
            .ok_or_else(|| AppError::forbidden("招聘者账号尚未绑定企业"))?;
        app_repo::list_for_company(&state.pool, &company_id, &query).await?
    } else {
        app_repo::list_for_seeker(&state.pool, &auth.user_id, &query).await?
    };

    Ok(HttpResponse::Ok().json(page))
}

/// 投递详情（求职者本人或企业招聘者可看）
#[utoipa::path(
    get,
    path = "/applications/{id}",
    tag = "application",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "投递 ID")),
    responses(
        (status = 200, description = "投递详情", body = crate::models::application::ApplicationView),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "无权查看", body = ErrorResponse),
        (status = 404, description = "投递不存在", body = ErrorResponse),
    )
)]
pub async fn get_application(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
) -> ApiResult<HttpResponse> {
    let id = path.into_inner();
    let app = app_repo::get_view(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("application `{id}`")))?;

    let user = user_repo::get(&state.pool, &auth.user_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("user `{}`", auth.user_id)))?;

    let can_view = app.seeker_id == auth.user_id
        || (user.role == "recruiter"
            && user.company_id.map(|cid| cid == app.company_id).unwrap_or(false));
    if !can_view {
        return Err(AppError::forbidden("无权查看该投递"));
    }

    Ok(HttpResponse::Ok().json(app))
}

/// 更新投递状态：招聘者推进（viewed/interviewing/offered/rejected），求职者撤回（withdrawn）
#[utoipa::path(
    post,
    path = "/applications/{id}/status",
    tag = "application",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "投递 ID")),
    responses(
        (status = 200, description = "更新成功", body = crate::models::application::ApplicationView),
        (status = 400, description = "非法状态", body = ErrorResponse),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "无权操作或非法流转", body = ErrorResponse),
        (status = 404, description = "投递不存在", body = ErrorResponse),
    )
)]
pub async fn update_application_status(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
    payload: web::Json<UpdateApplicationStatus>,
) -> ApiResult<HttpResponse> {
    let id = path.into_inner();
    let status = payload.into_inner().status;
    ensure_status_valid(&status)?;

    let app = app_repo::get_view(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("application `{id}`")))?;
    let user = user_repo::get(&state.pool, &auth.user_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("user `{}`", auth.user_id)))?;

    if user.role == "recruiter" {
        let company_ok = user
            .company_id
            .map(|cid| cid == app.company_id)
            .unwrap_or(false);
        if !company_ok {
            return Err(AppError::forbidden("仅可操作本企业收到的投递"));
        }
        if !RECRUITER_TRANSITIONS.contains(&status.as_str()) {
            return Err(AppError::forbidden("招聘者可设置的状态: viewed / interviewing / offered / rejected"));
        }
    } else {
        // 求职者只能撤回自己的投递
        if app.seeker_id != auth.user_id {
            return Err(AppError::forbidden("无权操作该投递"));
        }
        if status != SEEKER_WITHDRAW {
            return Err(AppError::forbidden("求职者仅可撤回（withdrawn）"));
        }
    }

    let app = app_repo::update_status(&state.pool, &id, &status)
        .await?
        .ok_or_else(|| AppError::not_found(format!("application `{id}`")))?;
    // 实时推送：状态变化同步给求职者与本企业招聘者
    let recruiters = user_repo::recruiters_of_company(&state.pool, &app.company_id).await?;
    ws::notify_application(&state.hub, &app, &recruiters);
    Ok(HttpResponse::Ok().json(app))
}
