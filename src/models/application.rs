//! 投递（申请）模型

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

/// 投递状态全集与各自允许的流转
pub const APPLICATION_STATUSES: [&str; 6] =
    ["pending", "viewed", "interviewing", "offered", "rejected", "withdrawn"];
/// 招聘者可推进到的状态
pub const RECRUITER_TRANSITIONS: [&str; 4] = ["viewed", "interviewing", "offered", "rejected"];
/// 求职者只能撤回自己的投递
pub const SEEKER_WITHDRAW: &str = "withdrawn";

/// applications 联表查询行（含职位/企业/求职者信息）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationView {
    pub id: Uuid,
    pub job_id: Uuid,
    pub job_title: String,
    pub company_id: Uuid,
    pub company_name: String,
    pub seeker_id: Uuid,
    pub seeker_name: String,
    /// 求职者联系邮箱（可空；手机号注册用户无邮箱）
    pub seeker_email: Option<String>,
    /// 求职者手机号（可空；历史邮箱账号无手机号）
    pub seeker_phone: Option<String>,
    pub resume_id: Option<Uuid>,
    pub cover_letter: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 投递职位请求体
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApplyRequest {
    /// 使用的简历 id（可不传）
    pub resume_id: Option<Uuid>,
    pub cover_letter: Option<String>,
}

/// 更新投递状态请求体
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateApplicationStatus {
    /// viewed / interviewing / offered / rejected（招聘者）；
    /// withdrawn（求职者撤回）
    #[schema(example = "interviewing")]
    pub status: String,
}

/// 投递查询参数
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationQuery {
    /// 只看某职位
    pub job_id: Option<Uuid>,
    /// 按状态过滤
    pub status: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

/// 投递分页响应
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationPage {
    pub items: Vec<ApplicationView>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}
