//! 投诉数据访问。
//!
//! 查询统一走只读视图 `complaint_views`（联表企业/投诉人/审核人/持锁人并算出锁是否有效），
//! 认领、释放、审结都是**带条件的 UPDATE**，由数据库保证并发下只有一个审核能成功。

use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::models::complaint::ComplaintView;

/// 认领锁的租约（秒）：审核端在弹窗打开期间按此周期续约
pub const LOCK_TTL_SECONDS: i64 = 10 * 60;

async fn get_view(pool: &PgPool, id: &Uuid) -> ApiResult<Option<ComplaintView>> {
    let view = sqlx::query_as::<_, ComplaintView>("SELECT * FROM complaint_views WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(view)
}

/// 是否与目标企业有过“实际交流”：存在会话且该会话里至少一条消息，
/// 会话对端为企业任一招聘者账号。
pub async fn has_exchange(pool: &PgPool, complainant: &Uuid, company_id: &Uuid) -> ApiResult<bool> {
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
        "SELECT * FROM complaint_views
          WHERE complainant_id = $1
          ORDER BY created_at DESC",
    )
    .bind(complainant)
    .fetch_all(pool)
    .await?;
    Ok(views)
}

pub async fn for_company(pool: &PgPool, company_id: &Uuid) -> ApiResult<Vec<ComplaintView>> {
    let views = sqlx::query_as::<_, ComplaintView>(
        "SELECT * FROM complaint_views
          WHERE company_id = $1
          ORDER BY created_at DESC",
    )
    .bind(company_id)
    .fetch_all(pool)
    .await?;
    Ok(views)
}

pub async fn all(pool: &PgPool, status: Option<&str>) -> ApiResult<Vec<ComplaintView>> {
    let views = sqlx::query_as::<_, ComplaintView>(
        "SELECT * FROM complaint_views
          WHERE ($1::text IS NULL OR status = $1)
          ORDER BY created_at DESC",
    )
    .bind(status)
    .fetch_all(pool)
    .await?;
    Ok(views)
}

/// 认领（或续约）一条待审投诉。
///
/// 原子性：`UPDATE ... WHERE 空闲 OR 自己持有 OR 已过期`，并发下只有一个请求能命中；
/// 未命中时再读一次当前状态，给出「已被谁占用 / 已审结」的准确报错，而不是笼统的 409。
pub async fn claim(
    pool: &PgPool,
    id: &Uuid,
    reviewer_id: &Uuid,
    ttl_seconds: i64,
) -> ApiResult<ComplaintView> {
    let claimed = sqlx::query_scalar::<_, Uuid>(
        "UPDATE complaints
            SET locked_by = $2,
                locked_at = CASE WHEN locked_by = $2 THEN locked_at ELSE now() END,
                lock_expires_at = now() + ($3::double precision * interval '1 second'),
                review_started_at = COALESCE(review_started_at, now()),
                updated_at = now()
          WHERE id = $1
            AND status = 'pending'
            AND (locked_by IS NULL OR locked_by = $2 OR lock_expires_at <= now())
        RETURNING id",
    )
    .bind(id)
    .bind(reviewer_id)
    .bind(ttl_seconds as f64)
    .fetch_optional(pool)
    .await?;

    if claimed.is_some() {
        return get_view(pool, id)
            .await?
            .ok_or_else(|| AppError::internal("认领成功但读取失败"));
    }

    let Some(current) = get_view(pool, id).await? else {
        return Err(AppError::not_found(format!("complaint `{id}`")));
    };
    if current.status != "pending" {
        return Err(AppError::conflict("该投诉已审结，无需认领"));
    }
    Err(AppError::conflict(format!(
        "该投诉正由「{}」审核中，锁至 {}，可等待锁自动释放",
        current.locked_by_name.as_deref().unwrap_or("其他审核"),
        current
            .lock_expires_at
            .map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "-".to_string())
    )))
}

/// 释放锁：本人可释放自己的锁；已过期的锁任何人可清理；平台管理员可用 `force` 强制释放。
pub async fn release(
    pool: &PgPool,
    id: &Uuid,
    actor_id: &Uuid,
    force: bool,
) -> ApiResult<ComplaintView> {
    let released = sqlx::query_scalar::<_, Uuid>(
        "UPDATE complaints
            SET locked_by = NULL, locked_at = NULL, lock_expires_at = NULL, updated_at = now()
          WHERE id = $1
            AND locked_by IS NOT NULL
            AND ($2 OR locked_by = $3 OR lock_expires_at <= now())
        RETURNING id",
    )
    .bind(id)
    .bind(force)
    .bind(actor_id)
    .fetch_optional(pool)
    .await?;

    if released.is_none() {
        let Some(current) = get_view(pool, id).await? else {
            return Err(AppError::not_found(format!("complaint `{id}`")));
        };
        if current.locked_by.is_none() {
            return Err(AppError::conflict("该投诉当前没有被锁定"));
        }
        return Err(AppError::forbidden(
            "只能释放自己认领的投诉（平台管理员可用 force 强制释放）",
        ));
    }
    get_view(pool, id)
        .await?
        .ok_or_else(|| AppError::internal("释放成功但读取失败"))
}

/// 审结：通过时累计企业投诉次数；同时清空锁字段并保留 `review_started_at`。
///
/// 并发保护：条件 UPDATE 要求「仍是 pending 且没有被他人有效锁定」，
/// 因此即使两个审核同时提交，也只有一个会成功（另一个拿到 409）。
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
    if view.lock_active && view.locked_by != Some(*reviewer_id) {
        return Err(AppError::conflict(format!(
            "该投诉正由「{}」审核中，请等待对方完成或锁自动释放",
            view.locked_by_name.as_deref().unwrap_or("其他审核")
        )));
    }

    let mut tx: Transaction<'_, Postgres> = pool.begin().await?;
    let updated = sqlx::query_scalar::<_, Uuid>(
        "UPDATE complaints
            SET status = $2, review_note = $3, reviewed_by = $4,
                reviewed_at = now(), updated_at = now(),
                locked_by = NULL, locked_at = NULL, lock_expires_at = NULL,
                review_started_at = COALESCE(review_started_at, now())
          WHERE id = $1
            AND status = 'pending'
            AND (locked_by IS NULL OR locked_by = $4 OR lock_expires_at <= now())
        RETURNING id",
    )
    .bind(id)
    .bind(if approved { "approved" } else { "rejected" })
    .bind(note)
    .bind(reviewer_id)
    .fetch_optional(&mut *tx)
    .await?;

    if updated.is_none() {
        tx.rollback().await?;
        return Err(AppError::conflict("该投诉刚被其他审核处理，请刷新后重试"));
    }

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
