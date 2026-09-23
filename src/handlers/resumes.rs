//! 简历端点（求职者管理自己；招聘者可检索公开简历）。

use actix_web::{web, HttpResponse};
use uuid::Uuid;

use crate::auth::AuthenticatedUser;
use crate::error::{ApiResult, AppError, ErrorResponse};
use crate::models::resume::{ResumePage, ResumeQuery, ResumeWrite};
use crate::repositories::{resume as resume_repo, user as user_repo};
use crate::state::AppState;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/resumes")
            .route("", web::post().to(create_resume))
            .route("", web::get().to(my_resumes))
            // /search 必须先于 /{id} 注册
            .route("/search", web::get().to(search_resumes))
            .route("/{id}", web::get().to(get_resume))
            .route("/{id}", web::put().to(update_resume))
            .route("/{id}", web::delete().to(delete_resume)),
    );
}

fn validate_text(field: &str, value: &str, max: usize) -> Result<(), AppError> {
    if value.trim().chars().count() > max {
        return Err(AppError::bad_request(format!("{field} 不能超过 {max} 字符")));
    }
    Ok(())
}

fn validate_resume(input: &ResumeWrite) -> Result<(), AppError> {
    let full_name = input.full_name.as_deref().map(str::trim).unwrap_or_default();
    if full_name.is_empty() {
        return Err(AppError::bad_request("fullName 不能为空"));
    }
    validate_text("fullName", full_name, 64)?;
    let title = input.title.as_deref().map(str::trim).unwrap_or_default();
    if title.is_empty() {
        return Err(AppError::bad_request("title（期望职位）不能为空"));
    }
    validate_text("title", title, 128)?;
    if let Some(v) = input.phone.as_deref() {
        validate_text("phone", v, 32)?;
    }
    if let Some(v) = input.email.as_deref() {
        if v.trim().len() > 254 {
            return Err(AppError::bad_request("email 不能超过 254 字符"));
        }
    }
    if let Some(v) = input.education.as_deref() {
        validate_text("education", v, 128)?;
    }
    if let Some(v) = input.skills.as_deref() {
        validate_text("skills", v, 5000)?;
    }
    if let Some(v) = input.summary.as_deref() {
        validate_text("summary", v, 5000)?;
    }
    if input.years.is_some_and(|years| !(0..=100).contains(&years)) {
        return Err(AppError::bad_request("years 须在 0-100 之间"));
    }
    Ok(())
}

/// 新建简历（求职者）
#[utoipa::path(
    post,
    path = "/resumes",
    tag = "resume",
    security(("bearerAuth" = [])),
    responses(
        (status = 201, description = "创建成功", body = crate::models::resume::Resume),
        (status = 400, description = "fullName/title 必填", body = ErrorResponse),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非求职者", body = ErrorResponse),
    )
)]
pub async fn create_resume(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    payload: web::Json<ResumeWrite>,
) -> ApiResult<HttpResponse> {
    let seeker = user_repo::require_seeker(&state.pool, &auth.user_id).await?;
    validate_resume(&payload)?;

    let resume = resume_repo::create(&state.pool, &seeker.id, &payload).await?;
    Ok(HttpResponse::Created().json(resume))
}

/// 我的简历列表
#[utoipa::path(
    get,
    path = "/resumes",
    tag = "resume",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "简历列表", body = [crate::models::resume::Resume]),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非求职者", body = ErrorResponse),
    )
)]
pub async fn my_resumes(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
) -> ApiResult<HttpResponse> {
    let seeker = user_repo::require_seeker(&state.pool, &auth.user_id).await?;
    let resumes = resume_repo::list_mine(&state.pool, &seeker.id).await?;
    Ok(HttpResponse::Ok().json(resumes))
}

/// 招聘者检索公开简历
#[utoipa::path(
    get,
    path = "/resumes/search",
    tag = "resume",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "公开简历（仅公开可见）", body = ResumePage),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非招聘者", body = ErrorResponse),
    )
)]
pub async fn search_resumes(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    query: web::Query<ResumeQuery>,
) -> ApiResult<HttpResponse> {
    user_repo::require_recruiter(&state.pool, &auth.user_id).await?;
    let query = query.into_inner();
    if let Some(keyword) = query.keyword.as_deref() {
        if keyword.chars().count() > 200 {
            return Err(AppError::bad_request("keyword 不能超过 200 字符"));
        }
    }
    let page = resume_repo::search_public(&state.pool, &query).await?;
    Ok(HttpResponse::Ok().json(page))
}

/// 简历详情：本人可见全部；招聘者仅可见公开简历（隐私：非公开对他人按 404 处理）
#[utoipa::path(
    get,
    path = "/resumes/{id}",
    tag = "resume",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "简历 ID")),
    responses(
        (status = 200, description = "简历", body = crate::models::resume::Resume),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 404, description = "简历不存在或不可见", body = ErrorResponse),
    )
)]
pub async fn get_resume(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
) -> ApiResult<HttpResponse> {
    let id = path.into_inner();
    let Some(resume) = resume_repo::get(&state.pool, &id).await? else {
        return Err(AppError::not_found(format!("resume `{id}`")));
    };

    let own = resume.user_id == auth.user_id;
    let recruiter_can_view = !own
        && resume.is_public
        && user_repo::get(&state.pool, &auth.user_id)
            .await?
            .map(|u| u.role == "recruiter")
            .unwrap_or(false);

    if own || recruiter_can_view {
        Ok(HttpResponse::Ok().json(resume))
    } else {
        // 不泄露简历是否存在
        Err(AppError::not_found(format!("resume `{id}`")))
    }
}

/// 更新简历（仅本人）
#[utoipa::path(
    put,
    path = "/resumes/{id}",
    tag = "resume",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "简历 ID")),
    responses(
        (status = 200, description = "更新成功", body = crate::models::resume::Resume),
        (status = 400, description = "参数不合法", body = ErrorResponse),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 404, description = "简历不存在或非本人", body = ErrorResponse),
    )
)]
pub async fn update_resume(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
    payload: web::Json<ResumeWrite>,
) -> ApiResult<HttpResponse> {
    let seeker = user_repo::require_seeker(&state.pool, &auth.user_id).await?;
    validate_resume(&payload)?;
    let id = path.into_inner();

    let resume = resume_repo::update(&state.pool, &id, &seeker.id, &payload)
        .await?
        .ok_or_else(|| AppError::not_found(format!("resume `{id}`")))?;
    Ok(HttpResponse::Ok().json(resume))
}

/// 删除简历（仅本人）
#[utoipa::path(
    delete,
    path = "/resumes/{id}",
    tag = "resume",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "简历 ID")),
    responses(
        (status = 204, description = "删除成功"),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 404, description = "简历不存在或非本人", body = ErrorResponse),
    )
)]
pub async fn delete_resume(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
) -> ApiResult<HttpResponse> {
    let seeker = user_repo::require_seeker(&state.pool, &auth.user_id).await?;
    let id = path.into_inner();

    let deleted = resume_repo::delete(&state.pool, &id, &seeker.id).await?;
    if !deleted {
        return Err(AppError::not_found(format!("resume `{id}`")));
    }
    Ok(HttpResponse::NoContent().finish())
}
