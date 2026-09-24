//! 投诉模型（视图/载荷）

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

/// 投诉视图（含企业与投诉人信息）
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
    /// 审核人姓名（多审核账号并行时用于留痕；未审核为 null）
    pub reviewed_by_name: Option<String>,
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
