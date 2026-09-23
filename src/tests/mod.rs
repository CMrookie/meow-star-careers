//! 集成测试（`cargo test`）：真实 PostgreSQL 测试库 + actix 测试服务。
//!
//! - 鉴权：注册/登录/登出、令牌校验、角色与越权防护（auth.rs）
//! - 举报审核：发起投诉的前置条件、管理员审核及其对排名的影响（complaints.rs）
//! - 职位列表排序：按用人单位投诉等级「从优到劣」、跨页全局有序（jobs_order.rs）

mod auth;
mod common;
mod complaints;
mod jobs_order;
