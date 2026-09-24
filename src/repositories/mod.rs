//! 数据访问层（仓储）：封装所有 SQL，向 handlers 暴露干净的异步接口。

pub mod application;
pub mod auth;
pub mod chat;
pub mod complaint;
pub mod company;
pub mod interview;
pub mod job;
pub mod push;
pub mod resume;
pub mod stats;
pub mod user;
