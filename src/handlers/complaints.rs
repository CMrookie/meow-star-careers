//! 投诉端点：发起（求职者，需实际沟通+有效证据）/ 查询 / 认领锁 / 审核
//!
//! 发起:  POST /api/v1/companies/{id}/complaints {evidence}
//! 查询:  GET  /api/v1/complaints/mine      （求职者我的投诉）
//!        GET  /api/v1/complaints           （审核账号/管理员=全部，招聘者=本企业，求职者=本人；?status=）
//! 认领:  POST /api/v1/complaints/{id}/claim    认领即锁定（重复调用=续约）
//! 释放:  POST /api/v1/complaints/{id}/release?force=  本人释放；管理员可强制释放
//! 审核:  POST /api/v1/complaints/{id}/review {approved, note?}
//!
//! 并行审核：多个审核账号同时在线时，`claim` 保证同一条投诉只有一个持锁人；
//! 锁到期自动失效，`review` 也会拒绝审结被别人持锁的投诉。

use actix_web::{web, HttpResponse};
use uuid::Uuid;

use crate::auth::AuthenticatedUser;
use crate::error::{ApiResult, AppError, ErrorResponse};
use crate::models::complaint::{ComplaintView, CreateComplaint, ReleaseLockQuery, ReviewComplaint};
use crate::repositories;
use crate::repositories::complaint as complaint_repo;
use crate::repositories::user as user_repo;
use crate::state::AppState;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/complaints")
            .route("/mine", web::get().to(my_complaints))
            .route("", web::get().to(list_complaints))
            .route("/{id}/claim", web::post().to(claim_complaint))
            .route("/{id}/release", web::post().to(release_complaint))
            .route("/{id}/review", web::post().to(review_complaint)),
    );
}

/// 投诉审核状态取值（与 0012_complaints.sql 的 CHECK 约束一致）
const COMPLAINT_STATUSES: [&str; 3] = ["pending", "approved", "rejected"];

fn validate_evidence(e: &str) -> Result<(), AppError> {
    let t = e.trim();
    if t.chars().count() < 20 || t.chars().count() > 5000 {
        return Err(AppError::bad_request("证据说明需在 20-5000 字符之间"));
    }
    Ok(())
}

/// 发起投诉（仅求职者；要求与该用人单位有过实际沟通：存在会话且至少一条消息）
#[utoipa::path(
    post,
    path = "/companies/{id}/complaints",
    tag = "complaint",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "被投诉企业 ID")),
    responses(
        (status = 201, description = "已提交，等待审核（审核通过才计入投诉次数）", body = ComplaintView),
        (status = 400, description = "证据说明需 20-5000 字符", body = ErrorResponse),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非求职者，或与该企业没有实际沟通记录", body = ErrorResponse),
        (status = 404, description = "企业不存在", body = ErrorResponse),
    )
)]
pub async fn create_complaint(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
    payload: web::Json<CreateComplaint>,
) -> ApiResult<HttpResponse> {
    let company_id = path.into_inner();
    let evidence = payload.into_inner().evidence;
    validate_evidence(&evidence)?;

    let me = user_repo::get(&state.pool, &auth.user_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("user `{}`", auth.user_id)))?;
    if me.role != "seeker" {
        return Err(AppError::forbidden("仅求职者可发起投诉"));
    }
    let company_exists = repositories::company::get(&state.pool, &company_id).await?.is_some();
    if !company_exists {
        return Err(AppError::not_found(format!("company `{company_id}`")));
    }
    // 规则：投诉前必须与该用人单位有过实际交流（存在会话且至少一条消息）
    if !complaint_repo::has_exchange(&state.pool, &auth.user_id, &company_id).await? {
        return Err(AppError::forbidden(
            "投诉前需与该用人单位有过实际沟通（如已发起会话并发送过消息）",
        ));
    }

    let view = complaint_repo::create(&state.pool, &company_id, &auth.user_id, evidence.trim()).await?;
    Ok(HttpResponse::Created().json(view))
}

/// 我的投诉（求职者）
#[utoipa::path(
    get,
    path = "/complaints/mine",
    tag = "complaint",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "我发起的投诉", body = [ComplaintView]),
        (status = 401, description = "未认证", body = ErrorResponse),
    )
)]
pub async fn my_complaints(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
) -> ApiResult<HttpResponse> {
    let views = complaint_repo::mine(&state.pool, &auth.user_id).await?;
    Ok(HttpResponse::Ok().json(views))
}

/// 角色化列表：审核账号/管理员=全部（可 ?status= 过滤）；招聘者=本企业投诉；求职者=本人
#[utoipa::path(
    get,
    path = "/complaints",
    tag = "complaint",
    security(("bearerAuth" = [])),
    params(("status" = Option<String>, Query, description = "pending / approved / rejected（仅对审核账号与管理员生效）")),
    responses(
        (status = 200, description = "投诉列表（按角色限定范围，含认领锁状态）", body = [ComplaintView]),
        (status = 400, description = "status 取值非法", body = ErrorResponse),
        (status = 401, description = "未认证", body = ErrorResponse),
    )
)]
pub async fn list_complaints(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    query: web::Query<ComplaintQuery>,
) -> ApiResult<HttpResponse> {
    if let Some(status) = query.status.as_deref() {
        if !COMPLAINT_STATUSES.contains(&status) {
            return Err(AppError::bad_request(format!(
                "status 取值: {}",
                COMPLAINT_STATUSES.join(" / ")
            )));
        }
    }
    let me = user_repo::get(&state.pool, &auth.user_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("user `{}`", auth.user_id)))?;
    let views: Vec<ComplaintView> = match me.role.as_str() {
        // 审核账号与平台管理员都能看到全部投诉（?status= 过滤待审队列）
        "admin" | "reviewer" => complaint_repo::all(&state.pool, query.status.as_deref()).await?,
        "recruiter" => {
            let cid = me.company_id.ok_or_else(|| AppError::forbidden("招聘者未绑定企业"))?;
            complaint_repo::for_company(&state.pool, &cid).await?
        }
        _ => complaint_repo::mine(&state.pool, &auth.user_id).await?,
    };
    Ok(HttpResponse::Ok().json(views))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComplaintQuery {
    pub status: Option<String>,
}

/// 认领投诉（认领即锁定；重复调用即续约）
///
/// 多审核并行场景下，同一条投诉同一时刻只允许一个审核持有锁；
/// 锁到期（默认 10 分钟）自动失效，可被他人重新认领。
#[utoipa::path(
    post,
    path = "/complaints/{id}/claim",
    tag = "complaint",
    description = "认领即锁定，避免多个审核同时审同一条；重复调用视为续约（延长锁到期时间）",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "投诉 ID")),
    responses(
        (status = 200, description = "认领成功（返回含锁状态的投诉）", body = ComplaintView),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非审核账号/平台管理员", body = ErrorResponse),
        (status = 404, description = "投诉不存在", body = ErrorResponse),
        (status = 409, description = "已被其他审核认领，或该投诉已审结", body = ErrorResponse),
    )
)]
pub async fn claim_complaint(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
) -> ApiResult<HttpResponse> {
    let reviewer = user_repo::require_reviewer(&state.pool, &auth.user_id).await?;
    let id = path.into_inner();
    let view = complaint_repo::claim(
        &state.pool,
        &id,
        &reviewer.id,
        complaint_repo::LOCK_TTL_SECONDS,
    )
    .await?;
    Ok(HttpResponse::Ok().json(view))
}

/// 释放认领锁（本人释放；平台管理员可 ?force=true 强制释放他人）
#[utoipa::path(
    post,
    path = "/complaints/{id}/release",
    tag = "complaint",
    description = "释放自己认领的投诉；平台管理员可用 force=true 强制解锁。已过期的锁任何人可清理",
    security(("bearerAuth" = [])),
    params(
        ("id" = Uuid, Path, description = "投诉 ID"),
        ("force" = Option<bool>, Query, description = "强制释放（仅平台管理员）"),
    ),
    responses(
        (status = 200, description = "释放成功（返回含锁状态的投诉）", body = ComplaintView),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非审核账号/管理员，或非管理员使用了 force", body = ErrorResponse),
        (status = 404, description = "投诉不存在", body = ErrorResponse),
        (status = 409, description = "该投诉当前没有被锁定", body = ErrorResponse),
    )
)]
pub async fn release_complaint(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
    query: web::Query<ReleaseLockQuery>,
) -> ApiResult<HttpResponse> {
    let actor = user_repo::require_reviewer(&state.pool, &auth.user_id).await?;
    let id = path.into_inner();
    let force = query.into_inner().force;
    if force && actor.role != "admin" {
        return Err(AppError::forbidden("仅平台管理员可强制释放他人的认领锁"));
    }
    let view = complaint_repo::release(&state.pool, &id, &actor.id, force).await?;
    Ok(HttpResponse::Ok().json(view))
}

/// 审核：通过后累计企业投诉次数（立即影响职位列表排序）；驳回不累计。
/// 并行保护：投诉被他人有效锁定时拒绝审结，重复提交由条件更新兜底为 409。
#[utoipa::path(
    post,
    path = "/complaints/{id}/review",
    tag = "complaint",
    description = "审核投诉（审核账号/平台管理员）。被他人在审（持锁）时返回 409；重复审核 409",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "投诉 ID")),
    responses(
        (status = 200, description = "审核结果（approved 已通过 / rejected 已驳回）", body = ComplaintView),
        (status = 400, description = "审核备注超过 500 字符", body = ErrorResponse),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非审核账号/平台管理员", body = ErrorResponse),
        (status = 404, description = "投诉不存在", body = ErrorResponse),
        (status = 409, description = "已审核过，或正被其他审核持锁处理中", body = ErrorResponse),
    )
)]
pub async fn review_complaint(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
    payload: web::Json<ReviewComplaint>,
) -> ApiResult<HttpResponse> {
    let reviewer = user_repo::require_reviewer(&state.pool, &auth.user_id).await?;
    let id = path.into_inner();
    let p = payload.into_inner();
    if p.note.as_deref().is_some_and(|note| note.trim().chars().count() > 500) {
        return Err(AppError::bad_request("审核备注不能超过 500 字符"));
    }
    let note = p.note.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let view = complaint_repo::review(&state.pool, &id, &reviewer.id, p.approved, note)
        .await?
        .ok_or_else(|| AppError::not_found(format!("complaint `{id}`")))?;
    Ok(HttpResponse::Ok().json(view))
}
