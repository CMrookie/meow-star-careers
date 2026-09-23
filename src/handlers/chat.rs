//! 私聊 REST 端点：会话创建/列表、消息历史/发送/标记已读。
//! WebSocket 升级与实时推送见 `crate::ws`。

use actix_web::{web, HttpResponse};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::AuthenticatedUser;
use crate::error::{ApiResult, AppError, ErrorResponse};
use crate::models::chat::{ConversationSummary, Message, NewMessage, StartConversationRequest};
use crate::models::user::UserPublic;
use crate::repositories;
use crate::repositories::chat as chat_repo;
use crate::state::AppState;
use crate::ws;

/// 注册 /conversations 资源路由（父 scope 为 /api/v1）
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/conversations")
            .route("", web::get().to(list_conversations))
            .route("", web::post().to(start_conversation))
            .route("/{id}", web::get().to(get_conversation))
            .route("/{id}/messages", web::get().to(list_messages))
            .route("/{id}/messages", web::post().to(send_message))
            .route("/{id}/read", web::post().to(mark_read)),
    );
}

/// 消息历史查询参数
#[derive(Debug, Clone, Deserialize)]
pub struct MessagesQuery {
    /// 每页条数（默认 50，上限 200）
    pub limit: Option<i64>,
    /// 消息 id 游标：返回比该 id 更早的消息
    pub before: Option<i64>,
}

async fn peer_public(pool: &PgPool, peer_id: Uuid) -> ApiResult<UserPublic> {
    let user = repositories::user::get(pool, &peer_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("user `{peer_id}`")))?;
    Ok(UserPublic {
        id: user.id,
        name: user.name,
    })
}

/// 获取或创建与某用户的一对一会话
#[utoipa::path(
    post,
    path = "/conversations",
    tag = "chat",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "会话（已存在或刚创建）", body = ConversationSummary),
        (status = 400, description = "不能与自己聊天", body = ErrorResponse),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 404, description = "对方不存在", body = ErrorResponse),
    )
)]
pub async fn start_conversation(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    payload: web::Json<StartConversationRequest>,
) -> ApiResult<HttpResponse> {
    let me = auth.user_id;
    let other = payload.into_inner().user_id;

    if me == other {
        return Err(AppError::bad_request("不能与自己建立私聊会话"));
    }
    if !chat_repo::user_exists_active(&state.pool, &other).await? {
        return Err(AppError::not_found(format!("user `{other}`")));
    }

    let (conversation, _created) = chat_repo::get_or_create(&state.pool, &me, &other).await?;
    let peer = peer_public(&state.pool, chat_repo::other_party(&conversation, &me)).await?;

    Ok(HttpResponse::Ok().json(ConversationSummary {
        id: conversation.id,
        peer,
        created_at: conversation.created_at,
        updated_at: conversation.updated_at,
        last_message: None,
        unread_count: 0,
    }))
}

/// 我的会话列表（含对端、末条消息、未读数）
#[utoipa::path(
    get,
    path = "/conversations",
    tag = "chat",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "会话列表", body = [ConversationSummary]),
        (status = 401, description = "未认证", body = ErrorResponse),
    )
)]
pub async fn list_conversations(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
) -> ApiResult<HttpResponse> {
    let me = auth.user_id;
    let rows = chat_repo::list_for_user(&state.pool, &me).await?;

    let mut summaries = Vec::with_capacity(rows.len());
    for (conversation, last, unread) in rows {
        let peer_id = chat_repo::other_party(&conversation, &me);
        let peer = match repositories::user::get(&state.pool, &peer_id).await? {
            Some(user) => UserPublic {
                id: user.id,
                name: user.name,
            },
            None => continue,
        };
        summaries.push(ConversationSummary {
            id: conversation.id,
            peer,
            created_at: conversation.created_at,
            updated_at: conversation.updated_at,
            last_message: last,
            unread_count: unread,
        });
    }

    Ok(HttpResponse::Ok().json(summaries))
}

/// 会话详情（含对端与未读数）
#[utoipa::path(
    get,
    path = "/conversations/{id}",
    tag = "chat",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "会话 ID")),
    responses(
        (status = 200, description = "会话详情", body = ConversationSummary),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非会话成员", body = ErrorResponse),
    )
)]
pub async fn get_conversation(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
) -> ApiResult<HttpResponse> {
    let me = auth.user_id;
    let conversation_id = path.into_inner();

    let conversation = chat_repo::conversation_for_member(&state.pool, &conversation_id, &me)
        .await?
        .ok_or_else(|| AppError::forbidden("conversation 不存在或不属于当前用户"))?;

    let peer_id = chat_repo::other_party(&conversation, &me);
    let peer = peer_public(&state.pool, peer_id).await?;
    let unread = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM messages
          WHERE conversation_id = $1 AND recipient_id = $2 AND read_at IS NULL",
    )
    .bind(conversation.id)
    .bind(me)
    .fetch_one(&state.pool)
    .await?;

    Ok(HttpResponse::Ok().json(ConversationSummary {
        id: conversation.id,
        peer,
        created_at: conversation.created_at,
        updated_at: conversation.updated_at,
        last_message: None,
        unread_count: unread,
    }))
}

/// 会话消息历史（升序返回）
#[utoipa::path(
    get,
    path = "/conversations/{id}/messages",
    tag = "chat",
    security(("bearerAuth" = [])),
    params(
        ("id" = Uuid, Path, description = "会话 ID"),
        ("limit" = Option<i64>, Query, description = "每页条数（默认 50，上限 200）"),
        ("before" = Option<i64>, Query, description = "消息 id 游标：更早的消息"),
    ),
    responses(
        (status = 200, description = "消息列表（升序）", body = [Message]),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非会话成员", body = ErrorResponse),
    )
)]
pub async fn list_messages(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
    query: web::Query<MessagesQuery>,
) -> ApiResult<HttpResponse> {
    let conversation_id = path.into_inner();

    let conversation = chat_repo::conversation_for_member(&state.pool, &conversation_id, &auth.user_id)
        .await?
        .ok_or_else(|| AppError::forbidden("conversation 不存在或不属于当前用户"))?;

    let messages = chat_repo::list_messages(
        &state.pool,
        &conversation.id,
        query.before,
        query.limit.unwrap_or(50),
    )
    .await?;

    Ok(HttpResponse::Ok().json(messages))
}

/// 发送消息（REST 通道；实时通道见 /ws）
#[utoipa::path(
    post,
    path = "/conversations/{id}/messages",
    tag = "chat",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "会话 ID")),
    responses(
        (status = 201, description = "发送成功（同时实时推送给双方在线连接）", body = Message),
        (status = 400, description = "内容为空或超长", body = ErrorResponse),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非会话成员", body = ErrorResponse),
    )
)]
pub async fn send_message(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
    payload: web::Json<NewMessage>,
) -> ApiResult<HttpResponse> {
    let conversation_id = path.into_inner();
    let body = payload.into_inner().body.trim().to_string();
    if body.is_empty() || body.len() > 2000 {
        return Err(AppError::bad_request("body 不能为空且不超过 2000 字符"));
    }

    let message = ws::persist_and_notify(
        &state.pool,
        &state.hub,
        &auth.user_id,
        &conversation_id,
        &body,
    )
    .await?;

    Ok(HttpResponse::Created().json(message))
}

/// 把该会话中对方发给我的消息标记为已读
#[utoipa::path(
    post,
    path = "/conversations/{id}/read",
    tag = "chat",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "会话 ID")),
    responses(
        (status = 200, description = "标记完成，返回本次标记条数"),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非会话成员", body = ErrorResponse),
    )
)]
pub async fn mark_read(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
    path: web::Path<Uuid>,
) -> ApiResult<HttpResponse> {
    let conversation_id = path.into_inner();

    let conversation = chat_repo::conversation_for_member(&state.pool, &conversation_id, &auth.user_id)
        .await?
        .ok_or_else(|| AppError::forbidden("conversation 不存在或不属于当前用户"))?;

    let marked = chat_repo::mark_conversation_read(&state.pool, &conversation.id, &auth.user_id).await?;
    if let Ok(Some(peer)) = chat_repo::peer_for_member(&state.pool, &conversation.id, &auth.user_id).await {
        ws::notify_conversation_read(&state.hub, &conversation.id, &auth.user_id, &peer);
    }
    Ok(HttpResponse::Ok().json(serde_json::json!({ "marked": marked })))
}
