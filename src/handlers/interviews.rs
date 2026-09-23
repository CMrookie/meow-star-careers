//! 线上面试端点：发起（即时/预约）/ 我的列表 / 详情 / 开始 / 结束 / 取消。
//! 音视频信令（offer/answer/ICE）经现有 WebSocket 实时转发（见 ws.rs 的 "signal" 报文）。

use actix_web::{web, HttpResponse};
use uuid::Uuid;

use crate::auth::AuthenticatedUser;
use crate::error::{ApiResult, AppError, ErrorResponse};
use crate::models::application::ApplicationView;
use crate::models::interview::{InterviewView, NewInterview};
use crate::models::user::User;
use crate::repositories;
use crate::repositories::application as app_repo;
use crate::repositories::interview as interview_repo;
use crate::state::AppState;
use crate::ws;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/interviews")
            .route("", web::get().to(list_mine))
            .route("", web::post().to(create_interview))
            .route("/{id}", web::get().to(get_interview))
            .route("/{id}/start", web::post().to(start_interview))
            .route("/{id}/finish", web::post().to(finish_interview))
            .route("/{id}/cancel", web::post().to(cancel_interview)),
    );
}

/// 发起面试前：必须是招聘者，且投递属于本企业
async fn recruiter_of_application(
    state: &AppState,
    user_id: &Uuid,
    application_id: Uuid,
) -> ApiResult<(ApplicationView, User)> {
    let me = repositories::user::get(&state.pool, user_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("user `{user_id}`")))?;
    if me.role != "recruiter" {
        return Err(AppError::forbidden("仅招聘者可发起视频面试"));
    }
    let app = app_repo::get_view(&state.pool, &application_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("application `{application_id}`")))?;
    let company_ok = me.company_id.map(|cid| cid == app.company_id).unwrap_or(false);
    if !company_ok {
        return Err(AppError::forbidden("仅可对本企业收到的投递发起面试"));
    }
    Ok((app, me))
}

#[utoipa::path(
    post,
    path = "/interviews",
    tag = "interview",
    security(("bearerAuth" = [])),
    responses(
        (status = 201, description = "面试已创建（即时或预约）", body = InterviewView),
        (status = 400, description = "参数不合法", body = ErrorResponse),
        (status = 403, description = "非招聘者 / 非本企业投递", body = ErrorResponse),
        (status = 409, description = "该投递已存在活跃面试", body = ErrorResponse),
    )
)]
pub async fn create_interview(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    payload: web::Json<NewInterview>,
) -> ApiResult<HttpResponse> {
    let input = payload.into_inner();
    let (app, me) = recruiter_of_application(&state, &auth.user_id, input.application_id).await?;
    let view = interview_repo::create(&state.pool, &app, &me.id, input.scheduled_at).await?;
    ws::notify_interview(&state.hub, &view);
    Ok(HttpResponse::Created().json(view))
}

#[utoipa::path(
    get,
    path = "/interviews",
    tag = "interview",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "我参与的面试列表", body = Vec<InterviewView>),
        (status = 401, description = "未认证", body = ErrorResponse),
    )
)]
pub async fn list_mine(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
) -> ApiResult<HttpResponse> {
    let views = interview_repo::mine(&state.pool, &auth.user_id).await?;
    Ok(HttpResponse::Ok().json(views))
}

#[utoipa::path(
    get,
    path = "/interviews/{id}",
    tag = "interview",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "面试 ID")),
    responses(
        (status = 200, description = "面试详情", body = InterviewView),
        (status = 403, description = "非参与者", body = ErrorResponse),
        (status = 404, description = "不存在", body = ErrorResponse),
    )
)]
pub async fn get_interview(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
) -> ApiResult<HttpResponse> {
    let id = path.into_inner();
    let view = interview_repo::get(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("interview `{id}`")))?;
    if view.interviewer_id != auth.user_id && view.interviewee_id != auth.user_id {
        return Err(AppError::forbidden("非面试参与者"));
    }
    Ok(HttpResponse::Ok().json(view))
}

/// 状态流转通用：仓储更新成功后向双方推送 interview-updated
async fn mutate_and_notify(
    state: &AppState,
    id: Uuid,
    repo_call: impl std::future::Future<Output = ApiResult<Option<InterviewView>>>,
) -> ApiResult<HttpResponse> {
    let view = repo_call
        .await?
        .ok_or_else(|| AppError::not_found(format!("interview `{id}`")))?;
    ws::notify_interview(&state.hub, &view);
    Ok(HttpResponse::Ok().json(view))
}

#[utoipa::path(
    post,
    path = "/interviews/{id}/start",
    tag = "interview",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "面试 ID")),
    responses(
        (status = 200, description = "面试开始（状态→in_progress）", body = InterviewView),
        (status = 400, description = "未到预约时间 / 状态不允许", body = ErrorResponse),
        (status = 403, description = "非参与者", body = ErrorResponse),
        (status = 404, description = "不存在", body = ErrorResponse),
    )
)]
pub async fn start_interview(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
) -> ApiResult<HttpResponse> {
    let id = path.into_inner();
    mutate_and_notify(
        &state,
        id,
        interview_repo::start(&state.pool, &id, &auth.user_id),
    )
    .await
}

#[utoipa::path(
    post,
    path = "/interviews/{id}/finish",
    tag = "interview",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "面试 ID")),
    responses(
        (status = 200, description = "面试结束（状态→finished）", body = InterviewView),
        (status = 400, description = "状态不允许", body = ErrorResponse),
        (status = 403, description = "非参与者", body = ErrorResponse),
        (status = 404, description = "不存在", body = ErrorResponse),
    )
)]
pub async fn finish_interview(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
) -> ApiResult<HttpResponse> {
    let id = path.into_inner();
    mutate_and_notify(
        &state,
        id,
        interview_repo::finish(&state.pool, &id, &auth.user_id),
    )
    .await
}

#[utoipa::path(
    post,
    path = "/interviews/{id}/cancel",
    tag = "interview",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "面试 ID")),
    responses(
        (status = 200, description = "面试取消（状态→cancelled）", body = InterviewView),
        (status = 400, description = "状态不允许", body = ErrorResponse),
        (status = 403, description = "非参与者", body = ErrorResponse),
        (status = 404, description = "不存在", body = ErrorResponse),
    )
)]
pub async fn cancel_interview(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
) -> ApiResult<HttpResponse> {
    let id = path.into_inner();
    mutate_and_notify(
        &state,
        id,
        interview_repo::cancel(&state.pool, &id, &auth.user_id),
    )
    .await
}
