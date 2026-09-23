//! 企业数据访问

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;
use crate::models::company::Company;

/// 按 id 查询企业
pub async fn get(pool: &PgPool, id: &Uuid) -> ApiResult<Option<Company>> {
    let company = sqlx::query_as::<_, Company>(
        "SELECT id, name, industry, description, location, address, website, logo_url,
                is_active, created_by, created_at, updated_at, complaints_count
           FROM companies
          WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(company)
}

