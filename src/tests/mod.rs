//! 集成测试（`cargo test`）：真实 PostgreSQL 测试库 + actix 测试服务。
//!
//! - 鉴权：注册/登录/登出、令牌校验、角色与越权防护（auth.rs）
//! - 举报审核：发起投诉的前置条件、审核账号/管理员审核及其对排名的影响（complaints.rs）
//! - 审核专用账号：admin 创建管理、多账号同时在线并行审核（reviewers.rs）
//! - 并行锁定：认领/续约/过期/释放/持锁审结的并发保护（review_locks.rs）
//! - 平台统计：审核及时性口径与可见范围、用人单位优劣、求职用户分析（stats.rs）
//! - 职位列表排序：按用人单位投诉等级「从优到劣」、跨页全局有序（jobs_order.rs）

mod auth;
mod common;
mod complaints;
mod jobs_order;
mod review_locks;
mod reviewers;
mod stats;
