//! 测试公共设施：应用状态、请求构造宏与数据夹具。
//!
//! 每个用例由 `#[sqlx::test]` 提供**独立测试库**（自动建库 + 执行 `./migrations`），
//! 因此夹具之间互不干扰、也无需手工清理。

use std::sync::Arc;

use actix_web::http::{header, Method};
use actix_web::test::TestRequest;
use actix_web::web;
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::company::NewCompany;
use crate::repositories::{auth as auth_repo, chat as chat_repo};
use crate::security;
use crate::state::AppState;
use crate::ws::RealtimeHub;

/// 测试固定密码（满足 8-128 位规则）
pub const PASSWORD: &str = "secret123";

/// 用测试库连接池构造应用状态（每个用例一份，登录限流器互不影响）
pub fn state(pool: PgPool) -> web::Data<AppState> {
    web::Data::new(AppState::new(pool, Arc::new(RealtimeHub::default())))
}

/// 构造请求（可选 Bearer 令牌）
pub fn request(method: Method, uri: &str, token: Option<&str>) -> TestRequest {
    let req = TestRequest::default().method(method).uri(uri);
    match token {
        Some(token) => req.insert_header((header::AUTHORIZATION, format!("Bearer {token}"))),
        None => req,
    }
}

/// 构造带 JSON 请求体的请求（字段名 camelCase，与接口一致）
pub fn json_request(method: Method, uri: &str, token: Option<&str>, body: Value) -> TestRequest {
    request(method, uri, token).set_json(body)
}

/// 发起请求并读出 JSON 响应：返回 `(状态码, body)`；204 时 body 为 `null`。
/// 用宏而非泛型函数，避免在测试里书写 `Service<Request, ...>` 的具体类型。
macro_rules! call {
    ($app:expr, $req:expr) => {{
        let resp = actix_web::test::call_service(&$app, $req.to_request()).await;
        let status = resp.status();
        let body: serde_json::Value = if status == actix_web::http::StatusCode::NO_CONTENT {
            serde_json::Value::Null
        } else {
            actix_web::test::read_body_json(resp).await
        };
        (status, body)
    }};
}
pub(crate) use call;

/// 创建求职者并签发令牌 -> (user_id, token)
pub async fn seeker(pool: &PgPool, phone: &str) -> (Uuid, String) {
    let hash = security::hash_password(PASSWORD).unwrap();
    let user = auth_repo::register_seeker(pool, phone, "测试求职者", &hash)
        .await
        .unwrap();
    let token = auth_repo::issue_token(pool, user.id).await.unwrap();
    (user.id, token)
}

/// 创建招聘者 + 其企业并签发令牌 -> (user_id, company_id, token)
pub async fn recruiter(pool: &PgPool, phone: &str, company_name: &str) -> (Uuid, Uuid, String) {
    let hash = security::hash_password(PASSWORD).unwrap();
    let new_company = NewCompany {
        name: company_name.to_string(),
        industry: Some("互联网".to_string()),
        description: None,
        location: Some("北京".to_string()),
        address: None,
        website: None,
        logo_url: None,
        staff_size: None,
    };
    let (user, company) =
        auth_repo::register_recruiter(pool, phone, "测试招聘者", &hash, &new_company)
            .await
            .unwrap();
    let token = auth_repo::issue_token(pool, user.id).await.unwrap();
    (user.id, company.id, token)
}

/// 引导平台管理员（走生产同款 `ensure_admin`）并签发令牌 -> (user_id, token)
pub async fn admin(pool: &PgPool, phone: &str) -> (Uuid, String) {
    let outcome = auth_repo::ensure_admin(pool, phone, "平台管理员", PASSWORD)
        .await
        .unwrap();
    assert_eq!(outcome, auth_repo::AdminBootstrap::Created, "首次引导应创建 admin");
    let stored = auth_repo::find_by_phone(pool, phone)
        .await
        .unwrap()
        .expect("admin 已创建");
    let token = auth_repo::issue_token(pool, stored.user.id).await.unwrap();
    (stored.user.id, token)
}

/// 创建审核专用账号（走生产同款 `create_reviewer`）并签发令牌 -> (user_id, token)
pub async fn reviewer(pool: &PgPool, phone: &str) -> (Uuid, String) {
    reviewer_named(pool, phone, "测试审核员").await
}

/// 同上，可指定姓名（多审核账号留痕用例需要区分是谁审的）
pub async fn reviewer_named(pool: &PgPool, phone: &str, name: &str) -> (Uuid, String) {
    let hash = security::hash_password(PASSWORD).unwrap();
    let user = auth_repo::create_reviewer(pool, phone, name, &hash)
        .await
        .unwrap();
    let token = auth_repo::issue_token(pool, user.id).await.unwrap();
    (user.id, token)
}

/// 造出「求职者与该企业招聘者实际沟通过」的记录（会话 + 至少一条消息），
/// 这是发起投诉的前置条件；返回会话 id。
pub async fn make_exchange(pool: &PgPool, seeker_id: &Uuid, recruiter_id: &Uuid) -> Uuid {
    let (conversation, _created) = chat_repo::get_or_create(pool, seeker_id, recruiter_id)
        .await
        .unwrap();
    chat_repo::insert_message(
        pool,
        &conversation,
        seeker_id,
        "你好，想了解一下岗位的实际情况。",
    )
    .await
    .unwrap();
    conversation.id
}

/// 直接插企业（指定投诉次数），用于排序 / 审核用例（未申报规模）
pub async fn seed_company(pool: &PgPool, name: &str, complaints_count: i32) -> Uuid {
    seed_company_sized(pool, name, complaints_count, None).await
}

/// 同上，可指定企业规模（员工人数）—— 投诉定级 v2 的分母
pub async fn seed_company_sized(
    pool: &PgPool,
    name: &str,
    complaints_count: i32,
    staff_size: Option<i32>,
) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO companies (name, industry, location, complaints_count, staff_size)
         VALUES ($1, '互联网', '北京', $2, $3)
         RETURNING id",
    )
    .bind(name)
    .bind(complaints_count)
    .bind(staff_size)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 直接插职位（指定发布时间），用于排序用例
pub async fn seed_job(
    pool: &PgPool,
    company_id: &Uuid,
    title: &str,
    created_at: DateTime<Utc>,
) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO jobs (company_id, title, description, location, created_at)
         VALUES ($1, $2, '测试职位（排序用例）', '北京', $3)
         RETURNING id",
    )
    .bind(company_id)
    .bind(title)
    .bind(created_at)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// 相对当前时间的「几天前」（用于构造确定的发布时间次序）
pub fn days_ago(days: i64) -> DateTime<Utc> {
    Utc::now() - Duration::days(days)
}
