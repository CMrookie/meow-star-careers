//! 应用配置：全部来自环境变量（支持 .env 文件），集中管理、易于测试。

use std::net::SocketAddr;

use anyhow::Context;

#[derive(Debug, Clone)]
pub struct Config {
    /// 监听地址 host
    pub server_host: String,
    /// 监听端口
    pub server_port: u16,
    /// PostgreSQL 连接串
    pub database_url: String,
    /// 连接池最大连接数
    pub db_max_connections: u32,
    /// CORS 允许的来源（逗号分隔；空 = 默认拒绝跨域，仅同源可用）
    pub cors_origins: Vec<String>,
    /// 平台管理员引导手机号（与 admin_password 同时提供才生效；投诉审核需要 admin 账号）
    pub admin_phone: Option<String>,
    /// 平台管理员引导密码（8-128 位）
    pub admin_password: Option<String>,
    /// 平台管理员显示名（默认「平台管理员」）
    pub admin_name: String,
}

impl Config {
    /// 从环境变量读取配置；未提供的项使用开发默认值。
    pub fn from_env() -> anyhow::Result<Self> {
        // 存在 .env 时自动加载（失败静默，例如生产环境仅用真实环境变量）
        dotenvy::dotenv().ok();

        let database_url = std::env::var("DATABASE_URL")
            .context("缺少环境变量 DATABASE_URL（postgres://user:pass@host:port/db）")?;

        let server_host = std::env::var("SERVER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let server_port = env_or("SERVER_PORT", "8080")?;
        let db_max_connections = env_or("DB_MAX_CONNECTIONS", "10")?;
        let cors_origins = std::env::var("CORS_ALLOWED_ORIGINS")
            .map(|raw| {
                raw.split(',')
                    .map(|item| item.trim().to_string())
                    .filter(|item| !item.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        // 平台管理员引导：ADMIN_PHONE / ADMIN_PASSWORD 必须成对出现，否则视为误配置直接失败
        let admin_phone = env_opt("ADMIN_PHONE");
        let admin_password = env_opt("ADMIN_PASSWORD");
        if admin_phone.is_some() != admin_password.is_some() {
            anyhow::bail!(
                "ADMIN_PHONE 与 ADMIN_PASSWORD 必须同时提供（用于引导平台管理员账号，投诉审核依赖 admin）"
            );
        }
        let admin_name = env_opt("ADMIN_NAME").unwrap_or_else(|| "平台管理员".to_string());

        Ok(Self {
            server_host,
            server_port,
            database_url,
            db_max_connections,
            cors_origins,
            admin_phone,
            admin_password,
            admin_name,
        })
    }

    /// 解析出 SocketAddr 供 HttpServer::bind 使用
    pub fn bind_addr(&self) -> anyhow::Result<SocketAddr> {
        format!("{}:{}", self.server_host, self.server_port)
            .parse()
            .with_context(|| {
                format!(
                    "非法的监听地址 {}:{}",
                    self.server_host, self.server_port
                )
            })
    }
}

fn env_or<T>(key: &str, default: &str) -> anyhow::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match std::env::var(key) {
        Ok(raw) => raw
            .parse()
            .with_context(|| format!("环境变量 {key} 的值无法解析")),
        Err(_) => default
            .parse()
            .with_context(|| format!("环境变量 {key} 的默认值无法解析")),
    }
}

/// 读取可选环境变量：未设置或空白一律视为「未提供」
fn env_opt(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}
