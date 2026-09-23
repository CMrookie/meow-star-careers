//! 私聊数据访问：会话 get-or-create / 列表 / 成员校验，消息插入 / 历史 / 已读。
//! 会话以 (user_lo, user_hi) 唯一约束保证两人只有一个会话。

use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::models::chat::{ConversationRow, Message};

/// 规范化两个用户 id 为 (lo, hi)
fn normalize(a: &Uuid, b: &Uuid) -> (Uuid, Uuid) {
    if a < b {
        (*a, *b)
    } else {
        (*b, *a)
    }
}

/// 判断用户是否存在且启用
pub async fn user_exists_active(pool: &PgPool, user_id: &Uuid) -> ApiResult<bool> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND is_active = true)",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(exists)
}

/// 获取或创建与指定用户的会话；第二个返回值表示本次是否新建
pub async fn get_or_create(
    pool: &PgPool,
    me: &Uuid,
    other: &Uuid,
) -> ApiResult<(ConversationRow, bool)> {
    let (lo, hi) = normalize(me, other);

    let insert = sqlx::query(
        "INSERT INTO conversations (user_lo, user_hi)
         VALUES ($1, $2)
         ON CONFLICT (user_lo, user_hi) DO NOTHING",
    )
    .bind(lo)
    .bind(hi)
    .execute(pool)
    .await?;
    let created = insert.rows_affected() > 0;

    let row = sqlx::query_as::<_, ConversationRow>(
        "SELECT id, user_lo, user_hi, created_at, updated_at
           FROM conversations
          WHERE user_lo = $1 AND user_hi = $2",
    )
    .bind(lo)
    .bind(hi)
    .fetch_one(pool)
    .await?;

    Ok((row, created))
}

/// 当前用户参与的会话（含每个会话的末条消息与未读数）
pub async fn list_for_user(
    pool: &PgPool,
    user: &Uuid,
) -> ApiResult<Vec<(ConversationRow, Option<Message>, i64)>> {
    let conversations = sqlx::query_as::<_, ConversationRow>(
        "SELECT id, user_lo, user_hi, created_at, updated_at
           FROM conversations
          WHERE user_lo = $1 OR user_hi = $1
          ORDER BY updated_at DESC",
    )
    .bind(user)
    .fetch_all(pool)
    .await?;

    let mut result = Vec::with_capacity(conversations.len());
    for conv in conversations {
        let last = sqlx::query_as::<_, Message>(
            "SELECT id, conversation_id, sender_id, recipient_id, content, created_at, read_at
               FROM messages
              WHERE conversation_id = $1
              ORDER BY id DESC
              LIMIT 1",
        )
        .bind(conv.id)
        .fetch_optional(pool)
        .await?;

        let unread = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM messages
              WHERE conversation_id = $1 AND recipient_id = $2 AND read_at IS NULL",
        )
        .bind(conv.id)
        .bind(user)
        .fetch_one(pool)
        .await?;

        result.push((conv, last, unread));
    }

    Ok(result)
}

/// 校验会话归属：必须是参与者才能访问（返回 None 视为越权）
pub async fn conversation_for_member(
    pool: &PgPool,
    conversation_id: &Uuid,
    user: &Uuid,
) -> ApiResult<Option<ConversationRow>> {
    let row = sqlx::query_as::<_, ConversationRow>(
        "SELECT id, user_lo, user_hi, created_at, updated_at
           FROM conversations
          WHERE id = $1 AND (user_lo = $2 OR user_hi = $2)",
    )
    .bind(conversation_id)
    .bind(user)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 会话中的另一位成员 id
pub fn other_party(conv: &ConversationRow, me: &Uuid) -> Uuid {
    if conv.user_lo == *me {
        conv.user_hi
    } else {
        conv.user_lo
    }
}

/// 在会话内插入消息并刷新会话时间（事务内完成），返回落库后的消息
pub async fn insert_message(
    pool: &PgPool,
    conversation: &ConversationRow,
    sender: &Uuid,
    content: &str,
) -> ApiResult<Message> {
    let recipient = other_party(conversation, sender);

    let mut tx = pool.begin().await?;
    let message =
        insert_message_tx(&mut tx, &conversation.id, sender, &recipient, content).await?;
    sqlx::query("UPDATE conversations SET updated_at = now() WHERE id = $1")
        .bind(conversation.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(message)
}

async fn insert_message_tx(
    tx: &mut Transaction<'_, Postgres>,
    conversation_id: &Uuid,
    sender: &Uuid,
    recipient: &Uuid,
    content: &str,
) -> ApiResult<Message> {
    let message = sqlx::query_as::<_, Message>(
        "INSERT INTO messages (conversation_id, sender_id, recipient_id, content)
         VALUES ($1, $2, $3, $4)
         RETURNING id, conversation_id, sender_id, recipient_id, content, created_at, read_at",
    )
    .bind(conversation_id)
    .bind(sender)
    .bind(recipient)
    .bind(content)
    .fetch_one(&mut **tx)
    .await?;
    Ok(message)
}

/// 拉取某会话的历史消息（升序）；before 为消息 id 游标（不含），limit 默认 50
pub async fn list_messages(
    pool: &PgPool,
    conversation_id: &Uuid,
    before: Option<i64>,
    limit: i64,
) -> ApiResult<Vec<Message>> {
    let limit = limit.clamp(1, 200);

    let mut messages = match before {
        Some(cursor) => {
            sqlx::query_as::<_, Message>(
                "SELECT id, conversation_id, sender_id, recipient_id, content, created_at, read_at
                   FROM messages
                  WHERE conversation_id = $1 AND id < $2
                  ORDER BY id DESC
                  LIMIT $3",
            )
            .bind(conversation_id)
            .bind(cursor)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<_, Message>(
                "SELECT id, conversation_id, sender_id, recipient_id, content, created_at, read_at
                   FROM messages
                  WHERE conversation_id = $1
                  ORDER BY id DESC
                  LIMIT $2",
            )
            .bind(conversation_id)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
    };
    messages.reverse();
    Ok(messages)
}

/// 把对方发给我的消息全部标记为已读，返回标记条数
pub async fn mark_conversation_read(
    pool: &PgPool,
    conversation_id: &Uuid,
    reader: &Uuid,
) -> ApiResult<u64> {
    let result = sqlx::query(
        "UPDATE messages
            SET read_at = now()
          WHERE conversation_id = $1 AND recipient_id = $2 AND read_at IS NULL",
    )
    .bind(conversation_id)
    .bind(reader)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// 发消息前的公共校验：非参与者直接 403
pub fn require_member<'a>(
    conversation: &'a ConversationRow,
    user: &Uuid,
) -> std::result::Result<&'a ConversationRow, AppError> {
    if conversation.user_lo == *user || conversation.user_hi == *user {
        Ok(conversation)
    } else {
        Err(AppError::forbidden("conversation 不属于当前用户"))
    }
}

/// 会话中 user_id 的对端（用于已读回执实时同步）
pub async fn peer_for_member(pool: &PgPool, conversation_id: &Uuid, user_id: &Uuid) -> ApiResult<Option<Uuid>> {
    #[derive(sqlx::FromRow)]
    struct Pair { user_lo: Uuid, user_hi: Uuid }
    let row = sqlx::query_as::<_, Pair>(
        "SELECT user_lo, user_hi FROM conversations WHERE id = $1",
    )
    .bind(conversation_id)
    .fetch_optional(pool)
    .await?;
    Ok(match row {
        Some(r) if r.user_lo == *user_id => Some(r.user_hi),
        Some(r) if r.user_hi == *user_id => Some(r.user_lo),
        _ => None,
    })
}
