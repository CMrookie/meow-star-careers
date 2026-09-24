//! users 数据访问：所有 SQL 集中于此。错误经 `From<sqlx::Error>` 统一转 AppError。
//! 注意：sqlx 0.9 要求 SQL 为编译期字面量，不得用 format! 拼接。

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::models::user::{NewUser, UpdateUser, User};

/// 列出全部用户（按创建时间升序）
pub async fn list(pool: &PgPool) -> ApiResult<Vec<User>> {
    let users = sqlx::query_as::<_, User>(
        "SELECT id, email, name, is_active, role, company_id, created_at, updated_at, phone
           FROM users
          ORDER BY created_at",
    )
    .fetch_all(pool)
    .await?;
    Ok(users)
}

/// 按 id 查询单个用户，不存在时返回 None
pub async fn get(pool: &PgPool, id: &Uuid) -> ApiResult<Option<User>> {
    let user = sqlx::query_as::<_, User>(
        "SELECT id, email, name, is_active, role, company_id, created_at, updated_at, phone
           FROM users
          WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(user)
}

/// 创建用户（默认求职者）；email 唯一冲突时由错误转换返回 409
pub async fn create(pool: &PgPool, input: &NewUser) -> ApiResult<User> {
    let user = sqlx::query_as::<_, User>(
        "INSERT INTO users (email, name)
         VALUES ($1, $2)
         RETURNING id, email, name, is_active, role, company_id, created_at, updated_at, phone",
    )
    .bind(&input.email)
    .bind(&input.name)
    .fetch_one(pool)
    .await?;
    Ok(user)
}

/// 更新用户（name / is_active 仅更新提供的字段），返回 None 表示不存在
pub async fn update(pool: &PgPool, id: &Uuid, input: &UpdateUser) -> ApiResult<Option<User>> {
    let user = sqlx::query_as::<_, User>(
        "UPDATE users
            SET name       = COALESCE($2, name),
                is_active  = COALESCE($3, is_active),
                updated_at = now()
          WHERE id = $1
          RETURNING id, email, name, is_active, role, company_id, created_at, updated_at, phone",
    )
    .bind(id)
    .bind(input.name.as_deref())
    .bind(input.is_active)
    .fetch_optional(pool)
    .await?;
    Ok(user)
}

/// 删除用户，返回是否真的删除了某行
pub async fn delete(pool: &PgPool, id: &Uuid) -> ApiResult<bool> {
    let result = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// 角色校验辅助：必须是招聘者且已绑定企业才放行，否则 403
pub async fn require_recruiter(pool: &PgPool, user_id: &Uuid) -> ApiResult<User> {
    let user = get(pool, user_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("user `{user_id}`")))?;
    if user.role != "recruiter" || user.company_id.is_none() {
        return Err(AppError::forbidden("仅招聘者账号（且已绑定企业）可执行该操作"));
    }
    Ok(user)
}

/// 必须是求职者角色才放行
pub async fn require_seeker(pool: &PgPool, user_id: &Uuid) -> ApiResult<User> {
    let user = get(pool, user_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("user `{user_id}`")))?;
    if user.role != "seeker" {
        return Err(AppError::forbidden("仅求职者可执行该操作"));
    }
    Ok(user)
}

/// 必须是平台管理员才放行（账号管理 / 用户管理等平台级操作）
pub async fn require_admin(pool: &PgPool, user_id: &Uuid) -> ApiResult<User> {
    let user = get(pool, user_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("user `{user_id}`")))?;
    if user.role != "admin" {
        return Err(AppError::forbidden("仅平台管理员可执行该操作"));
    }
    Ok(user)
}

/// 审核权限：审核专用账号（reviewer）或平台管理员（admin，作为超级角色可代审）。
/// 各审核账号的令牌独立并存，因此可同时在线并行审核。
pub async fn require_reviewer(pool: &PgPool, user_id: &Uuid) -> ApiResult<User> {
    let user = get(pool, user_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("user `{user_id}`")))?;
    if user.role != "reviewer" && user.role != "admin" {
        return Err(AppError::forbidden("仅审核账号或平台管理员可执行该操作"));
    }
    Ok(user)
}

/// 按角色列出账号（审核账号管理用），按创建时间升序
pub async fn list_by_role(pool: &PgPool, role: &str) -> ApiResult<Vec<User>> {
    let users = sqlx::query_as::<_, User>(
        "SELECT id, email, name, is_active, role, company_id, created_at, updated_at, phone
           FROM users
          WHERE role = $1
          ORDER BY created_at",
    )
    .bind(role)
    .fetch_all(pool)
    .await?;
    Ok(users)
}

/// 某企业全部招聘者（用于投递状态/面试变更实时推送）
pub async fn recruiters_of_company(pool: &PgPool, company_id: &Uuid) -> ApiResult<Vec<Uuid>> {
    let ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM users WHERE role = 'recruiter' AND company_id = $1",
    )
    .bind(company_id)
    .fetch_all(pool)
    .await?;
    Ok(ids)
}
