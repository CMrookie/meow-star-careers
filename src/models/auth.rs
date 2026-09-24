//! 认证相关的请求/响应体

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::models::company::NewCompany;
use crate::models::user::User;

/// 用户角色
///
/// `Admin` / `Reviewer` 仅用于表达数据库/文档里的角色取值：注册接口**拒绝**
/// 自助注册这两种角色 —— admin 由服务端按 `ADMIN_PHONE` / `ADMIN_PASSWORD` 引导创建
/// （`repositories::auth::ensure_admin`），reviewer（审核专用账号）由平台管理员创建
/// （`POST /api/v1/reviewers`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// 求职者（默认）
    #[default]
    Seeker,
    /// 招聘者
    Recruiter,
    /// 平台管理（账号与用户管理；不可自助注册）
    Admin,
    /// 审核专用账号（投诉审核；不可自助注册，由 admin 创建）
    Reviewer,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Seeker => "seeker",
            Self::Recruiter => "recruiter",
            Self::Admin => "admin",
            Self::Reviewer => "reviewer",
        }
    }
}

/// 注册请求（手机号 + 密码）
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RegisterRequest {
    #[schema(example = "13800138000", min_length = 11, max_length = 11)]
    pub phone: String,
    #[schema(example = "Bob")]
    pub name: String,
    #[schema(example = "secret123", min_length = 8)]
    pub password: String,
    /// seeker（默认）或 recruiter（admin 不可自助注册）
    #[serde(default)]
    pub role: Role,
    /// 角色为 recruiter 时必填：其所属企业
    pub company: Option<NewCompany>,
}

/// 登录请求（手机号 + 密码）
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    #[schema(example = "13800138000", min_length = 11, max_length = 11)]
    pub phone: String,
    #[schema(example = "secret123")]
    pub password: String,
}

/// 认证成功响应：签发的一次性令牌 + 用户信息
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthResponse {
    pub token: String,
    pub user: User,
}
