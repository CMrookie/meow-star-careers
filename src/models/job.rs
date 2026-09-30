//! 职位模型

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

/// 职位可用的工作性质
pub const JOB_TYPES: [&str; 4] = ["full_time", "part_time", "contract", "intern"];

/// jobs 表 + 企业名的查询行（列表/详情共用）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub id: Uuid,
    pub company_id: Uuid,
    pub company_name: String,
    pub title: String,
    pub description: String,
    pub requirements: Option<String>,
    pub location: Option<String>,
    pub salary_min: Option<i32>,
    pub salary_max: Option<i32>,
    /// full_time / part_time / contract / intern
    pub job_type: String,
    pub experience: Option<String>,
    pub education: Option<String>,
    pub is_active: bool,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// 企业被投诉次数（决定职位展示颜色）
    pub complaints_count: i32,
    /// 用人单位规模（员工人数，企业申报；NULL = 未申报）。
    /// 投诉定级 v2 的分母：>=50 人时按每百人投诉率定级，否则退回次数口径
    /// （与求职 App 的 `companyStaffSize` 字段一一对应）。
    pub company_staff_size: Option<i32>,
}

/// 创建职位
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NewJob {
    #[schema(example = "后端工程师")]
    pub title: String,
    #[schema(example = "负责 xxx 系统的设计与开发")]
    pub description: String,
    pub requirements: Option<String>,
    #[schema(example = "北京")]
    pub location: Option<String>,
    pub salary_min: Option<i32>,
    pub salary_max: Option<i32>,
    #[schema(example = "full_time")]
    pub job_type: Option<String>,
    #[schema(example = "3-5年")]
    pub experience: Option<String>,
    #[schema(example = "本科")]
    pub education: Option<String>,
}

/// 更新职位（仅更新提供的字段）
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateJob {
    pub title: Option<String>,
    pub description: Option<String>,
    pub requirements: Option<String>,
    pub location: Option<String>,
    pub salary_min: Option<i32>,
    pub salary_max: Option<i32>,
    pub job_type: Option<String>,
    pub experience: Option<String>,
    pub education: Option<String>,
}

/// 职位搜索/列表查询参数
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobQuery {
    /// 关键词（匹配职位名/描述/要求）
    pub keyword: Option<String>,
    /// 城市
    pub location: Option<String>,
    /// full_time / part_time / contract / intern
    pub job_type: Option<String>,
    /// 期望最低月薪（取职位上限 >= 该值）
    pub salary_min: Option<i32>,
    /// 期望最高月薪（取职位下限 <= 该值）
    pub salary_max: Option<i32>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

/// 职位列表分页响应
///
/// `items` 一律按用人单位投诉等级**从优到劣**排序（等级函数见
/// `migrations/0013_complaint_level_order.sql`；排序在服务端完成、翻页全局有序），
/// 客户端按返回顺序渲染即可，无需再自行排序。
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobPage {
    pub items: Vec<JobView>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}
