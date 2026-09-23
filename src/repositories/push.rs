//! 设备推送令牌注册（预留：FCM/APNs 接入后读取投递）

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;

/// 注册/更新推送令牌（同平台同令牌幂等 upsert）
pub async fn register(
    pool: &PgPool,
    user_id: &Uuid,
    platform: &str,
    token: &str,
) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO device_push_tokens (user_id, platform, token)
         VALUES ($1, $2, $3)
         ON CONFLICT (user_id, platform, token)
         DO UPDATE SET updated_at = now()",
    )
    .bind(user_id)
    .bind(platform)
    .bind(token)
    .execute(pool)
    .await?;
    Ok(())
}

/// 注销某平台的推送令牌
pub async fn remove(pool: &PgPool, user_id: &Uuid, platform: &str, token: &str) -> ApiResult<()> {
    sqlx::query(
        "DELETE FROM device_push_tokens WHERE user_id = $1 AND platform = $2 AND token = $3",
    )
    .bind(user_id)
    .bind(platform)
    .bind(token)
    .execute(pool)
    .await?;
    Ok(())
}
