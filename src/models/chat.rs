//! 即时通信模型：会话行、消息、会话列表视图。DB 列为 snake_case，JSON 为 camelCase。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::models::user::UserPublic;

/// conversations 表的一行
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationRow {
    pub id: Uuid,
    pub user_lo: Uuid,
    pub user_hi: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// messages 表的一行（同时也是 API 消息实体）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: i64,
    pub conversation_id: Uuid,
    pub sender_id: Uuid,
    pub recipient_id: Uuid,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub read_at: Option<DateTime<Utc>>,
}

/// 发起私聊请求体
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct StartConversationRequest {
    #[schema(example = "uuid-of-other-user")]
    pub user_id: Uuid,
}

/// 发送消息请求体
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NewMessage {
    #[schema(example = "你好！", max_length = 2000)]
    pub body: String,
}

/// 会话列表项视图
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub id: Uuid,
    pub peer: UserPublic,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message: Option<Message>,
    pub unread_count: i64,
}
