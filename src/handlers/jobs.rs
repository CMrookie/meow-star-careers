//! 职位端点：公开浏览/搜索、招聘者发布管理、收藏。

use actix_web::{web, HttpResponse};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::AuthenticatedUser;
use crate::error::{ApiResult, AppError, ErrorResponse};
use crate::models::job::{JOB_TYPES, JobPage, JobQuery, NewJob, UpdateJob};
use crate::repositories::{application as app_repo, job as job_repo, user as user_repo};
use crate::state::AppState;
use utoipa::ToSchema;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/jobs")
            .route("", web::get().to(list_jobs))
            .route("", web::post().to(create_job))
            // /my 必须先于 /{id} 注册，避免被动态段抢占
            .route("/my", web::get().to(my_jobs))
            .route("/{id}", web::get().to(get_job))
            .route("/{id}", web::put().to(update_job))
            .route("/{id}", web::delete().to(delete_job))
            .route("/{id}/apply", web::post().to(apply_job))
            .route("/{id}/active", web::post().to(set_job_active))
            .route("/{id}/save", web::post().to(save_job))
            .route("/{id}/unsave", web::post().to(unsave_job)),
    );
}

/// 上下架请求体
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActiveRequest {
    pub is_active: bool,
}

fn validate_text(field: &str, value: &str, max: usize) -> Result<(), AppError> {
    if value.trim().chars().count() > max {
        return Err(AppError::bad_request(format!("{field} 不能超过 {max} 字符")));
    }
    Ok(())
}

fn validate_new_job(input: &NewJob) -> Result<(), AppError> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err(AppError::bad_request("title 不能为空"));
    }
    validate_text("title", title, 128)?;
    let description = input.description.trim();
    if description.is_empty() {
        return Err(AppError::bad_request("description 不能为空"));
    }
    validate_text("description", description, 10_000)?;
    if let Some(v) = input.requirements.as_deref() {
        validate_text("requirements", v, 10_000)?;
    }
    if let Some(v) = input.location.as_deref() {
        validate_text("location", v, 100)?;
    }
    if let Some(v) = input.experience.as_deref() {
        validate_text("experience", v, 100)?;
    }
    if let Some(v) = input.education.as_deref() {
        validate_text("education", v, 100)?;
    }
    if let Some(job_type) = &input.job_type {
        if !JOB_TYPES.contains(&job_type.as_str()) {
            return Err(AppError::bad_request(format!(
                "job_type 取值: {}",
                JOB_TYPES.join(" / ")
            )));
        }
    }
    for (label, value) in [("salary_min", input.salary_min), ("salary_max", input.salary_max)] {
        if let Some(value) = value {
            if !(0..=10_000_000).contains(&value) {
                return Err(AppError::bad_request(format!("{label} 须在 0~10,000,000 之间")));
            }
        }
    }
    if let (Some(min), Some(max)) = (input.salary_min, input.salary_max) {
        if min > max {
            return Err(AppError::bad_request("salary_min 不能大于 salary_max"));
        }
    }
    Ok(())
}

fn validate_update_job(input: &UpdateJob) -> Result<(), AppError> {
    if let Some(v) = input.title.as_deref() {
        if v.trim().is_empty() {
            return Err(AppError::bad_request("title 不能为空"));
        }
        validate_text("title", v, 128)?;
    }
    if let Some(v) = input.description.as_deref() {
        if v.trim().is_empty() {
            return Err(AppError::bad_request("description 不能为空"));
        }
        validate_text("description", v, 10_000)?;
    }
    if let Some(v) = input.requirements.as_deref() {
        validate_text("requirements", v, 10_000)?;
    }
    if let Some(v) = input.location.as_deref() {
        validate_text("location", v, 100)?;
    }
    if let Some(v) = input.experience.as_deref() {
        validate_text("experience", v, 100)?;
    }
    if let Some(v) = input.education.as_deref() {
        validate_text("education", v, 100)?;
    }
    if let Some(job_type) = &input.job_type {
        if !JOB_TYPES.contains(&job_type.as_str()) {
            return Err(AppError::bad_request(format!(
                "job_type 取值: {}",
                JOB_TYPES.join(" / ")
            )));
        }
    }
    for (label, value) in [("salary_min", input.salary_min), ("salary_max", input.salary_max)] {
        if let Some(value) = value {
            if !(0..=10_000_000).contains(&value) {
                return Err(AppError::bad_request(format!("{label} 须在 0~10,000,000 之间")));
            }
        }
    }
    if let (Some(min), Some(max)) = (input.salary_min, input.salary_max) {
        if min > max {
            return Err(AppError::bad_request("salary_min 不能大于 salary_max"));
        }
    }
    Ok(())
}

/// 搜索关键词等查询参数长度上限校验
fn validate_search_query(query: &JobQuery) -> Result<(), AppError> {
    if let Some(keyword) = query.keyword.as_deref() {
        if keyword.chars().count() > 200 {
            return Err(AppError::bad_request("keyword 不能超过 200 字符"));
        }
    }
    if let Some(location) = query.location.as_deref() {
        if location.chars().count() > 100 {
            return Err(AppError::bad_request("location 不能超过 100 字符"));
        }
    }
    Ok(())
}

/// 公开搜索在招职位
///
/// 排序契约（定级 v2）：结果按用人单位投诉等级**从优到劣**返回（优秀 → 轻微 → 预警 → 警告 → 严重）。
/// 等级按**公司规模折算**：企业申报了员工数且 ≥50 人时用每百人投诉率（≤0.5% 轻微 / ≤1.5% 预警 /
/// ≤3.0% 警告 / >3.0% 严重），未申报或 <50 人（小样本波动大）退回投诉次数口径；
/// 同等级内「有规模折算的」优先，再按率（或次数）由小到大，然后发布时间倒序、id 兜底。
/// 排序在服务端完成，因此翻页时全局有序（函数与阈值见 `migrations/0017_company_staff_size.sql`，
/// 与求职 App 的 `assessComplaints` / `jobSortKey` 同一口径）。
#[utoipa::path(
    get,
    path = "/jobs",
    tag = "job",
    description = "搜索在招职位；按投诉等级从优到劣排序（优秀→轻微→预警→警告→严重）。等级按公司规模折算：申报规模且 ≥50 人时用每百人投诉率（0.5/1.5/3.0%），否则退回投诉次数；同级内有规模折算者优先，再按率/次数小者优先，翻页全局有序。响应含 companyStaffSize（企业规模，未申报为 null）",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "职位列表", body = JobPage),
        (status = 401, description = "未认证", body = ErrorResponse),
    )
)]
pub async fn list_jobs(
    state: web::Data<AppState>,
    _auth: AuthenticatedUser,
    query: web::Query<JobQuery>,
) -> ApiResult<HttpResponse> {
    let query = query.into_inner();
    validate_search_query(&query)?;
    let (page, page_size, _) = crate::models::pagination::paginate(query.page, query.page_size);
    let (items, total) = job_repo::search_active(&state.pool, &query).await?;
    Ok(HttpResponse::Ok().json(JobPage {
        items,
        total,
        page,
        page_size,
    }))
}

/// 发布职位（招聘者）
#[utoipa::path(
    post,
    path = "/jobs",
    tag = "job",
    security(("bearerAuth" = [])),
    responses(
        (status = 201, description = "发布成功", body = crate::models::job::JobView),
        (status = 400, description = "参数不合法", body = ErrorResponse),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非招聘者", body = ErrorResponse),
    )
)]
pub async fn create_job(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    payload: web::Json<NewJob>,
) -> ApiResult<HttpResponse> {
    let recruiter = user_repo::require_recruiter(&state.pool, &auth.user_id).await?;
    validate_new_job(&payload)?;

    let company_id = recruiter.company_id.expect("require_recruiter 已校验");
    let job = job_repo::create(&state.pool, &company_id, &recruiter.id, &payload).await?;
    Ok(HttpResponse::Created().json(job))
}

/// 我的（本企业）职位
///
/// 排序与公开列表一致（投诉等级从优到劣）；本企业职位投诉次数相同，实际退化为按发布时间倒序。
#[utoipa::path(
    get,
    path = "/jobs/my",
    tag = "job",
    description = "本企业职位；排序与公开列表一致（投诉等级从优到劣、按公司规模折算），本企业内等级相同，故实际按发布时间倒序",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "本企业职位", body = JobPage),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非招聘者", body = ErrorResponse),
    )
)]
pub async fn my_jobs(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    query: web::Query<crate::models::pagination::PageQuery>,
) -> ApiResult<HttpResponse> {
    let recruiter = user_repo::require_recruiter(&state.pool, &auth.user_id).await?;
    let company_id = recruiter.company_id.expect("require_recruiter 已校验");

    let (items, total) =
        job_repo::list_for_company(&state.pool, &company_id, query.page, query.page_size).await?;
    let (page, page_size, _) = crate::models::pagination::paginate(query.page, query.page_size);
    Ok(HttpResponse::Ok().json(JobPage {
        items,
        total,
        page,
        page_size,
    }))
}

/// 职位详情（公开上架职位；本企业招聘者可看下架职位）
#[utoipa::path(
    get,
    path = "/jobs/{id}",
    tag = "job",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "职位 ID")),
    responses(
        (status = 200, description = "职位详情", body = crate::models::job::JobView),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 404, description = "职位不存在或未上架", body = ErrorResponse),
    )
)]
pub async fn get_job(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
) -> ApiResult<HttpResponse> {
    let id = path.into_inner();

    if let Some(job) = job_repo::get_active(&state.pool, &id).await? {
        return Ok(HttpResponse::Ok().json(job));
    }
    // 本企业招聘者允许查看已下架职位
    if let Ok(recruiter) = user_repo::require_recruiter(&state.pool, &auth.user_id).await {
        if let Some(job) = job_repo::get_by_id(&state.pool, &id).await? {
            if job.company_id == recruiter.company_id.unwrap_or_default() {
                return Ok(HttpResponse::Ok().json(job));
            }
        }
    }

    Err(AppError::not_found(format!("job `{id}`")))
}

/// 更新职位（招聘者，仅本企业职位）
#[utoipa::path(
    put,
    path = "/jobs/{id}",
    tag = "job",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "职位 ID")),
    responses(
        (status = 200, description = "更新成功", body = crate::models::job::JobView),
        (status = 400, description = "参数不合法", body = ErrorResponse),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非本企业职位", body = ErrorResponse),
        (status = 404, description = "职位不存在", body = ErrorResponse),
    )
)]
pub async fn update_job(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
    payload: web::Json<UpdateJob>,
) -> ApiResult<HttpResponse> {
    let recruiter = user_repo::require_recruiter(&state.pool, &auth.user_id).await?;
    let id = path.into_inner();
    validate_update_job(&payload)?;

    let job = job_repo::get_by_id(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("job `{id}`")))?;
    if job.company_id != recruiter.company_id.unwrap_or_default() {
        return Err(AppError::forbidden("仅可操作本企业职位"));
    }

    let job = job_repo::update(&state.pool, &id, &payload)
        .await?
        .ok_or_else(|| AppError::not_found(format!("job `{id}`")))?;
    Ok(HttpResponse::Ok().json(job))
}

/// 删除职位（招聘者，仅本企业职位）
#[utoipa::path(
    delete,
    path = "/jobs/{id}",
    tag = "job",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "职位 ID")),
    responses(
        (status = 204, description = "删除成功"),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非本企业职位", body = ErrorResponse),
        (status = 404, description = "职位不存在", body = ErrorResponse),
    )
)]
pub async fn delete_job(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
) -> ApiResult<HttpResponse> {
    let recruiter = user_repo::require_recruiter(&state.pool, &auth.user_id).await?;
    let id = path.into_inner();

    let job = job_repo::get_by_id(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("job `{id}`")))?;
    if job.company_id != recruiter.company_id.unwrap_or_default() {
        return Err(AppError::forbidden("仅可操作本企业职位"));
    }

    job_repo::delete(&state.pool, &id).await?;
    Ok(HttpResponse::NoContent().finish())
}

/// 上架/下架职位（招聘者）
#[utoipa::path(
    post,
    path = "/jobs/{id}/active",
    tag = "job",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "职位 ID")),
    responses(
        (status = 200, description = "更新后的职位", body = crate::models::job::JobView),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非本企业职位", body = ErrorResponse),
        (status = 404, description = "职位不存在", body = ErrorResponse),
    )
)]
pub async fn set_job_active(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
    payload: web::Json<ActiveRequest>,
) -> ApiResult<HttpResponse> {
    let recruiter = user_repo::require_recruiter(&state.pool, &auth.user_id).await?;
    let id = path.into_inner();

    let job = job_repo::get_by_id(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("job `{id}`")))?;
    if job.company_id != recruiter.company_id.unwrap_or_default() {
        return Err(AppError::forbidden("仅可操作本企业职位"));
    }

    job_repo::set_active(&state.pool, &id, payload.is_active).await?;
    let job = job_repo::get_by_id(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("job `{id}`")))?;
    Ok(HttpResponse::Ok().json(job))
}

/// 求职者投递职位
#[utoipa::path(
    post,
    path = "/jobs/{id}/apply",
    tag = "application",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "职位 ID")),
    responses(
        (status = 201, description = "投递成功", body = crate::models::application::ApplicationView),
        (status = 400, description = "职位未上架/参数不合法", body = ErrorResponse),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非求职者", body = ErrorResponse),
        (status = 404, description = "职位不存在", body = ErrorResponse),
        (status = 409, description = "重复投递", body = ErrorResponse),
    )
)]
pub async fn apply_job(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
    payload: web::Json<crate::models::application::ApplyRequest>,
) -> ApiResult<HttpResponse> {
    let seeker = user_repo::require_seeker(&state.pool, &auth.user_id).await?;
    let job_id = path.into_inner();

    if job_repo::get_active(&state.pool, &job_id)
        .await?
        .is_none()
    {
        return Err(AppError::not_found(format!("job `{job_id}` 未在招或不存在")));
    }
    if app_repo::find_by_job_and_seeker(&state.pool, &job_id, &seeker.id)
        .await?
        .is_some()
    {
        return Err(AppError::conflict("已投递过该职位"));
    }

    let payload = payload.into_inner();
    if let Some(cover) = payload.cover_letter.as_deref() {
        if cover.trim().chars().count() > 2000 {
            return Err(AppError::bad_request("coverLetter 不能超过 2000 字符"));
        }
    }
    let application =
        app_repo::apply(&state.pool, &job_id, &seeker.id, payload.resume_id, payload.cover_letter.as_deref())
            .await?;
    Ok(HttpResponse::Created().json(application))
}

/// 收藏职位（任意角色可收藏，防重复）
#[utoipa::path(
    post,
    path = "/jobs/{id}/save",
    tag = "job",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "职位 ID")),
    responses(
        (status = 204, description = "已收藏"),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 404, description = "职位不存在", body = ErrorResponse),
    )
)]
pub async fn save_job(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
) -> ApiResult<HttpResponse> {
    let job_id = path.into_inner();
    if job_repo::get_active(&state.pool, &job_id)
        .await?
        .is_none()
    {
        return Err(AppError::not_found(format!("job `{job_id}`")));
    }
    job_repo::save(&state.pool, &auth.user_id, &job_id).await?;
    Ok(HttpResponse::NoContent().finish())
}

/// 取消收藏
#[utoipa::path(
    post,
    path = "/jobs/{id}/unsave",
    tag = "job",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "职位 ID")),
    responses(
        (status = 204, description = "已取消收藏"),
        (status = 401, description = "未认证", body = ErrorResponse),
    )
)]
pub async fn unsave_job(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
) -> ApiResult<HttpResponse> {
    let job_id = path.into_inner();
    job_repo::unsave(&state.pool, &auth.user_id, &job_id).await?;
    Ok(HttpResponse::NoContent().finish())
}

/// 我的收藏职位列表
///
/// 排序与公开列表一致（投诉等级从优到劣）；同级内按最近收藏时间倒序。
#[utoipa::path(
    get,
    path = "/saved-jobs",
    tag = "job",
    description = "收藏的职位；排序与公开列表一致（投诉等级从优到劣、按公司规模折算），同级内按最近收藏时间倒序",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "收藏的职位", body = JobPage),
        (status = 401, description = "未认证", body = ErrorResponse),
    )
)]
pub async fn saved_jobs(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    query: web::Query<crate::models::pagination::PageQuery>,
) -> ApiResult<HttpResponse> {
    let (items, total) = job_repo::list_saved(&state.pool, &auth.user_id, query.page, query.page_size)
        .await?;
    let (page, page_size, _) = crate::models::pagination::paginate(query.page, query.page_size);
    Ok(HttpResponse::Ok().json(JobPage {
        items,
        total,
        page,
        page_size,
    }))
}
