//! 投诉模型（视图/载荷）

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

/// 投诉视图（企业、投诉人、审核人与认领锁）
///
/// 对应数据库只读视图 `complaint_views`（见 `0016_complaint_review_lock.sql`），
/// `lock_active` 由数据库按 `now()` 计算，因此不会出现前端拿到过期锁状态的情况。
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ComplaintView {
    pub id: Uuid,
    pub company_id: Uuid,
    pub company_name: String,
    pub complainant_id: Uuid,
    pub complainant_name: String,
    pub evidence: String,
    pub status: String,
    pub review_note: Option<String>,
    pub reviewed_by: Option<Uuid>,
    pub reviewed_by_name: Option<String>,
    /// 当前认领该投诉的审核账号（None = 空闲）
    pub locked_by: Option<Uuid>,
    pub locked_by_name: Option<String>,
    /// 本次获得锁的时间（续约不改写）
    pub locked_at: Option<DateTime<Utc>>,
    /// 锁到期时间；到期即视为空闲，可被他人重新认领
    pub lock_expires_at: Option<DateTime<Utc>>,
    /// 锁当前是否仍有效（数据库计算）；false 表示空闲或已过期
    pub lock_active: bool,
    /// 首次认领（接单）时间：审核处理时长的起点，审结后仍保留
    pub review_started_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

/// 发起投诉载荷
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateComplaint {
    /// 有效证据说明（要求非空且 >=20 字符）
    pub evidence: String,
}

/// 审核载荷
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewComplaint {
    pub approved: bool,
    /// 审核备注（驳回建议填写原因）
    pub note: Option<String>,
}

/// 释放认领锁的查询参数
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseLockQuery {
    /// 强制释放：仅平台管理员可用，用于解锁他人忘记释放的投诉
    #[serde(default)]
    pub force: bool,
}

/// 投诉定级规则（**服务端单一来源**）
///
/// 数字全部来自数据库里的 `complaint_rule_*` 函数 —— 排序、`JobView` 的
/// `complaintLevel/complaintBasis/complaintRatePercent` 与本接口三者同源，
/// 客户端只需渲染，不再自己维护阈值表。
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ComplaintRules {
    /// 规则版本（如 v2）；客户端可据此决定是否重新提示用户
    pub version: String,
    /// 等级序列（由优到劣）：excellent / minor / alert / warning / severe
    pub levels: Vec<String>,
    /// 按率定级所需的最小企业规模（人），低于它退回次数口径
    pub min_staff_size_for_rate: i32,
    /// 每百人投诉率上界（%）：0.5 / 1.5 / 3.0
    pub rate_thresholds: Vec<f64>,
    /// 投诉次数下界：[0, 1, 3, 6, 10]（与客户端原 complaintLevelThresholds 同形）
    pub count_thresholds: Vec<i32>,
}
