//! WebSocket 实时通道（actix-ws，无 actor 的流式方案）：
//! - `RealtimeHub`：user_id -> 在线连接集合（每连接一个无界通道），负责定向转发；
//! - `ws_upgrade`：GET /ws?token=xxx —— 校验令牌后升级，随后在独立 task 中收发；
//! - `persist_and_notify`：REST 与 WS 共用的「入库 + 实时推送」入口。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use actix_web::web::Data;
use actix_web::{web, HttpRequest, HttpResponse};
use actix_ws::AggregatedMessage;
use futures_util::StreamExt as _;
use serde::Deserialize;
use sqlx::PgPool;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::error::{ApiResult, AppError};
use crate::models::chat::Message;
use crate::repositories;
use crate::state::AppState;

/// 单个在线连接在枢纽中的登记项
struct SessionEntry {
    id: Uuid,
    tx: mpsc::UnboundedSender<String>,
}

/// 实时枢纽：维护在线表并把下行负载推送给指定用户的所有连接。
/// 用普通 Mutex + 无界通道：锁内只做注册/查找，IO 都在各自 task 中，开销可忽略。
#[derive(Clone, Default)]
pub struct RealtimeHub {
    inner: Arc<Mutex<HashMap<Uuid, Vec<SessionEntry>>>>,
}

impl RealtimeHub {
    /// 注册一条新连接，返回本连接的 session id 与下行接收端
    fn register(&self, user: Uuid) -> (Uuid, mpsc::UnboundedReceiver<String>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let id = Uuid::new_v4();
        self.inner
            .lock()
            .unwrap()
            .entry(user)
            .or_default()
            .push(SessionEntry { id, tx });
        (id, rx)
    }

    /// 连接断开时注销
    fn unregister(&self, user: Uuid, id: Uuid) {
        let mut map = self.inner.lock().unwrap();
        if let Some(list) = map.get_mut(&user) {
            list.retain(|entry| entry.id != id);
            if list.is_empty() {
                map.remove(&user);
            }
        }
    }

    /// 把文本负载推送给某用户的全部在线连接
    fn deliver(&self, user: Uuid, payload: String) {
        let map = self.inner.lock().unwrap();
        if let Some(list) = map.get(&user) {
            for entry in list {
                let _ = entry.tx.send(payload.clone());
            }
        }
    }
}

// ---------------------------------------------------------------- 报文

/// 上行报文（type 为消息类型）
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Incoming {
    #[serde(rename = "type")]
    kind: String,
    conversation_id: Option<Uuid>,
    body: Option<String>,
    /// 视频面试信令：interviewId + payload(offer/answer/ice/joined)
    interview_id: Option<Uuid>,
    payload: Option<serde_json::Value>,
}

/// 事件封装工具
fn json_event(kind: &str, extra: &serde_json::Value) -> String {
    let mut value = extra.clone();
    value["type"] = serde_json::Value::String(kind.to_string());
    value.to_string()
}

/// {"type": "message", "conversationId": ..., "message": {...}}
fn message_event(message: &Message) -> String {
    json_event(
        "message",
        &serde_json::json!({
            "conversationId": message.conversation_id,
            "message": message,
        }),
    )
}

/// {"type": "error", "code": ..., "message": ...}
fn error_event(code: &str, message: impl std::fmt::Display) -> String {
    json_event(
        "error",
        &serde_json::json!({ "code": code, "message": message.to_string() }),
    )
}

// ---------------------------------------------------------------- 连接循环

/// 处理一条上行文本报文（可能涉及异步落库）
async fn handle_incoming_text(
    pool: &PgPool,
    hub: &RealtimeHub,
    user_id: Uuid,
    session: &mut actix_ws::Session,
    text: &str,
) {
    let incoming: Incoming = match serde_json::from_str(text) {
        Ok(incoming) => incoming,
        Err(_) => {
            let _ = session
                .text(error_event(
                    "bad_request",
                    "非法报文：应为 JSON { type, conversationId?, body? }",
                ))
                .await;
            return;
        }
    };

    match incoming.kind.as_str() {
        "ping" => {
            let _ = session.text(json_event("pong", &serde_json::json!({}))).await;
        }
        "send" => {
            let (Some(conversation_id), Some(body)) = (incoming.conversation_id, incoming.body)
            else {
                let _ = session
                    .text(error_event("bad_request", "send 报文需要 conversationId 与 body"))
                    .await;
                return;
            };
            let body = body.trim().to_string();
            if body.is_empty() || body.len() > 2000 {
                let _ = session
                    .text(error_event("bad_request", "body 不能为空且不超过 2000 字符"))
                    .await;
                return;
            }

            match persist_and_notify(pool, hub, &user_id, &conversation_id, &body).await {
                Ok(_) => {
                    let _ = session
                        .text(json_event(
                            "ack",
                            &serde_json::json!({ "conversationId": conversation_id }),
                        ))
                        .await;
                }
                Err(err) => {
                    let _ = session.text(error_event(err.code(), err.to_string())).await;
                }
            }
        }
        "signal" => {
            let (Some(interview_id), Some(payload)) = (incoming.interview_id, incoming.payload)
            else {
                let _ = session
                    .text(error_event("bad_request", "signal 报文需要 interviewId 与 payload"))
                    .await;
                return;
            };
            match repositories::interview::peer_of(pool, &interview_id, &user_id).await {
                Ok(Some(peer_id)) => {
                    tracing::info!(interview_id=%interview_id, from=%user_id, to=%peer_id, kind=%payload.get("kind").map(|v| v.to_string()).unwrap_or_default(), "ws interview signal forward");
                    let event = json_event(
                        "interview-signal",
                        &serde_json::json!({
                            "interviewId": interview_id,
                            "from": user_id,
                            "payload": payload,
                        }),
                    );
                    hub.deliver(peer_id, event);
                }
                Ok(None) => {
                    let _ = session
                        .text(error_event("forbidden", "非面试参与者或面试不存在"))
                        .await;
                }
                Err(err) => {
                    let _ = session.text(error_event(err.code(), err.to_string())).await;
                }
            }
        }
        other => {
            let _ = session
                .text(error_event("bad_request", format!("未知报文类型: {other}")))
                .await;
        }
    }
}

// ---------------------------------------------------------------- HTTP 升级

/// 解析 token 查询参数（浏览器 WebSocket 无法自带头，因此走 query）
fn token_from_query(query: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "token").then(|| value.to_string())
    })
}

/// GET /ws?token=xxx —— 校验令牌后升级为 WebSocket
pub async fn ws_upgrade(
    req: HttpRequest,
    stream: web::Payload,
    state: Data<AppState>,
) -> ApiResult<HttpResponse> {
    let token = token_from_query(req.query_string())
        .filter(|token| !token.is_empty())
        .ok_or_else(|| AppError::unauthorized("缺少 token 查询参数"))?;

    let user = repositories::auth::find_user_by_token(&state.pool, &token)
        .await?
        .ok_or_else(|| AppError::unauthorized("无效或已过期的 token"))?;
    tracing::info!(user_id=%user.id, "ws connected");

    let (response, mut session, msg_stream) =
        actix_ws::handle(&req, stream).map_err(|err| {
            AppError::internal(format!("websocket 握手失败: {err}"))
        })?;

    let pool = state.pool.clone();
    let hub = state.hub.clone();
    let user_id = user.id;

    actix_web::rt::spawn(async move {
        let (session_id, mut rx) = hub.register(user_id);
        let mut msg_stream = msg_stream
            .aggregate_continuations()
            .max_continuation_size(2 * 1024 * 1024);

        loop {
            tokio::select! {
                // 上行：来自客户端
                maybe = msg_stream.next() => {
                    match maybe {
                        Some(Ok(AggregatedMessage::Ping(bytes))) => {
                            let _ = session.pong(&bytes).await;
                        }
                        Some(Ok(AggregatedMessage::Text(text))) => {
                            handle_incoming_text(&pool, &hub, user_id, &mut session, &text).await;
                        }
                        Some(Err(_)) | None => break,
                        _ => {}
                    }
                }
                // 下行：来自枢纽（他人消息 / 自己的多端回执）
                Some(payload) = rx.recv() => {
                    if session.text(payload).await.is_err() {
                        break;
                    }
                }
            }
        }

        hub.unregister(user_id, session_id);
        let _ = session.close(None).await;
    });

    Ok(response)
}

// ---------------------------------------------------------------- 公共入口

/// 入库一条消息并实时推送：发给发送者（多端回执）与接收者（投递）。
/// REST 发消息与 WS 发消息共用此函数，保证行为一致。
pub async fn persist_and_notify(
    pool: &PgPool,
    hub: &RealtimeHub,
    sender: &Uuid,
    conversation_id: &Uuid,
    body: &str,
) -> ApiResult<Message> {
    let conversation = repositories::chat::conversation_for_member(pool, conversation_id, sender)
        .await?
        .ok_or_else(|| AppError::forbidden("conversation 不存在或不属于当前用户"))?;
    repositories::chat::require_member(&conversation, sender)?;

    let message = repositories::chat::insert_message(pool, &conversation, sender, body).await?;
    deliver_message(hub, &message);
    Ok(message)
}

/// 把消息负载投递给参与双方的所有在线连接
pub fn deliver_message(hub: &RealtimeHub, message: &Message) {
    let payload = message_event(message);
    hub.deliver(message.sender_id, payload.clone());
    hub.deliver(message.recipient_id, payload);
}

/// 面试状态变化：把 interview-updated 推送给双方参与者的所有在线连接
pub fn notify_interview(hub: &RealtimeHub, view: &crate::models::interview::InterviewView) {
    let payload = json_event(
        "interview-updated",
        &serde_json::json!({ "interview": view }),
    );
    hub.deliver(view.interviewer_id, payload.clone());
    hub.deliver(view.interviewee_id, payload);
}

/// 投递状态变化：推送给求职者与该企业全部招聘者（含操作者）
pub fn notify_application(hub: &RealtimeHub, view: &crate::models::application::ApplicationView, extra: &[uuid::Uuid]) {
    let payload = json_event(
        "application-updated",
        &serde_json::json!({ "application": view }),
    );
    hub.deliver(view.seeker_id, payload.clone());
    let mut seen = vec![view.seeker_id];
    for uid in extra {
        if *uid != view.seeker_id && !seen.contains(uid) {
            seen.push(*uid);
            hub.deliver(*uid, payload.clone());
        }
    }
}

/// 会话已读：告知对端（readBy 是我，用于消息已读状态同步）
pub fn notify_conversation_read(hub: &RealtimeHub, conversation_id: &uuid::Uuid, reader: &uuid::Uuid, other: &uuid::Uuid) {
    let payload = json_event(
        "conversation-read",
        &serde_json::json!({ "conversationId": conversation_id, "userId": reader }),
    );
    hub.deliver(*other, payload);
}

