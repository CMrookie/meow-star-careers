//! 线上面试模型：行/视图结构、创建载荷。JSON camelCase。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

/// 面试视图（含职位/企业与双方姓名，列表与详情共用）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InterviewView {
    pub id: Uuid,
    pub application_id: Uuid,
    pub job_id: Uuid,
    pub job_title: String,
    pub company_id: Uuid,
    pub company_name: String,
    pub interviewer_id: Uuid,
    pub interviewer_name: String,
    pub interviewee_id: Uuid,
    pub interviewee_name: String,
    pub status: String,
    /// NULL = 即时面试；非空 = 预约时间
    pub scheduled_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 发起面试请求体
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NewInterview {
    /// 对哪个投递发起（招聘者名下企业的投递）
    pub application_id: Uuid,
    /// 预约时间（可选；NULL = 即时面试）
    pub scheduled_at: Option<DateTime<Utc>>,
}
