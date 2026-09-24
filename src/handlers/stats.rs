//! 平台统计端点：审核工作量与及时性 / 用人单位优劣 / 求职用户分析。
//!
//! 权限与可见范围：
//! - `GET /stats/review`：审核账号与平台管理员。管理员看全部审核账号；
//!   审核账号**只看自己**（工作量看板不对同事互相暴露）。
//! - `GET /stats/companies`、`GET /stats/seekers`：**仅平台管理员**——它们覆盖全平台数据。
//!
//! 口径：审核只评价及时性（提交->审结、接单->审结与及时率），不返回任何通过/驳回结论指标。

use actix_web::{web, HttpResponse};

use crate::auth::AuthenticatedUser;
use crate::error::{ApiResult, ErrorResponse};
use crate::models::stats::{ReviewStats, SeekerStats, REVIEW_SLA_SECONDS};
use crate::repositories::{stats as stats_repo, user as user_repo};
use crate::state::AppState;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/stats")
            .route("/review", web::get().to(review_stats))
            .route("/companies", web::get().to(company_stats))
            .route("/seekers", web::get().to(seeker_stats)),
    );
}

/// 公司统计的查询参数
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanyStatsQuery {
    /// 返回条数上限（默认 200，1-1000）
    pub limit: Option<i64>,
}

/// 审核队列概览 + 审核账号工作量（管理员看全部，审核账号只看自己）
#[utoipa::path(
    get,
    path = "/stats/review",
    tag = "stats",
    description = "审核统计：队列积压 + 每个审核账号的工作量与及时性。只衡量及时性，不含通过/驳回结论分布。",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "审核统计", body = crate::models::stats::ReviewStats),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非审核账号/平台管理员", body = ErrorResponse),
    )
)]
pub async fn review_stats(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
) -> ApiResult<HttpResponse> {
    let me = user_repo::require_reviewer(&state.pool, &auth.user_id).await?;
    let queue = stats_repo::review_queue(&state.pool, &me.id, REVIEW_SLA_SECONDS).await?;
    let reviewers = if me.role == "admin" {
        stats_repo::reviewer_workloads(&state.pool, None, REVIEW_SLA_SECONDS).await?
    } else {
        stats_repo::reviewer_workloads(&state.pool, Some(&me.id), REVIEW_SLA_SECONDS).await?
    };
    Ok(HttpResponse::Ok().json(ReviewStats {
        queue,
        reviewers,
        sla_seconds: REVIEW_SLA_SECONDS,
    }))
}

/// 用人单位质量画像（仅平台管理员）
#[utoipa::path(
    get,
    path = "/stats/companies",
    tag = "stats",
    description = "用人单位优劣分析：投诉结构（待审/已核实/驳回）、职位规模、等级序、投诉率与综合质量分，最差在前",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "用人单位质量列表", body = [crate::models::stats::CompanyQuality]),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非平台管理员", body = ErrorResponse),
    )
)]
pub async fn company_stats(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    query: web::Query<CompanyStatsQuery>,
) -> ApiResult<HttpResponse> {
    user_repo::require_admin(&state.pool, &auth.user_id).await?;
    let limit = query.into_inner().limit.unwrap_or(200).clamp(1, 1000);
    let rows = stats_repo::company_quality(&state.pool, limit).await?;
    Ok(HttpResponse::Ok().json(rows))
}

/// 求职用户分析（仅平台管理员）
#[utoipa::path(
    get,
    path = "/stats/seekers",
    tag = "stats",
    description = "求职用户分析：账号总量/活跃、注册趋势（近 12 个月）、简历与投递参与度、投递状态与分档分布",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "求职用户统计", body = crate::models::stats::SeekerStats),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非平台管理员", body = ErrorResponse),
    )
)]
pub async fn seeker_stats(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
) -> ApiResult<HttpResponse> {
    user_repo::require_admin(&state.pool, &auth.user_id).await?;
    let stats: SeekerStats = stats_repo::seeker_stats(&state.pool).await?;
    Ok(HttpResponse::Ok().json(stats))
}
