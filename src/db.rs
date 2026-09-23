//! 数据库模块：负责建立连接池与执行迁移，与业务代码解耦。

use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

use crate::config::Config;

/// 连接 PostgreSQL 并返回连接池
pub async fn connect(config: &Config) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(config.db_max_connections)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&config.database_url)
        .await
        .with_context(|| {
            format!("连接 PostgreSQL 失败（{}）", sanitize_url(&config.database_url))
        })?;

    tracing::info!("数据库连接池就绪");
    Ok(pool)
}

/// 执行 ./migrations 下的迁移（编译期内嵌，服务启动时自动应用）
pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("执行数据库迁移失败")?;

    tracing::info!("数据库迁移完成");
    Ok(())
}

/// 打日志时不泄露连接串中的密码（示例：postgres://app:***@127.0.0.1:5432/appdb）
fn sanitize_url(url: &str) -> String {
    let Some(at) = url.rfind('@') else {
        return url.to_string();
    };
    let scheme_end = url.find("://").map(|i| i + 3).unwrap_or(0);
    if scheme_end < at {
        let cred = &url[scheme_end..at];
        if let Some(colon) = cred.find(':') {
            return format!(
                "{}{}:***@{}",
                &url[..scheme_end],
                &cred[..=colon],
                &url[at + 1..]
            );
        }
    }
    url.to_string()
}
