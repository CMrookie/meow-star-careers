//! 简历数据访问

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;
use crate::models::pagination::paginate;
use crate::models::resume::{Resume, ResumePage, ResumeQuery, ResumeWrite};

/// 创建简历（full_name / title 由 handler 校验必填）
pub async fn create(pool: &PgPool, user_id: &Uuid, input: &ResumeWrite) -> ApiResult<Resume> {
    let resume = sqlx::query_as::<_, Resume>(
        "INSERT INTO resumes (user_id, full_name, title, phone, email, years,
                              education, skills, summary, is_public)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING id, user_id, full_name, title, phone, email, years,
                   education, skills, summary, is_public, created_at, updated_at",
    )
    .bind(user_id)
    .bind(input.full_name.as_deref().unwrap_or_default())
    .bind(input.title.as_deref().unwrap_or_default())
    .bind(input.phone.as_deref())
    .bind(input.email.as_deref())
    .bind(input.years)
    .bind(input.education.as_deref())
    .bind(input.skills.as_deref())
    .bind(input.summary.as_deref())
    .bind(input.is_public.unwrap_or(false))
    .fetch_one(pool)
    .await?;
    Ok(resume)
}

/// 我的简历列表
pub async fn list_mine(pool: &PgPool, user_id: &Uuid) -> ApiResult<Vec<Resume>> {
    let resumes = sqlx::query_as::<_, Resume>(
        "SELECT id, user_id, full_name, title, phone, email, years,
                education, skills, summary, is_public, created_at, updated_at
           FROM resumes
          WHERE user_id = $1
          ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(resumes)
}

/// 按 id 查询简历（任意可见性；可见性判断由 handler 做）
pub async fn get(pool: &PgPool, id: &Uuid) -> ApiResult<Option<Resume>> {
    let resume = sqlx::query_as::<_, Resume>(
        "SELECT id, user_id, full_name, title, phone, email, years,
                education, skills, summary, is_public, created_at, updated_at
           FROM resumes
          WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(resume)
}

/// 更新自己的简历（仅更新提供的字段），返回 None 表示不存在/非本人
pub async fn update(
    pool: &PgPool,
    id: &Uuid,
    user_id: &Uuid,
    input: &ResumeWrite,
) -> ApiResult<Option<Resume>> {
    let updated = sqlx::query(
        "UPDATE resumes
            SET full_name = COALESCE($3, full_name),
                title     = COALESCE($4, title),
                phone     = COALESCE($5, phone),
                email     = COALESCE($6, email),
                years     = COALESCE($7, years),
                education = COALESCE($8, education),
                skills    = COALESCE($9, skills),
                summary   = COALESCE($10, summary),
                is_public = COALESCE($11, is_public),
                updated_at = now()
          WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .bind(input.full_name.as_deref())
    .bind(input.title.as_deref())
    .bind(input.phone.as_deref())
    .bind(input.email.as_deref())
    .bind(input.years)
    .bind(input.education.as_deref())
    .bind(input.skills.as_deref())
    .bind(input.summary.as_deref())
    .bind(input.is_public)
    .execute(pool)
    .await?;

    if updated.rows_affected() == 0 {
        return Ok(None);
    }
    get(pool, id).await
}

/// 删除自己的简历
pub async fn delete(pool: &PgPool, id: &Uuid, user_id: &Uuid) -> ApiResult<bool> {
    let result = sqlx::query("DELETE FROM resumes WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// 招聘者检索公开简历
pub async fn search_public(pool: &PgPool, query: &ResumeQuery) -> ApiResult<ResumePage> {
    let (page, page_size, offset) = paginate(query.page, query.page_size);

    let total = sqlx::query_scalar::<_, i64>(
        "SELECT count(*)
           FROM resumes
          WHERE is_public = true
            AND ($1::text IS NULL
                 OR full_name ILIKE '%' || $1 || '%'
                 OR title ILIKE '%' || $1 || '%'
                 OR skills ILIKE '%' || $1 || '%'
                 OR summary ILIKE '%' || $1 || '%')",
    )
    .bind(query.keyword.as_deref())
    .fetch_one(pool)
    .await?;

    let items = sqlx::query_as::<_, Resume>(
        "SELECT id, user_id, full_name, title, phone, email, years,
                education, skills, summary, is_public, created_at, updated_at
           FROM resumes
          WHERE is_public = true
            AND ($1::text IS NULL
                 OR full_name ILIKE '%' || $1 || '%'
                 OR title ILIKE '%' || $1 || '%'
                 OR skills ILIKE '%' || $1 || '%'
                 OR summary ILIKE '%' || $1 || '%')
          ORDER BY updated_at DESC
          LIMIT $2 OFFSET $3",
    )
    .bind(query.keyword.as_deref())
    .bind(page_size)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(ResumePage {
        items,
        total,
        page,
        page_size,
    })
}
