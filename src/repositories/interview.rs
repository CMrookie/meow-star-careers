//! 线上面试数据访问
//! 注意：sqlx 0.9 要求 SQL 为编译期字面量，因此每个查询都显式写全列清单。

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::models::application::ApplicationView;
use crate::models::interview::InterviewView;


#[derive(sqlx::FromRow)]
struct ParticipantRow {
    interviewer_id: Uuid,
    interviewee_id: Uuid,
}

/// 某用户在该面试中的对方（信令转发用）；非参与者返回 None
pub async fn peer_of(
    pool: &PgPool,
    interview_id: &Uuid,
    user_id: &Uuid,
) -> ApiResult<Option<Uuid>> {
    let row = sqlx::query_as::<_, ParticipantRow>(
        "SELECT interviewer_id, interviewee_id FROM interviews WHERE id = $1",
    )
    .bind(interview_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else { return Ok(None) };
    if row.interviewer_id == *user_id {
        Ok(Some(row.interviewee_id))
    } else if row.interviewee_id == *user_id {
        Ok(Some(row.interviewer_id))
    } else {
        Ok(None)
    }
}

async fn get_view(pool: &PgPool, id: &Uuid) -> ApiResult<Option<InterviewView>> {
    let view = sqlx::query_as::<_, InterviewView>(
        "SELECT i.id, i.application_id,
                j.id AS job_id, j.title AS job_title,
                c.id AS company_id, c.name AS company_name,
                hr.id AS interviewer_id, hr.name AS interviewer_name,
                sk.id AS interviewee_id, sk.name AS interviewee_name,
                i.status, i.scheduled_at, i.started_at, i.ended_at,
                i.created_at, i.updated_at
           FROM interviews i
           JOIN applications a ON a.id = i.application_id
           JOIN jobs j ON j.id = a.job_id
           JOIN companies c ON c.id = j.company_id
           JOIN users hr ON hr.id = i.interviewer_id
           JOIN users sk ON sk.id = i.interviewee_id
          WHERE i.id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(view)
}

/// 发起面试（招聘者对其企业收到的投递）；同一投递活跃面试唯一
pub async fn create(
    pool: &PgPool,
    application: &ApplicationView,
    interviewer_id: &Uuid,
    scheduled_at: Option<chrono::DateTime<Utc>>,
) -> ApiResult<InterviewView> {
    let active = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
           SELECT 1 FROM interviews
            WHERE application_id = $1 AND status IN ('invited','in_progress'))",
    )
    .bind(application.id)
    .fetch_one(pool)
    .await?;
    if active {
        return Err(AppError::conflict("该投递已有一个进行中的面试"));
    }

    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO interviews
            (application_id, job_id, company_id, interviewer_id, interviewee_id,
             status, scheduled_at, created_by)
         VALUES ($1, $2, $3, $4, $5, 'invited', $6, $4)
         RETURNING id",
    )
    .bind(application.id)
    .bind(application.job_id)
    .bind(application.company_id)
    .bind(interviewer_id)
    .bind(application.seeker_id)
    .bind(scheduled_at)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            AppError::conflict("该投递已有一个进行中的面试")
        }
        other => other.into(),
    })?;

    get_view(pool, &id)
        .await?
        .ok_or_else(|| AppError::internal("面试创建成功但读取失败"))
}

pub async fn get(pool: &PgPool, id: &Uuid) -> ApiResult<Option<InterviewView>> {
    get_view(pool, id).await
}

/// 我参与的面试（招聘者或求职者），最新优先
pub async fn mine(pool: &PgPool, user_id: &Uuid) -> ApiResult<Vec<InterviewView>> {
    let views = sqlx::query_as::<_, InterviewView>(
        "SELECT i.id, i.application_id,
                j.id AS job_id, j.title AS job_title,
                c.id AS company_id, c.name AS company_name,
                hr.id AS interviewer_id, hr.name AS interviewer_name,
                sk.id AS interviewee_id, sk.name AS interviewee_name,
                i.status, i.scheduled_at, i.started_at, i.ended_at,
                i.created_at, i.updated_at
           FROM interviews i
           JOIN applications a ON a.id = i.application_id
           JOIN jobs j ON j.id = a.job_id
           JOIN companies c ON c.id = j.company_id
           JOIN users hr ON hr.id = i.interviewer_id
           JOIN users sk ON sk.id = i.interviewee_id
          WHERE hr.id = $1 OR sk.id = $1
          ORDER BY i.updated_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(views)
}

fn member_guard(view: &InterviewView, user_id: &Uuid, allowed: &[&str], action: &str) -> ApiResult<()> {
    let is_member = view.interviewer_id == *user_id || view.interviewee_id == *user_id;
    if !is_member {
        return Err(AppError::forbidden("非面试参与者"));
    }
    if !allowed.contains(&view.status.as_str()) {
        return Err(AppError::bad_request(format!(
            "当前状态（{}）不允许{action}",
            view.status
        )));
    }
    Ok(())
}

async fn reload_after_update(pool: &PgPool, id: &Uuid) -> ApiResult<Option<InterviewView>> {
    get_view(pool, id).await
}

pub async fn start(pool: &PgPool, id: &Uuid, user_id: &Uuid) -> ApiResult<Option<InterviewView>> {
    let view = match get_view(pool, id).await? {
        Some(v) => v,
        None => return Ok(None),
    };
    member_guard(&view, user_id, &["invited"], "开始面试")?;
    if view
        .scheduled_at
        .is_some_and(|t| t > Utc::now() + chrono::Duration::minutes(1))
    {
        return Err(AppError::bad_request("未到预约时间，暂不能进入面试"));
    }
    let updated = sqlx::query_scalar::<_, Uuid>(
        "UPDATE interviews
            SET status = 'in_progress', started_at = now(), updated_at = now()
          WHERE id = $1
          RETURNING id",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    match updated {
        Some(_) => reload_after_update(pool, id).await,
        None => Ok(None),
    }
}

pub async fn finish(pool: &PgPool, id: &Uuid, user_id: &Uuid) -> ApiResult<Option<InterviewView>> {
    let view = match get_view(pool, id).await? {
        Some(v) => v,
        None => return Ok(None),
    };
    member_guard(&view, user_id, &["invited", "in_progress"], "结束面试")?;
    let updated = sqlx::query_scalar::<_, Uuid>(
        "UPDATE interviews
            SET status = CASE WHEN status = 'in_progress' THEN 'finished' ELSE status END,
                ended_at = CASE WHEN status = 'in_progress' THEN now() ELSE ended_at END,
                updated_at = now()
          WHERE id = $1
          RETURNING id",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    match updated {
        Some(_) => reload_after_update(pool, id).await,
        None => Ok(None),
    }
}

pub async fn cancel(pool: &PgPool, id: &Uuid, user_id: &Uuid) -> ApiResult<Option<InterviewView>> {
    let view = match get_view(pool, id).await? {
        Some(v) => v,
        None => return Ok(None),
    };
    member_guard(&view, user_id, &["invited"], "取消面试")?;
    let updated = sqlx::query_scalar::<_, Uuid>(
        "UPDATE interviews
            SET status = 'cancelled', ended_at = now(), updated_at = now()
          WHERE id = $1
          RETURNING id",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    match updated {
        Some(_) => reload_after_update(pool, id).await,
        None => Ok(None),
    }
}
