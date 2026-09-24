//! 投诉数据访问

use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::models::complaint::ComplaintView;

async fn get_view(pool: &PgPool, id: &Uuid) -> ApiResult<Option<ComplaintView>> {
    let view = sqlx::query_as::<_, ComplaintView>(
        "SELECT cp.id, cp.company_id, c.name AS company_name,
                cp.complainant_id, u.name AS complainant_name,
                cp.evidence, cp.status, cp.review_note, cp.reviewed_by,
                reviewer.name AS reviewed_by_name,
                cp.created_at, cp.reviewed_at, cp.updated_at
           FROM complaints cp
           JOIN companies c ON c.id = cp.company_id
           JOIN users u ON u.id = cp.complainant_id
           LEFT JOIN users reviewer ON reviewer.id = cp.reviewed_by
          WHERE cp.id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(view)
}

/// 是否与目标企业有过“实际交流”：存在会话且该会话里至少一条消息，
/// 会话对端为企业任一招聘者账号。
pub async fn has_exchange(
    pool: &PgPool,
    complainant: &Uuid,
    company_id: &Uuid,
) -> ApiResult<bool> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
           SELECT 1
             FROM conversations cv
             JOIN messages m ON m.conversation_id = cv.id
            WHERE (cv.user_lo = $1 OR cv.user_hi = $1)
              AND EXISTS (
                    SELECT 1 FROM users u
                     WHERE u.id IN (cv.user_lo, cv.user_hi)
                       AND u.id <> $1
                       AND u.company_id = $2)
         )",
    )
    .bind(complainant)
    .bind(company_id)
    .fetch_one(pool)
    .await?;
    Ok(exists)
}

pub async fn create(
    pool: &PgPool,
    company_id: &Uuid,
    complainant_id: &Uuid,
    evidence: &str,
) -> ApiResult<ComplaintView> {
    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO complaints (company_id, complainant_id, evidence)
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(company_id)
    .bind(complainant_id)
    .bind(evidence)
    .fetch_one(pool)
    .await?;
    get_view(pool, &id)
        .await?
        .ok_or_else(|| AppError::internal("投诉创建成功但读取失败"))
}

pub async fn mine(pool: &PgPool, complainant: &Uuid) -> ApiResult<Vec<ComplaintView>> {
    let views = sqlx::query_as::<_, ComplaintView>(
        "SELECT cp.id, cp.company_id, c.name AS company_name,
                cp.complainant_id, u.name AS complainant_name,
                cp.evidence, cp.status, cp.review_note, cp.reviewed_by,
                reviewer.name AS reviewed_by_name,
                cp.created_at, cp.reviewed_at, cp.updated_at
           FROM complaints cp
           JOIN companies c ON c.id = cp.company_id
           JOIN users u ON u.id = cp.complainant_id
           LEFT JOIN users reviewer ON reviewer.id = cp.reviewed_by
          WHERE cp.complainant_id = $1
          ORDER BY cp.created_at DESC",
    )
    .bind(complainant)
    .fetch_all(pool)
    .await?;
    Ok(views)
}

pub async fn for_company(pool: &PgPool, company_id: &Uuid) -> ApiResult<Vec<ComplaintView>> {
    let views = sqlx::query_as::<_, ComplaintView>(
        "SELECT cp.id, cp.company_id, c.name AS company_name,
                cp.complainant_id, u.name AS complainant_name,
                cp.evidence, cp.status, cp.review_note, cp.reviewed_by,
                reviewer.name AS reviewed_by_name,
                cp.created_at, cp.reviewed_at, cp.updated_at
           FROM complaints cp
           JOIN companies c ON c.id = cp.company_id
           JOIN users u ON u.id = cp.complainant_id
           LEFT JOIN users reviewer ON reviewer.id = cp.reviewed_by
          WHERE cp.company_id = $1
          ORDER BY cp.created_at DESC",
    )
    .bind(company_id)
    .fetch_all(pool)
    .await?;
    Ok(views)
}

pub async fn all(pool: &PgPool, status: Option<&str>) -> ApiResult<Vec<ComplaintView>> {
    let views = sqlx::query_as::<_, ComplaintView>(
        "SELECT cp.id, cp.company_id, c.name AS company_name,
                cp.complainant_id, u.name AS complainant_name,
                cp.evidence, cp.status, cp.review_note, cp.reviewed_by,
                reviewer.name AS reviewed_by_name,
                cp.created_at, cp.reviewed_at, cp.updated_at
           FROM complaints cp
           JOIN companies c ON c.id = cp.company_id
           JOIN users u ON u.id = cp.complainant_id
           LEFT JOIN users reviewer ON reviewer.id = cp.reviewed_by
          WHERE ($1::text IS NULL OR cp.status = $1)
          ORDER BY cp.created_at DESC",
    )
    .bind(status)
    .fetch_all(pool)
    .await?;
    Ok(views)
}

/// 审核：事务内更新状态；通过时累计企业投诉次数
pub async fn review(
    pool: &PgPool,
    id: &Uuid,
    reviewer_id: &Uuid,
    approved: bool,
    note: Option<&str>,
) -> ApiResult<Option<ComplaintView>> {
    let row = get_view(pool, id).await?;
    let Some(view) = row else { return Ok(None) };
    if view.status != "pending" {
        return Err(AppError::conflict("该投诉已处理，请勿重复审核"));
    }

    let mut tx: Transaction<'_, Postgres> = pool.begin().await?;
    sqlx::query(
        "UPDATE complaints
            SET status = $2, review_note = $3, reviewed_by = $4,
                reviewed_at = now(), updated_at = now()
          WHERE id = $1",
    )
    .bind(id)
    .bind(if approved { "approved" } else { "rejected" })
    .bind(note)
    .bind(reviewer_id)
    .execute(&mut *tx)
    .await?;

    if approved {
        sqlx::query(
            "UPDATE companies
                SET complaints_count = complaints_count + 1,
                    updated_at = now()
              WHERE id = $1",
        )
        .bind(view.company_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    get_view(pool, id).await
}
