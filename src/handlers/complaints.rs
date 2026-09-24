//! 投诉端点：发起（求职者，需实际沟通+有效证据）/ 查询 / 管理员审核
//! 发起:  POST /api/v1/companies/{id}/complaints {evidence}
//! 查询:  GET  /api/v1/complaints/mine      （求职者我的投诉）
//!        GET  /api/v1/complaints           （招聘者=本企业 / 管理员=全部，?status=）
//! 审核:  POST /api/v1/complaints/{id}/review {approved, note?}（仅 admin）

use actix_web::{web, HttpResponse};
use uuid::Uuid;

use crate::auth::AuthenticatedUser;
use crate::error::{ApiResult, AppError, ErrorResponse};
use crate::models::complaint::{ComplaintView, CreateComplaint, ReviewComplaint};
use crate::repositories;
use crate::repositories::complaint as complaint_repo;
use crate::repositories::user as user_repo;
use crate::state::AppState;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/complaints")
            .route("/mine", web::get().to(my_complaints))
            .route("", web::get().to(list_complaints))
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
        (status = 201, description = "已提交，等待平台管理员审核（审核通过才计入投诉次数）", body = ComplaintView),
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

/// 角色化列表：管理员=全部（可 ?status= 过滤）；招聘者=本企业投诉；求职者=本人
#[utoipa::path(
    get,
    path = "/complaints",
    tag = "complaint",
    security(("bearerAuth" = [])),
    params(("status" = Option<String>, Query, description = "pending / approved / rejected（仅对管理员生效）")),
    responses(
        (status = 200, description = "投诉列表（按角色限定范围）", body = [ComplaintView]),
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

/// 审核（审核专用账号 reviewer 或平台管理员 admin）：通过后累计企业投诉次数
/// （立即影响职位列表「投诉等级从优到劣」排序）；驳回不累计。审核人记录在 `reviewedBy`，
/// 多审核账号并行时可通过 `reviewedByName` 追溯是谁审的。
#[utoipa::path(
    post,
    path = "/complaints/{id}/review",
    tag = "complaint",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "投诉 ID")),
    responses(
        (status = 200, description = "审核结果（approved 已通过 / rejected 已驳回）", body = ComplaintView),
        (status = 400, description = "审核备注超过 500 字符", body = ErrorResponse),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "既非审核账号也非平台管理员", body = ErrorResponse),
        (status = 404, description = "投诉不存在", body = ErrorResponse),
        (status = 409, description = "该投诉已审核过，请勿重复审核", body = ErrorResponse),
    )
)]
pub async fn review_complaint(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
    payload: web::Json<ReviewComplaint>,
) -> ApiResult<HttpResponse> {
    let moderator = user_repo::require_reviewer(&state.pool, &auth.user_id).await?;
    let id = path.into_inner();
    let p = payload.into_inner();
    if p.note.as_deref().is_some_and(|note| note.trim().chars().count() > 500) {
        return Err(AppError::bad_request("审核备注不能超过 500 字符"));
    }
    let note = p.note.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let view = complaint_repo::review(&state.pool, &id, &moderator.id, p.approved, note)
        .await?
        .ok_or_else(|| AppError::not_found(format!("complaint `{id}`")))?;
    tracing::info!(
        reviewer_id = %moderator.id, reviewer_role = %moderator.role, complaint_id = %id,
        approved = p.approved, "投诉审核完成"
    );
    Ok(HttpResponse::Ok().json(view))
}
