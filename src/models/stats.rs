//! 平台统计模型：审核工作量与及时性、用人单位优劣、求职用户分析。
//!
//! **评价口径（重要）**：审核工作量**只衡量及时性**——即「提交 -> 审结」总时长、
//! 「接单 -> 审结」处理时长及其及时率；**不统计通过/驳回的数量与结论分布**，
//! 避免用审核结论去评价审核人员（结论分布另有平台合规意义，不作为个人 KPI）。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

/// 及时线：24 小时。改这一个常量即可让后端与前端展示同步（响应里也会回传）。
pub const REVIEW_SLA_SECONDS: i64 = 24 * 60 * 60;

/// 待审队列概览
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewQueue {
    pub pending_total: i64,
    /// 正被有效锁持有（有人正在审）
    pub locked_active: i64,
    /// 锁已过期但仍留着锁字段（可被任何人重新认领）
    pub locked_expired: i64,
    /// 当前调用者自己持有的有效锁
    pub locked_by_me: i64,
    /// 已超过及时线仍未审结的待审投诉
    pub overdue_total: i64,
    /// 最老的待审投诉已积压多少秒
    pub oldest_pending_seconds: Option<f64>,
}

/// 单个审核账号的工作量与及时性（不含任何结论指标）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewerWorkload {
    pub reviewer_id: Uuid,
    pub reviewer_name: String,
    pub phone: Option<String>,
    pub is_active: bool,
    /// 当前持有的有效锁（正在处理中）
    pub active_locks: i64,
    /// 已审结总数：仅作分母与工作量参考，不代表结论好坏
    pub reviewed_total: i64,
    pub reviewed_last_7d: i64,
    /// 提交 -> 审结 平均耗时（秒）
    pub avg_total_seconds: Option<f64>,
    pub median_total_seconds: Option<f64>,
    pub p90_total_seconds: Option<f64>,
    /// 接单 -> 审结 平均耗时（秒）；只有留下认领时间的记录参与
    pub avg_handle_seconds: Option<f64>,
    /// 参与处理时长统计的样本数（认领时间缺失的历史数据不计入）
    pub handle_sample: i64,
    /// 总耗时在及时线内的条数
    pub on_time_total: i64,
    /// 及时率 = on_time_total / reviewed_total
    pub on_time_rate: Option<f64>,
    pub last_reviewed_at: Option<DateTime<Utc>>,
}

/// 审核统计响应：队列概览 + 审核账号工作量
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewStats {
    pub queue: ReviewQueue,
    /// 平台管理员：全部审核账号；审核账号：只有自己
    pub reviewers: Vec<ReviewerWorkload>,
    /// 及时线（秒），与后端判定一致，供前端展示
    pub sla_seconds: i64,
}

/// 用人单位质量画像
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CompanyQuality {
    pub company_id: Uuid,
    pub name: String,
    pub industry: Option<String>,
    pub location: Option<String>,
    pub complaints_total: i64,
    pub complaints_pending: i64,
    /// 已核实（审核通过）投诉——平台的权威口径
    pub complaints_approved: i64,
    pub complaints_rejected: i64,
    pub complaints_last_30d: i64,
    pub jobs_total: i64,
    pub jobs_active: i64,
    /// 与 `complaint_level_rank(complaints_count)` 一致的等级序：0 优秀 … 4 严重
    pub level_rank: i16,
    /// 每个在招职位摊到的已核实投诉数（规模归一化后的风险）
    pub complaint_rate: Option<f64>,
    /// 综合质量分（0-100，越高越好）：100 - 已核实投诉*10 - 待核实投诉*2，下限 0。
    /// 「已核实投诉」取 `companies.complaints_count`，与 `level_rank` 同源，两者不会互相矛盾。
    pub quality_score: i32,
    pub first_complaint_at: Option<DateTime<Utc>>,
    pub last_complaint_at: Option<DateTime<Utc>>,
}

/// 求职用户总量与参与度概览
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SeekerOverview {
    pub total: i64,
    pub active: i64,
    pub disabled: i64,
    pub new_last_7d: i64,
    pub new_last_30d: i64,
    /// 建过简历的求职者数与其简历总数
    pub with_resume: i64,
    pub resumes_total: i64,
    /// 投过简历的求职者数与其投递总数
    pub with_application: i64,
    pub applications_total: i64,
    pub interviews_total: i64,
    /// 发起过投诉的求职者数与其投诉总数
    pub complainants: i64,
    pub complaints_total: i64,
}

/// 按月计数（注册趋势）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MonthlyCount {
    /// `YYYY-MM`
    pub month: String,
    pub count: i64,
}

/// 按状态计数（投递状态分布）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatusCount {
    pub status: String,
    pub count: i64,
}

/// 分档计数（投递数分档人数）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BucketCount {
    /// 0 / 1-2 / 3-5 / 6+
    pub bucket: String,
    pub count: i64,
}

/// 求职用户分析
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SeekerStats {
    pub overview: SeekerOverview,
    /// 近 12 个月注册趋势
    pub registrations_by_month: Vec<MonthlyCount>,
    pub applications_by_status: Vec<StatusCount>,
    pub application_buckets: Vec<BucketCount>,
    /// 参与度：有简历或有过投递的求职者占比
    pub engagement_rate: Option<f64>,
    /// 人均投诉数（仅统计发起过投诉的求职者）
    pub complaints_per_complainant: Option<f64>,
}
