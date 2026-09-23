//! 企业（公司）模型

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

/// companies 表的一行
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Company {
    pub id: Uuid,
    pub name: String,
    pub industry: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    /// 详细地址（街道级，可选；用于地图定位）
    pub address: Option<String>,
    pub website: Option<String>,
    pub logo_url: Option<String>,
    pub is_active: bool,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// 被投诉次数：只统计**管理员审核通过**的投诉（0 = 优秀档，见 complaint_level_rank）
    pub complaints_count: i32,
}

/// 新建企业（招聘者注册时必填）
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NewCompany {
    #[schema(example = "字节跳动")]
    pub name: String,
    #[schema(example = "互联网")]
    pub industry: Option<String>,
    pub description: Option<String>,
    #[schema(example = "北京")]
    pub location: Option<String>,
    pub address: Option<String>,
    pub website: Option<String>,
    pub logo_url: Option<String>,
}
