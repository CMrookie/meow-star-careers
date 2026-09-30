//! 职位数据访问：公开搜索/详情、招聘者管理、收藏。
//! 所有 SQL 为编译期字面量（sqlx 0.9 要求）。
//!
//! 列表排序契约：三个职位列表（公开搜索 / 本企业 / 我的收藏）**一律按用人单位投诉等级
//! 从优到劣**返回。排序键与求职 App 的 `jobSortKey` 逐项对齐（定级 v2，见
//! `migrations/0017_company_staff_size.sql`）：
//!
//! 1. `complaint_level_rank(complaints_count, staff_size)` —— 有申报规模且 >=50 人时按
//!    **每百人投诉率**（0.5/1.5/3.0%），否则退回投诉次数口径；
//! 2. 同等级内有规模折算的排前面（数据更可信）；
//! 3. 再按率（无规模时按次数）由小到大；
//! 4. 最后按时间倒序、`id` 兜底，保证分页稳定、跨页全局有序。
//!
//! 排序在**服务端**完成，客户端无需再排序。

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::models::job::{JobQuery, JobView, NewJob, UpdateJob};
use crate::models::pagination::paginate;

/// 公开搜索在招职位（仅 is_active = true）
pub async fn search_active(pool: &PgPool, query: &JobQuery) -> ApiResult<(Vec<JobView>, i64)> {
    let (_page, page_size, offset) = paginate(query.page, query.page_size);

    let total = sqlx::query_scalar::<_, i64>(
        "SELECT count(*)
           FROM jobs j
          WHERE j.is_active = true
            AND ($1::text IS NULL
                 OR j.title ILIKE '%' || $1 || '%'
                 OR j.description ILIKE '%' || $1 || '%'
                 OR j.requirements ILIKE '%' || $1 || '%')
            AND ($2::text IS NULL OR j.location ILIKE '%' || $2 || '%')
            AND ($3::text IS NULL OR j.job_type = $3)
            AND ($4::int IS NULL OR COALESCE(j.salary_max, 2147483647) >= $4)
            AND ($5::int IS NULL OR COALESCE(j.salary_min, 0) <= $5)",
    )
    .bind(query.keyword.as_deref())
    .bind(query.location.as_deref())
    .bind(query.job_type.as_deref())
    .bind(query.salary_min)
    .bind(query.salary_max)
    .fetch_one(pool)
    .await?;

    let items = sqlx::query_as::<_, JobView>(
        "SELECT j.id, j.company_id, c.name AS company_name, j.title, j.description,
                j.requirements, j.location, j.salary_min, j.salary_max, j.job_type,
                j.experience, j.education, j.is_active, j.created_by, j.created_at, j.updated_at,
                c.complaints_count, c.staff_size AS company_staff_size,
                complaint_level(c.complaints_count, c.staff_size) AS complaint_level,
                complaint_basis(c.complaints_count, c.staff_size) AS complaint_basis,
                complaint_rate_percent(c.complaints_count, c.staff_size)::float8 AS complaint_rate_percent
           FROM jobs j
           JOIN companies c ON c.id = j.company_id
          WHERE j.is_active = true
            AND ($1::text IS NULL
                 OR j.title ILIKE '%' || $1 || '%'
                 OR j.description ILIKE '%' || $1 || '%'
                 OR j.requirements ILIKE '%' || $1 || '%')
            AND ($2::text IS NULL OR j.location ILIKE '%' || $2 || '%')
            AND ($3::text IS NULL OR j.job_type = $3)
            AND ($4::int IS NULL OR COALESCE(j.salary_max, 2147483647) >= $4)
            AND ($5::int IS NULL OR COALESCE(j.salary_min, 0) <= $5)
          ORDER BY complaint_level_rank(c.complaints_count, c.staff_size) ASC,  -- 定级 v2：有规模按每百人投诉率
                   (complaint_rate_percent(c.complaints_count, c.staff_size) IS NULL) ASC,  -- 有规模折算的排前面
                   COALESCE(complaint_rate_percent(c.complaints_count, c.staff_size),
                            c.complaints_count::numeric) ASC,           -- 同级内率（或次数）小者优先
                   j.created_at DESC,                                   -- 再按发布时间倒序
                   j.id DESC                                            -- 末位兜底，保证分页稳定
          LIMIT $6 OFFSET $7",
    )
    .bind(query.keyword.as_deref())
    .bind(query.location.as_deref())
    .bind(query.job_type.as_deref())
    .bind(query.salary_min)
    .bind(query.salary_max)
    .bind(page_size)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok((items, total))
}

/// 招聘者视角：本公司职位（含已下架）
pub async fn list_for_company(
    pool: &PgPool,
    company_id: &Uuid,
    page: Option<i64>,
    page_size: Option<i64>,
) -> ApiResult<(Vec<JobView>, i64)> {
    let (_page, size, offset) = paginate(page, page_size);

    let total = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM jobs WHERE company_id = $1",
    )
    .bind(company_id)
    .fetch_one(pool)
    .await?;

    let items = sqlx::query_as::<_, JobView>(
        "SELECT j.id, j.company_id, c.name AS company_name, j.title, j.description,
                j.requirements, j.location, j.salary_min, j.salary_max, j.job_type,
                j.experience, j.education, j.is_active, j.created_by, j.created_at, j.updated_at,
                c.complaints_count, c.staff_size AS company_staff_size,
                complaint_level(c.complaints_count, c.staff_size) AS complaint_level,
                complaint_basis(c.complaints_count, c.staff_size) AS complaint_basis,
                complaint_rate_percent(c.complaints_count, c.staff_size)::float8 AS complaint_rate_percent
           FROM jobs j
           JOIN companies c ON c.id = j.company_id
          WHERE j.company_id = $1
          ORDER BY complaint_level_rank(c.complaints_count, c.staff_size) ASC,  -- 同一企业等级相同，实际退化为按发布时间倒序
                   (complaint_rate_percent(c.complaints_count, c.staff_size) IS NULL) ASC,
                   COALESCE(complaint_rate_percent(c.complaints_count, c.staff_size),
                            c.complaints_count::numeric) ASC,
                   j.created_at DESC,
                   j.id DESC
          LIMIT $2 OFFSET $3",
    )
    .bind(company_id)
    .bind(size)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok((items, total))
}

/// 任意状态职位（招聘者校验所有权后使用）
pub async fn get_by_id(pool: &PgPool, id: &Uuid) -> ApiResult<Option<JobView>> {
    let job = sqlx::query_as::<_, JobView>(
        "SELECT j.id, j.company_id, c.name AS company_name, j.title, j.description,
                j.requirements, j.location, j.salary_min, j.salary_max, j.job_type,
                j.experience, j.education, j.is_active, j.created_by, j.created_at, j.updated_at,
                c.complaints_count, c.staff_size AS company_staff_size,
                complaint_level(c.complaints_count, c.staff_size) AS complaint_level,
                complaint_basis(c.complaints_count, c.staff_size) AS complaint_basis,
                complaint_rate_percent(c.complaints_count, c.staff_size)::float8 AS complaint_rate_percent
           FROM jobs j
           JOIN companies c ON c.id = j.company_id
          WHERE j.id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(job)
}

/// 公开职位详情（仅上架中）
pub async fn get_active(pool: &PgPool, id: &Uuid) -> ApiResult<Option<JobView>> {
    let job = sqlx::query_as::<_, JobView>(
        "SELECT j.id, j.company_id, c.name AS company_name, j.title, j.description,
                j.requirements, j.location, j.salary_min, j.salary_max, j.job_type,
                j.experience, j.education, j.is_active, j.created_by, j.created_at, j.updated_at,
                c.complaints_count, c.staff_size AS company_staff_size,
                complaint_level(c.complaints_count, c.staff_size) AS complaint_level,
                complaint_basis(c.complaints_count, c.staff_size) AS complaint_basis,
                complaint_rate_percent(c.complaints_count, c.staff_size)::float8 AS complaint_rate_percent
           FROM jobs j
           JOIN companies c ON c.id = j.company_id
          WHERE j.id = $1 AND j.is_active = true",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(job)
}

/// 发布职位
pub async fn create(
    pool: &PgPool,
    company_id: &Uuid,
    created_by: &Uuid,
    input: &NewJob,
) -> ApiResult<JobView> {
    let job_type = input.job_type.clone().unwrap_or_else(|| "full_time".to_string());

    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO jobs (company_id, title, description, requirements, location,
                           salary_min, salary_max, job_type, experience, education, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         RETURNING id",
    )
    .bind(company_id)
    .bind(input.title.trim())
    .bind(input.description.trim())
    .bind(input.requirements.as_deref())
    .bind(input.location.as_deref())
    .bind(input.salary_min)
    .bind(input.salary_max)
    .bind(job_type.as_str())
    .bind(input.experience.as_deref())
    .bind(input.education.as_deref())
    .bind(created_by)
    .fetch_one(pool)
    .await?;

    get_by_id(pool, &id)
        .await?
        .ok_or_else(|| AppError::internal("职位创建成功但读取失败"))
}

/// 更新职位内容（仅更新提供的字段）
pub async fn update(pool: &PgPool, id: &Uuid, input: &UpdateJob) -> ApiResult<Option<JobView>> {
    let updated = sqlx::query(
        "UPDATE jobs
            SET title        = COALESCE($2, title),
                description  = COALESCE($3, description),
                requirements = COALESCE($4, requirements),
                location     = COALESCE($5, location),
                salary_min   = COALESCE($6, salary_min),
                salary_max   = COALESCE($7, salary_max),
                job_type     = COALESCE($8, job_type),
                experience   = COALESCE($9, experience),
                education    = COALESCE($10, education),
                updated_at   = now()
          WHERE id = $1",
    )
    .bind(id)
    .bind(input.title.as_deref())
    .bind(input.description.as_deref())
    .bind(input.requirements.as_deref())
    .bind(input.location.as_deref())
    .bind(input.salary_min)
    .bind(input.salary_max)
    .bind(input.job_type.as_deref())
    .bind(input.experience.as_deref())
    .bind(input.education.as_deref())
    .execute(pool)
    .await?;

    if updated.rows_affected() == 0 {
        return Ok(None);
    }
    get_by_id(pool, id).await
}

/// 上架/下架
pub async fn set_active(pool: &PgPool, id: &Uuid, active: bool) -> ApiResult<bool> {
    let result = sqlx::query(
        "UPDATE jobs SET is_active = $2, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(active)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// 删除职位（级联清理投递）
pub async fn delete(pool: &PgPool, id: &Uuid) -> ApiResult<bool> {
    let result = sqlx::query("DELETE FROM jobs WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

// ---------------------------------------------------------------- 收藏

pub async fn save(pool: &PgPool, user_id: &Uuid, job_id: &Uuid) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO saved_jobs (user_id, job_id) VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(job_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn unsave(pool: &PgPool, user_id: &Uuid, job_id: &Uuid) -> ApiResult<bool> {
    let result = sqlx::query("DELETE FROM saved_jobs WHERE user_id = $1 AND job_id = $2")
        .bind(user_id)
        .bind(job_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn list_saved(
    pool: &PgPool,
    user_id: &Uuid,
    page: Option<i64>,
    page_size: Option<i64>,
) -> ApiResult<(Vec<JobView>, i64)> {
    let (_page, size, offset) = paginate(page, page_size);

    let total = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM saved_jobs WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    let items = sqlx::query_as::<_, JobView>(
        "SELECT j.id, j.company_id, c.name AS company_name, j.title, j.description,
                j.requirements, j.location, j.salary_min, j.salary_max, j.job_type,
                j.experience, j.education, j.is_active, j.created_by, j.created_at, j.updated_at,
                c.complaints_count, c.staff_size AS company_staff_size,
                complaint_level(c.complaints_count, c.staff_size) AS complaint_level,
                complaint_basis(c.complaints_count, c.staff_size) AS complaint_basis,
                complaint_rate_percent(c.complaints_count, c.staff_size)::float8 AS complaint_rate_percent
           FROM saved_jobs s
           JOIN jobs j ON j.id = s.job_id
           JOIN companies c ON c.id = j.company_id
          WHERE s.user_id = $1
          ORDER BY complaint_level_rank(c.complaints_count, c.staff_size) ASC,  -- 定级 v2：有规模按每百人投诉率
                   (complaint_rate_percent(c.complaints_count, c.staff_size) IS NULL) ASC,
                   COALESCE(complaint_rate_percent(c.complaints_count, c.staff_size),
                            c.complaints_count::numeric) ASC,
                   s.created_at DESC,                             -- 同级内最近收藏的在前
                   j.id DESC
          LIMIT $2 OFFSET $3",
    )
    .bind(user_id)
    .bind(size)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok((items, total))
}
