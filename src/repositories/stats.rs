//! 统计查询（只读聚合）：审核工作量/及时性、用人单位优劣、求职用户分析。
//!
//! 所有查询都只返回**聚合值**，不返回用户明细，避免统计接口变成批量导出通道。

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;
use crate::models::stats::{
    BucketCount, CompanyQuality, MonthlyCount, ReviewQueue, ReviewerWorkload, SeekerOverview,
    SeekerStats, StatusCount,
};

/// 待审队列概览（`me` 用于算出「我自己持有几把锁」）
pub async fn review_queue(pool: &PgPool, me: &Uuid, sla_seconds: i64) -> ApiResult<ReviewQueue> {
    let row = sqlx::query_as::<_, ReviewQueue>(
        "SELECT count(*) FILTER (WHERE status = 'pending') AS pending_total,
                count(*) FILTER (WHERE status = 'pending' AND locked_by IS NOT NULL
                                   AND lock_expires_at > now()) AS locked_active,
                count(*) FILTER (WHERE status = 'pending' AND locked_by IS NOT NULL
                                   AND lock_expires_at <= now()) AS locked_expired,
                count(*) FILTER (WHERE status = 'pending' AND locked_by = $1) AS locked_by_me,
                count(*) FILTER (WHERE status = 'pending'
                                   AND created_at <= now() - ($2::double precision * interval '1 second'))
                    AS overdue_total,
                EXTRACT(EPOCH FROM (now() - min(created_at) FILTER (WHERE status = 'pending')))::float8
                    AS oldest_pending_seconds
           FROM complaints",
    )
    .bind(me)
    .bind(sla_seconds as f64)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// 审核账号工作量与及时性；`only` 为 `Some` 时只返回该账号（审核账号看自己）。
///
/// 注意：这里**刻意不返回**通过/驳回数量与结论分布——审核工作只按及时性衡量。
pub async fn reviewer_workloads(
    pool: &PgPool,
    only: Option<&Uuid>,
    sla_seconds: i64,
) -> ApiResult<Vec<ReviewerWorkload>> {
    let rows = sqlx::query_as::<_, ReviewerWorkload>(
        "SELECT u.id AS reviewer_id,
                u.name AS reviewer_name,
                u.phone,
                u.is_active,
                COALESCE(l.active_locks, 0) AS active_locks,
                COALESCE(s.reviewed_total, 0) AS reviewed_total,
                COALESCE(s.reviewed_last_7d, 0) AS reviewed_last_7d,
                s.avg_total_seconds,
                s.median_total_seconds,
                s.p90_total_seconds,
                s.avg_handle_seconds,
                COALESCE(s.handle_sample, 0) AS handle_sample,
                COALESCE(s.on_time_total, 0) AS on_time_total,
                s.on_time_rate,
                s.last_reviewed_at
           FROM users u
           LEFT JOIN (
                 SELECT cp.reviewed_by,
                        count(*) AS reviewed_total,
                        count(*) FILTER (WHERE cp.reviewed_at >= now() - interval '7 days')
                            AS reviewed_last_7d,
                        avg(EXTRACT(EPOCH FROM (cp.reviewed_at - cp.created_at)))::float8
                            AS avg_total_seconds,
                        (percentile_cont(0.5) WITHIN GROUP (
                            ORDER BY EXTRACT(EPOCH FROM (cp.reviewed_at - cp.created_at))::float8
                        ))::float8 AS median_total_seconds,
                        (percentile_cont(0.9) WITHIN GROUP (
                            ORDER BY EXTRACT(EPOCH FROM (cp.reviewed_at - cp.created_at))::float8
                        ))::float8 AS p90_total_seconds,
                        (avg(EXTRACT(EPOCH FROM (cp.reviewed_at - cp.review_started_at)))
                            FILTER (WHERE cp.review_started_at IS NOT NULL))::float8
                            AS avg_handle_seconds,
                        count(*) FILTER (WHERE cp.review_started_at IS NOT NULL) AS handle_sample,
                        count(*) FILTER (
                            WHERE cp.reviewed_at - cp.created_at
                                  <= ($2::double precision * interval '1 second')
                        ) AS on_time_total,
                        (count(*) FILTER (
                            WHERE cp.reviewed_at - cp.created_at
                                  <= ($2::double precision * interval '1 second')
                        ))::float8 / NULLIF(count(*), 0) AS on_time_rate,
                        max(cp.reviewed_at) AS last_reviewed_at
                   FROM complaints cp
                  WHERE cp.reviewed_by IS NOT NULL AND cp.reviewed_at IS NOT NULL
                  GROUP BY cp.reviewed_by
             ) s ON s.reviewed_by = u.id
           LEFT JOIN (
                 SELECT cp.locked_by, count(*) AS active_locks
                   FROM complaints cp
                  WHERE cp.status = 'pending'
                    AND cp.locked_by IS NOT NULL
                    AND cp.lock_expires_at > now()
                  GROUP BY cp.locked_by
             ) l ON l.locked_by = u.id
          WHERE u.role = 'reviewer'
            AND ($1::uuid IS NULL OR u.id = $1)
          ORDER BY s.avg_total_seconds ASC NULLS LAST, u.name ASC",
    )
    .bind(only)
    .bind(sla_seconds as f64)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 用人单位质量画像：投诉结构 + 职位规模 + 等级序（按最差在前排序）
pub async fn company_quality(pool: &PgPool, limit: i64) -> ApiResult<Vec<CompanyQuality>> {
    let rows = sqlx::query_as::<_, CompanyQuality>(
        "SELECT c.id AS company_id,
                c.name,
                c.industry,
                c.location,
                COALESCE(cp.total, 0) AS complaints_total,
                COALESCE(cp.pending, 0) AS complaints_pending,
                COALESCE(cp.approved, 0) AS complaints_approved,
                COALESCE(cp.rejected, 0) AS complaints_rejected,
                COALESCE(cp.last_30d, 0) AS complaints_last_30d,
                COALESCE(j.total, 0) AS jobs_total,
                COALESCE(j.active, 0) AS jobs_active,
                complaint_level_rank(c.complaints_count) AS level_rank,
                CASE WHEN COALESCE(j.active, 0) > 0
                     THEN COALESCE(cp.approved, 0)::float8 / j.active::float8
                     ELSE NULL END AS complaint_rate,
                -- 质量分与等级同源：都用 companies.complaints_count（审核事务维护的权威计数），
                -- 避免出现「等级=严重 而 分数=100」这类自相矛盾的分析结论
                GREATEST(
                    0,
                    100 - c.complaints_count * 10 - COALESCE(cp.pending, 0)::int * 2
                ) AS quality_score,
                cp.first_at AS first_complaint_at,
                cp.last_at AS last_complaint_at
           FROM companies c
           LEFT JOIN (
                 SELECT company_id,
                        count(*) AS total,
                        count(*) FILTER (WHERE status = 'pending') AS pending,
                        count(*) FILTER (WHERE status = 'approved') AS approved,
                        count(*) FILTER (WHERE status = 'rejected') AS rejected,
                        count(*) FILTER (WHERE created_at >= now() - interval '30 days') AS last_30d,
                        min(created_at) AS first_at,
                        max(created_at) AS last_at
                   FROM complaints
                  GROUP BY company_id
             ) cp ON cp.company_id = c.id
           LEFT JOIN (
                 SELECT company_id,
                        count(*) AS total,
                        count(*) FILTER (WHERE is_active) AS active
                   FROM jobs
                  GROUP BY company_id
             ) j ON j.company_id = c.id
          ORDER BY complaint_level_rank(c.complaints_count) DESC,
                   COALESCE(cp.approved, 0) DESC,
                   COALESCE(cp.total, 0) DESC,
                   c.name ASC
          LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 求职用户分析（账号 + 参与度 + 注册趋势 + 投递结构）
pub async fn seeker_stats(pool: &PgPool) -> ApiResult<SeekerStats> {
    let overview = sqlx::query_as::<_, SeekerOverview>(
        "SELECT count(*) AS total,
                count(*) FILTER (WHERE is_active) AS active,
                count(*) FILTER (WHERE NOT is_active) AS disabled,
                count(*) FILTER (WHERE created_at >= now() - interval '7 days') AS new_last_7d,
                count(*) FILTER (WHERE created_at >= now() - interval '30 days') AS new_last_30d,
                (SELECT count(DISTINCT r.user_id) FROM resumes r) AS with_resume,
                (SELECT count(*) FROM resumes) AS resumes_total,
                (SELECT count(DISTINCT a.seeker_id) FROM applications a) AS with_application,
                (SELECT count(*) FROM applications) AS applications_total,
                (SELECT count(*) FROM interviews) AS interviews_total,
                (SELECT count(DISTINCT cp.complainant_id) FROM complaints cp) AS complainants,
                (SELECT count(*) FROM complaints) AS complaints_total
           FROM users
          WHERE role = 'seeker'",
    )
    .fetch_one(pool)
    .await?;

    let registrations_by_month = sqlx::query_as::<_, MonthlyCount>(
        "SELECT to_char(date_trunc('month', created_at), 'YYYY-MM') AS month,
                count(*) AS count
           FROM users
          WHERE role = 'seeker'
            AND created_at >= date_trunc('month', now()) - interval '11 months'
          GROUP BY 1
          ORDER BY 1 ASC",
    )
    .fetch_all(pool)
    .await?;

    let applications_by_status = sqlx::query_as::<_, StatusCount>(
        "SELECT status, count(*) AS count
           FROM applications
          GROUP BY status
          ORDER BY count DESC, status ASC",
    )
    .fetch_all(pool)
    .await?;

    let application_buckets = sqlx::query_as::<_, BucketCount>(
        "SELECT CASE WHEN cnt = 0 THEN '0'
                     WHEN cnt <= 2 THEN '1-2'
                     WHEN cnt <= 5 THEN '3-5'
                     ELSE '6+' END AS bucket,
                count(*) AS count
           FROM (
                SELECT u.id, count(a.id) AS cnt
                  FROM users u
                  LEFT JOIN applications a ON a.seeker_id = u.id
                 WHERE u.role = 'seeker'
                 GROUP BY u.id
                ) t
          GROUP BY 1
          ORDER BY 1 ASC",
    )
    .fetch_all(pool)
    .await?;

    let engagement_rate = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT (count(*) FILTER (WHERE r.user_id IS NOT NULL OR a.seeker_id IS NOT NULL))::float8
                / NULLIF(count(*), 0)
           FROM users u
           LEFT JOIN (SELECT DISTINCT user_id FROM resumes) r ON r.user_id = u.id
           LEFT JOIN (SELECT DISTINCT seeker_id FROM applications) a ON a.seeker_id = u.id
          WHERE u.role = 'seeker'",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(None);

    let complaints_per_complainant = if overview.complainants > 0 {
        Some(overview.complaints_total as f64 / overview.complainants as f64)
    } else {
        None
    };

    Ok(SeekerStats {
        overview,
        registrations_by_month,
        applications_by_status,
        application_buckets,
        engagement_rate,
        complaints_per_complainant,
    })
}
