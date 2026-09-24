//! 用户模型：行结构、请求/响应体。JSON 采用 camelCase，数据库列为 snake_case。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

/// users 表中的一行（也是 GET 响应的实体）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: Uuid,
    /// 联系邮箱（可空；手机号注册的新账号为空）
    pub email: Option<String>,
    pub name: String,
    pub is_active: bool,
    /// seeker=求职者 / recruiter=招聘者 / admin=平台管理 / reviewer=审核专用账号（admin 创建）
    pub role: String,
    /// 招聘者所属企业（求职者为 null）
    pub company_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// 注册手机号（登录账号，唯一；历史邮箱账号为空）
    pub phone: Option<String>,
}

/// 对他人可见的最小用户信息（会话对端等场景）
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserPublic {
    pub id: Uuid,
    pub name: String,
}

/// 创建用户请求体（管理端用；auth/register 走手机号）
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NewUser {
    #[schema(example = "alice@example.com")]
    pub email: String,
    #[schema(example = "Alice")]
    pub name: String,
}

/// 更新用户请求体（字段均为可选，至少提供一个）
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUser {
    #[schema(example = "Alice Smith")]
    pub name: Option<String>,
    #[schema(example = false)]
    pub is_active: Option<bool>,
}

/// 新建审核专用账号（仅平台管理员；手机号 + 初始密码，登录后可改密）
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NewReviewer {
    #[schema(example = "13700000001", min_length = 11, max_length = 11)]
    pub phone: String,
    #[schema(example = "审核员小王")]
    pub name: String,
    #[schema(example = "secret123", min_length = 8)]
    pub password: String,
}

/// 重置审核账号密码（仅平台管理员；重置后该账号已登录的全部会话立即失效）
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResetPassword {
    #[schema(example = "secret123", min_length = 8)]
    pub password: String,
}

/// 启用 / 禁用请求体（审核账号管理）
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetActiveRequest {
    #[schema(example = false)]
    pub is_active: bool,
}
