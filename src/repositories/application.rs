//! 投递数据访问

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::models::application::{ApplicationPage, ApplicationQuery, ApplicationView};
use crate::models::pagination::paginate;

/// 该求职者是否已投递某职位（返回投递 id）
pub async fn find_by_job_and_seeker(
    pool: &PgPool,
    job_id: &Uuid,
    seeker_id: &Uuid,
) -> ApiResult<Option<ApplicationView>> {
    let app = sqlx::query_as::<_, ApplicationView>(
        "SELECT a.id, a.job_id, j.title AS job_title, c.id AS company_id, c.name AS company_name,
                a.seeker_id, u.name AS seeker_name, u.email AS seeker_email, u.phone AS seeker_phone,
                a.resume_id, a.cover_letter, a.status, a.created_at, a.updated_at
           FROM applications a
           JOIN jobs j ON j.id = a.job_id
           JOIN companies c ON c.id = j.company_id
           JOIN users u ON u.id = a.seeker_id
          WHERE a.job_id = $1 AND a.seeker_id = $2",
    )
    .bind(job_id)
    .bind(seeker_id)
    .fetch_optional(pool)
    .await?;
    Ok(app)
}

/// 创建投递并返回联表视图
pub async fn apply(
    pool: &PgPool,
    job_id: &Uuid,
    seeker_id: &Uuid,
    resume_id: Option<Uuid>,
    cover_letter: Option<&str>,
) -> ApiResult<ApplicationView> {
    // 引用的简历必须属于本人
    if let Some(resume_id) = resume_id {
        let owned = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM resumes WHERE id = $1 AND user_id = $2)",
        )
        .bind(resume_id)
        .bind(seeker_id)
        .fetch_one(pool)
        .await?;
        if !owned {
            return Err(AppError::forbidden("简历不属于当前用户"));
        }
    }

    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO applications (job_id, seeker_id, resume_id, cover_letter)
         VALUES ($1, $2, $3, $4)
         RETURNING id",
    )
    .bind(job_id)
    .bind(seeker_id)
    .bind(resume_id)
    .bind(cover_letter)
    .fetch_one(pool)
    .await?;

    get_view(pool, &id)
        .await?
        .ok_or_else(|| AppError::internal("投递成功但读取失败"))
}

/// 单个投递视图
pub async fn get_view(pool: &PgPool, id: &Uuid) -> ApiResult<Option<ApplicationView>> {
    let app = sqlx::query_as::<_, ApplicationView>(
        "SELECT a.id, a.job_id, j.title AS job_title, c.id AS company_id, c.name AS company_name,
                a.seeker_id, u.name AS seeker_name, u.email AS seeker_email, u.phone AS seeker_phone,
                a.resume_id, a.cover_letter, a.status, a.created_at, a.updated_at
           FROM applications a
           JOIN jobs j ON j.id = a.job_id
           JOIN companies c ON c.id = j.company_id
           JOIN users u ON u.id = a.seeker_id
          WHERE a.id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(app)
}

/// 求职者的投递列表（可按职位/状态过滤）
pub async fn list_for_seeker(
    pool: &PgPool,
    seeker_id: &Uuid,
    query: &ApplicationQuery,
) -> ApiResult<ApplicationPage> {
    let (page, page_size, offset) = paginate(query.page, query.page_size);

    let total = sqlx::query_scalar::<_, i64>(
        "SELECT count(*)
           FROM applications
          WHERE seeker_id = $1
            AND ($2::uuid IS NULL OR job_id = $2)
            AND ($3::text IS NULL OR status = $3)",
    )
    .bind(seeker_id)
    .bind(query.job_id)
    .bind(query.status.as_deref())
    .fetch_one(pool)
    .await?;

    let items = sqlx::query_as::<_, ApplicationView>(
        "SELECT a.id, a.job_id, j.title AS job_title, c.id AS company_id, c.name AS company_name,
                a.seeker_id, u.name AS seeker_name, u.email AS seeker_email, u.phone AS seeker_phone,
                a.resume_id, a.cover_letter, a.status, a.created_at, a.updated_at
           FROM applications a
           JOIN jobs j ON j.id = a.job_id
           JOIN companies c ON c.id = j.company_id
           JOIN users u ON u.id = a.seeker_id
          WHERE a.seeker_id = $1
            AND ($2::uuid IS NULL OR a.job_id = $2)
            AND ($3::text IS NULL OR a.status = $3)
          ORDER BY a.created_at DESC
          LIMIT $4 OFFSET $5",
    )
    .bind(seeker_id)
    .bind(query.job_id)
    .bind(query.status.as_deref())
    .bind(page_size)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(ApplicationPage {
        items,
        total,
        page,
        page_size,
    })
}

/// 企业收到的投递列表（本企业职位下）
pub async fn list_for_company(
    pool: &PgPool,
    company_id: &Uuid,
    query: &ApplicationQuery,
) -> ApiResult<ApplicationPage> {
    let (page, page_size, offset) = paginate(query.page, query.page_size);

    let total = sqlx::query_scalar::<_, i64>(
        "SELECT count(*)
           FROM applications a
           JOIN jobs j ON j.id = a.job_id
          WHERE j.company_id = $1
            AND ($2::uuid IS NULL OR a.job_id = $2)
            AND ($3::text IS NULL OR a.status = $3)",
    )
    .bind(company_id)
    .bind(query.job_id)
    .bind(query.status.as_deref())
    .fetch_one(pool)
    .await?;

    let items = sqlx::query_as::<_, ApplicationView>(
        "SELECT a.id, a.job_id, j.title AS job_title, c.id AS company_id, c.name AS company_name,
                a.seeker_id, u.name AS seeker_name, u.email AS seeker_email, u.phone AS seeker_phone,
                a.resume_id, a.cover_letter, a.status, a.created_at, a.updated_at
           FROM applications a
           JOIN jobs j ON j.id = a.job_id
           JOIN companies c ON c.id = j.company_id
           JOIN users u ON u.id = a.seeker_id
          WHERE j.company_id = $1
            AND ($2::uuid IS NULL OR a.job_id = $2)
            AND ($3::text IS NULL OR a.status = $3)
          ORDER BY a.created_at DESC
          LIMIT $4 OFFSET $5",
    )
    .bind(company_id)
    .bind(query.job_id)
    .bind(query.status.as_deref())
    .bind(page_size)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(ApplicationPage {
        items,
        total,
        page,
        page_size,
    })
}

/// 更新投递状态，返回更新后的视图
pub async fn update_status(
    pool: &PgPool,
    id: &Uuid,
    status: &str,
) -> ApiResult<Option<ApplicationView>> {
    let updated = sqlx::query(
        "UPDATE applications SET status = $2, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(status)
    .execute(pool)
    .await?;

    if updated.rows_affected() == 0 {
        return Ok(None);
    }
    get_view(pool, id).await
}
