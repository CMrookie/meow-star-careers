//! 简历模型

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

/// resumes 表的一行
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Resume {
    pub id: Uuid,
    pub user_id: Uuid,
    pub full_name: String,
    #[schema(example = "期望职位，如后端工程师")]
    pub title: String,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub years: Option<i32>,
    pub education: Option<String>,
    pub skills: Option<String>,
    pub summary: Option<String>,
    pub is_public: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 新建/更新简历（更新时仅覆盖提供的字段）
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResumeWrite {
    pub full_name: Option<String>,
    pub title: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub years: Option<i32>,
    pub education: Option<String>,
    pub skills: Option<String>,
    pub summary: Option<String>,
    pub is_public: Option<bool>,
}

/// 简历查询参数
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeQuery {
    /// 关键词（匹配姓名/期望职位/技能/自我评价），仅招聘者可见
    pub keyword: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

/// 简历分页响应
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResumePage {
    pub items: Vec<Resume>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}
