//! 认证数据访问：用户凭证查询/注册（含招聘者+企业事务）、令牌签发/校验/撤销。
//! 库中只保存 token 的 sha256 摘要，不保存明文。

use chrono::Utc;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::models::auth::Role;
use crate::models::company::{Company, NewCompany};
use crate::models::user::User;
use crate::security;

/// 带密码哈希的用户记录（仅登录校验用，不参与序列化）
pub struct StoredUser {
    pub user: User,
    pub password_hash: Option<String>,
}

#[derive(FromRow)]
struct UserWithHashRow {
    id: Uuid,
    email: Option<String>,
    name: String,
    is_active: bool,
    role: String,
    company_id: Option<Uuid>,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
    phone: Option<String>,
    password_hash: Option<String>,
}

/// 按手机号查找（用于登录）
pub async fn find_by_phone(pool: &PgPool, phone: &str) -> ApiResult<Option<StoredUser>> {
    let row = sqlx::query_as::<_, UserWithHashRow>(
        "SELECT id, email, name, is_active, role, company_id, created_at, updated_at, phone, password_hash
           FROM users
          WHERE phone = $1",
    )
    .bind(phone)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|row| StoredUser {
        user: User {
            id: row.id,
            email: row.email,
            name: row.name,
            is_active: row.is_active,
            role: row.role,
            company_id: row.company_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
            phone: row.phone,
        },
        password_hash: row.password_hash,
    }))
}

/// 注册求职者（phone 唯一冲突会经错误转换返回 409）
pub async fn register_seeker(
    pool: &PgPool,
    phone: &str,
    name: &str,
    password_hash: &str,
) -> ApiResult<User> {
    let user = sqlx::query_as::<_, User>(
        "INSERT INTO users (phone, name, password_hash, role)
         VALUES ($1, $2, $3, 'seeker')
         RETURNING id, email, name, is_active, role, company_id, created_at, updated_at, phone",
    )
    .bind(phone)
    .bind(name)
    .bind(password_hash)
    .fetch_one(pool)
    .await?;
    Ok(user)
}

/// 注册招聘者：事务内创建用户 + 其所属企业并回填 company_id
pub async fn register_recruiter(
    pool: &PgPool,
    phone: &str,
    name: &str,
    password_hash: &str,
    company: &NewCompany,
) -> ApiResult<(User, Company)> {
    let mut tx = pool.begin().await?;

    let user = insert_user_with_role(&mut tx, phone, name, password_hash, Role::Recruiter).await?;
    let company_row = insert_company(&mut tx, &user.id, company).await?;

    let user = sqlx::query_as::<_, User>(
        "UPDATE users
            SET company_id = $2
          WHERE id = $1
          RETURNING id, email, name, is_active, role, company_id, created_at, updated_at, phone",
    )
    .bind(user.id)
    .bind(company_row.id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok((user, company_row))
}

async fn insert_user_with_role(
    tx: &mut Transaction<'_, Postgres>,
    phone: &str,
    name: &str,
    password_hash: &str,
    role: Role,
) -> ApiResult<User> {
    let user = sqlx::query_as::<_, User>(
        "INSERT INTO users (phone, name, password_hash, role)
         VALUES ($1, $2, $3, $4)
         RETURNING id, email, name, is_active, role, company_id, created_at, updated_at, phone",
    )
    .bind(phone)
    .bind(name)
    .bind(password_hash)
    .bind(role.as_str())
    .fetch_one(&mut **tx)
    .await?;
    Ok(user)
}

async fn insert_company(
    tx: &mut Transaction<'_, Postgres>,
    created_by: &Uuid,
    input: &NewCompany,
) -> ApiResult<Company> {
    let company = sqlx::query_as::<_, Company>(
        "INSERT INTO companies (name, industry, description, location, address, website, logo_url,
                                created_by, staff_size)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         RETURNING id, name, industry, description, location, address, website, logo_url,
                   is_active, created_by, created_at, updated_at, complaints_count, staff_size",
    )
    .bind(input.name.trim())
    .bind(input.industry.as_deref())
    .bind(input.description.as_deref())
    .bind(input.location.as_deref())
    .bind(input.address.as_deref())
    .bind(input.website.as_deref())
    .bind(input.logo_url.as_deref())
    .bind(created_by)
    .bind(input.staff_size)
    .fetch_one(&mut **tx)
    .await?;
    Ok(company)
}

/// 为用户签发新令牌，返回明文令牌（仅此一次，调用方负责返回给客户端）
pub async fn issue_token(pool: &PgPool, user_id: Uuid) -> ApiResult<String> {
    let (raw, digest) = security::new_session_token();
    let expires_at = Utc::now() + chrono::Duration::days(30);

    sqlx::query(
        "INSERT INTO auth_tokens (token_hash, user_id, expires_at)
         VALUES ($1, $2, $3)",
    )
    .bind(&digest)
    .bind(user_id)
    .bind(expires_at)
    .execute(pool)
    .await?;

    Ok(raw)
}

/// 按明文令牌查找用户（未撤销且未过期）
pub async fn find_user_by_token(pool: &PgPool, token: &str) -> ApiResult<Option<User>> {
    let digest = security::sha256_hex(token.as_bytes());

    let user = sqlx::query_as::<_, User>(
        "SELECT u.id, u.email, u.name, u.is_active, u.role, u.company_id, u.created_at, u.updated_at, u.phone
           FROM auth_tokens t
           JOIN users u ON u.id = t.user_id
          WHERE t.token_hash = $1
            AND t.revoked_at IS NULL
            AND t.expires_at > now()
            AND u.is_active = true",
    )
    .bind(&digest)
    .fetch_optional(pool)
    .await?;
    Ok(user)
}

/// 撤销令牌（登出）
pub async fn revoke_token(pool: &PgPool, token: &str) -> ApiResult<bool> {
    let digest = security::sha256_hex(token.as_bytes());

    let result = sqlx::query(
        "UPDATE auth_tokens
            SET revoked_at = now()
          WHERE token_hash = $1 AND revoked_at IS NULL",
    )
    .bind(&digest)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// 管理员引导结果（服务启动时打印日志用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminBootstrap {
    /// 本次启动创建了管理员账号
    Created,
    /// 已存在同手机号的管理员账号（保持原密码不变）
    AlreadyExists,
}

/// 引导平台管理员账号（幂等，服务启动时调用；投诉审核需要 admin 角色）。
///
/// 规则：
/// - 手机号不存在 → 创建 `role=admin` 的账号（密码立即 argon2id 哈希入库）；
/// - 已存在且 `role=admin` → 直接返回，**不覆盖密码**（避免每次启动静默改密）；
/// - 已存在但为其它角色 → 返回 409，**绝不把既有账号提权**（防越权）。
pub async fn ensure_admin(
    pool: &PgPool,
    phone: &str,
    name: &str,
    password: &str,
) -> ApiResult<AdminBootstrap> {
    let phone = phone.trim();
    if !security::is_valid_cn_phone(phone) {
        return Err(AppError::bad_request("ADMIN_PHONE 必须为 11 位手机号（1 开头）"));
    }
    if !(8..=128).contains(&password.len()) {
        return Err(AppError::bad_request("ADMIN_PASSWORD 长度须在 8-128 之间"));
    }
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(AppError::bad_request("ADMIN_NAME 必填且不超过 64 字符"));
    }

    let existing_role = sqlx::query_scalar::<_, String>("SELECT role FROM users WHERE phone = $1")
        .bind(phone)
        .fetch_optional(pool)
        .await?;

    if let Some(role) = existing_role {
        if role == "admin" {
            return Ok(AdminBootstrap::AlreadyExists);
        }
        return Err(AppError::conflict(format!(
            "ADMIN_PHONE 已被 {role} 账号占用，拒绝自动提权"
        )));
    }

    let password_hash =
        security::hash_password(password).map_err(|err| AppError::internal(err.to_string()))?;
    sqlx::query(
        "INSERT INTO users (phone, name, password_hash, role)
         VALUES ($1, $2, $3, 'admin')",
    )
    .bind(phone)
    .bind(name)
    .bind(&password_hash)
    .execute(pool)
    .await?;

    Ok(AdminBootstrap::Created)
}

/// 创建审核专用账号（`role=reviewer`，仅平台管理员调用）。
/// 手机号唯一冲突由错误转换返回 409；密码由调用方哈希后传入。
pub async fn create_reviewer(
    pool: &PgPool,
    phone: &str,
    name: &str,
    password_hash: &str,
) -> ApiResult<User> {
    let user = sqlx::query_as::<_, User>(
        "INSERT INTO users (phone, name, password_hash, role)
         VALUES ($1, $2, $3, 'reviewer')
         RETURNING id, email, name, is_active, role, company_id, created_at, updated_at, phone",
    )
    .bind(phone.trim())
    .bind(name.trim())
    .bind(password_hash)
    .fetch_one(pool)
    .await?;
    Ok(user)
}

/// 重置密码（审核账号管理用），返回是否命中某行
pub async fn update_password(pool: &PgPool, user_id: &Uuid, password_hash: &str) -> ApiResult<bool> {
    let result = sqlx::query(
        "UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1",
    )
    .bind(user_id)
    .bind(password_hash)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// 撤销某账号的**全部**有效令牌（重置密码 / 禁用账号时强制其重新登录）。
/// 返回被撤销的会话数；多个审核账号并行在线时互不影响。
pub async fn revoke_all_tokens(pool: &PgPool, user_id: &Uuid) -> ApiResult<u64> {
    let result = sqlx::query(
        "UPDATE auth_tokens
            SET revoked_at = now()
          WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}
