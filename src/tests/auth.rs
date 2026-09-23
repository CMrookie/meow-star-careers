//! 用户鉴权测试：注册 / 登录 / 登出、令牌校验、角色与越权防护。
//!
//! 覆盖面：
//! - 令牌：缺失/伪造/已登出/账号被禁用 一律 401；
//! - 注册：手机号与密码校验、重复手机号 409、**admin 不可自助注册**；
//! - 登录：密码错误与账号不存在返回同样的 401（防探测）、连续失败限流 429；
//! - 角色：求职者不能发布职位、招聘者/求职者不能访问管理员接口；
//! - 越权：/users 仅管理员可用，本人可查改自己，改他人 403、查他人 404。

use actix_web::http::{Method, StatusCode};
use actix_web::test;
use serde_json::json;
use sqlx::PgPool;
use crate::error::AppError;
use crate::repositories::auth::{self as auth_repo, AdminBootstrap};
use crate::tests::common::{self, call, json_request, request, PASSWORD};

#[sqlx::test]
async fn healthz_public_and_business_endpoints_need_token(pool: PgPool) {
    let app = test::init_service(crate::app::create_app(common::state(pool))).await;

    let (status, _) = call!(app, request(Method::GET, "/healthz", None));
    assert_eq!(status, StatusCode::OK);

    // 无令牌 / 伪造令牌：一律 401
    let (status, _) = call!(app, request(Method::GET, "/api/v1/jobs", None));
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call!(
        app,
        request(Method::GET, "/api/v1/auth/me", Some("deadbeef-not-a-token"))
    );
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn register_login_me_logout_flow(pool: PgPool) {
    let app = test::init_service(crate::app::create_app(common::state(pool))).await;

    let (status, body) = call!(
        app,
        json_request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            json!({"phone": "13800000001", "name": "小明", "password": PASSWORD}),
        )
    );
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    let token = body["token"].as_str().expect("注册应返回 token").to_string();
    assert_eq!(token.len(), 32, "令牌为 32 位十六进制");
    assert_eq!(body["user"]["role"], json!("seeker"));
    assert_eq!(body["user"]["phone"], json!("13800000001"));

    // 令牌可用
    let (status, me) = call!(app, request(Method::GET, "/api/v1/auth/me", Some(&token)));
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["phone"], json!("13800000001"));

    // 登出后令牌立即失效
    let (status, _) = call!(
        app,
        request(Method::POST, "/api/v1/auth/logout", Some(&token))
    );
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call!(app, request(Method::GET, "/api/v1/auth/me", Some(&token)));
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // 重新登录拿新令牌
    let (status, body) = call!(
        app,
        json_request(
            Method::POST,
            "/api/v1/auth/login",
            None,
            json!({"phone": "13800000001", "password": PASSWORD}),
        )
    );
    assert_eq!(status, StatusCode::OK);
    assert!(body["token"].as_str().is_some_and(|t| t.len() == 32));

    // 密码错误 / 手机号不存在：同为 401 且文案一致（不泄露账号是否存在）
    let (status, wrong_password) = call!(
        app,
        json_request(
            Method::POST,
            "/api/v1/auth/login",
            None,
            json!({"phone": "13800000001", "password": "wrong-password"}),
        )
    );
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, unknown_phone) = call!(
        app,
        json_request(
            Method::POST,
            "/api/v1/auth/login",
            None,
            json!({"phone": "13800009999", "password": PASSWORD}),
        )
    );
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong_password["message"], unknown_phone["message"]);
}

#[sqlx::test]
async fn register_validates_input_and_rejects_duplicates(pool: PgPool) {
    let app = test::init_service(crate::app::create_app(common::state(pool))).await;

    // 手机号不合法 / 密码过短 / 招聘者缺企业名
    for payload in [
        json!({"phone": "1380000", "name": "小明", "password": PASSWORD}),
        json!({"phone": "13800000002", "name": "小明", "password": "short"}),
        json!({"phone": "13800000003", "name": "HR", "password": PASSWORD, "role": "recruiter"}),
    ] {
        let (status, _) = call!(app, json_request(Method::POST, "/api/v1/auth/register", None, payload));
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // 正常注册后重复注册同手机号 -> 409
    let body = json!({"phone": "13800000004", "name": "小明", "password": PASSWORD});
    let (status, _) = call!(app, json_request(Method::POST, "/api/v1/auth/register", None, body.clone()));
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = call!(app, json_request(Method::POST, "/api/v1/auth/register", None, body));
    assert_eq!(status, StatusCode::CONFLICT);
}

#[sqlx::test]
async fn admin_cannot_self_register(pool: PgPool) {
    let app = test::init_service(crate::app::create_app(common::state(pool))).await;

    let (status, body) = call!(
        app,
        json_request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            json!({"phone": "13900000001", "name": "伪管理员", "password": PASSWORD, "role": "admin"}),
        )
    );
    assert_eq!(status, StatusCode::BAD_REQUEST, "admin 不可自助注册: {body}");
    assert!(
        body["message"].as_str().unwrap_or_default().contains("不可自助注册"),
        "应给出明确原因: {body}"
    );
}

#[sqlx::test]
async fn login_is_rate_limited_after_repeated_failures(pool: PgPool) {
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;
    let _ = common::seeker(&pool, "13800000001").await;

    let wrong = json!({"phone": "13800000001", "password": "wrong-password"});
    for attempt in 1..=5 {
        let (status, _) = call!(
            app,
            json_request(Method::POST, "/api/v1/auth/login", None, wrong.clone())
        );
        assert_eq!(status, StatusCode::UNAUTHORIZED, "第 {attempt} 次失败应 401");
    }

    // 达到失败上限：即使密码正确也先被冻结（429）
    let (status, _) = call!(
        app,
        json_request(
            Method::POST,
            "/api/v1/auth/login",
            None,
            json!({"phone": "13800000001", "password": PASSWORD}),
        )
    );
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test]
async fn role_gates_on_job_publishing(pool: PgPool) {
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;
    let (_, seeker_token) = common::seeker(&pool, "13800000001").await;
    let (_, _, recruiter_token) = common::recruiter(&pool, "13900000001", "角色校验企业").await;

    let job = json!({"title": "后端工程师", "description": "负责服务端开发", "location": "北京"});

    // 求职者不能发布职位
    let (status, _) = call!(
        app,
        json_request(Method::POST, "/api/v1/jobs", Some(&seeker_token), job.clone())
    );
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 招聘者可以发布；求职者可以浏览
    let (status, created) = call!(
        app,
        json_request(Method::POST, "/api/v1/jobs", Some(&recruiter_token), job)
    );
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["complaintsCount"], json!(0), "新职位所在企业 0 次投诉");

    let (status, list) = call!(app, request(Method::GET, "/api/v1/jobs", Some(&seeker_token)));
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["total"], json!(1));
}

#[sqlx::test]
async fn users_endpoints_are_admin_only_and_self_scoped(pool: PgPool) {
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;
    let (seeker_id, seeker_token) = common::seeker(&pool, "13800000001").await;
    let (other_id, _other_token) = common::seeker(&pool, "13800000002").await;
    let (admin_id, admin_token) = common::admin(&pool, "13900000001").await;

    // 列表：仅管理员
    let (status, _) = call!(app, request(Method::GET, "/api/v1/users", Some(&seeker_token)));
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, users) = call!(app, request(Method::GET, "/api/v1/users", Some(&admin_token)));
    assert_eq!(status, StatusCode::OK);
    assert!(users.as_array().is_some_and(|list| list.len() >= 3));

    // 查自己 200；查他人 404（不泄露账号存在性）
    let (status, me) = call!(
        app,
        request(Method::GET, &format!("/api/v1/users/{seeker_id}"), Some(&seeker_token))
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["id"], json!(seeker_id.to_string()));
    let (status, _) = call!(
        app,
        request(Method::GET, &format!("/api/v1/users/{other_id}"), Some(&seeker_token))
    );
    assert_eq!(status, StatusCode::NOT_FOUND);

    // 改他人 403；改自己 200
    let (status, _) = call!(
        app,
        json_request(
            Method::PUT,
            &format!("/api/v1/users/{other_id}"),
            Some(&seeker_token),
            json!({"name": "被篡改"}),
        )
    );
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, updated) = call!(
        app,
        json_request(
            Method::PUT,
            &format!("/api/v1/users/{seeker_id}"),
            Some(&seeker_token),
            json!({"name": "本人改名"}),
        )
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["name"], json!("本人改名"));

    // 删除：非管理员 403
    let (status, _) = call!(
        app,
        request(Method::DELETE, &format!("/api/v1/users/{other_id}"), Some(&seeker_token))
    );
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 管理员禁用账号：既有令牌立即失效、重新登录被拒（403）
    let (status, disabled) = call!(
        app,
        json_request(
            Method::PUT,
            &format!("/api/v1/users/{seeker_id}"),
            Some(&admin_token),
            json!({"isActive": false}),
        )
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(disabled["isActive"], json!(false));
    let (status, _) = call!(app, request(Method::GET, "/api/v1/auth/me", Some(&seeker_token)));
    assert_eq!(status, StatusCode::UNAUTHORIZED, "禁用后既有令牌应立刻失效");
    let (status, _) = call!(
        app,
        json_request(
            Method::POST,
            "/api/v1/auth/login",
            None,
            json!({"phone": "13800000001", "password": PASSWORD}),
        )
    );
    assert_eq!(status, StatusCode::FORBIDDEN, "禁用账号登录应 403");

    // 管理员删除用户
    let (status, _) = call!(
        app,
        request(Method::DELETE, &format!("/api/v1/users/{other_id}"), Some(&admin_token))
    );
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call!(
        app,
        request(Method::GET, &format!("/api/v1/users/{other_id}"), Some(&admin_token))
    );
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_ne!(admin_id, seeker_id);
}

#[sqlx::test]
async fn admin_bootstrap_is_idempotent_and_never_promotes(pool: PgPool) {
    // 首次引导创建
    assert_eq!(
        auth_repo::ensure_admin(&pool, "13900000001", "平台管理员", PASSWORD)
            .await
            .unwrap(),
        AdminBootstrap::Created
    );
    // 幂等：不重复创建、不覆盖密码
    assert_eq!(
        auth_repo::ensure_admin(&pool, "13900000001", "平台管理员", PASSWORD)
            .await
            .unwrap(),
        AdminBootstrap::AlreadyExists
    );

    // 手机号已被求职者占用 -> 拒绝自动提权（409）
    let _ = common::seeker(&pool, "13800000001").await;
    let err = auth_repo::ensure_admin(&pool, "13800000001", "平台管理员", PASSWORD)
        .await
        .expect_err("不得把既有账号提升为 admin");
    assert!(matches!(err, AppError::Conflict(_)), "应为 409，实际 {err:?}");

    // 非法手机号 / 短密码
    assert!(auth_repo::ensure_admin(&pool, "123", "管理员", PASSWORD).await.is_err());
    assert!(auth_repo::ensure_admin(&pool, "13900000002", "管理员", "short").await.is_err());

    // 引导出来的账号可用密码登录，且角色为 admin
    let app = test::init_service(crate::app::create_app(common::state(pool))).await;
    let (status, body) = call!(
        app,
        json_request(
            Method::POST,
            "/api/v1/auth/login",
            None,
            json!({"phone": "13900000001", "password": PASSWORD}),
        )
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["user"]["role"], json!("admin"));
}
