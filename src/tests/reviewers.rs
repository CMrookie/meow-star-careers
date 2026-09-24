//! 审核专用账号（reviewer）测试：由平台管理员创建管理、多账号同时在线并行审核。
//!
//! 覆盖面：
//! - 管理权限：仅 admin 能创建/列表/启停/改密/删除（求职者、招聘者、审核账号自身都 403）；
//!   该 API 只认 `role=reviewer` 的账号，对其它角色一律 404（防误伤/防探测）；
//! - 账号可用性：admin 创建后可用手机号+初始密码登录并审核，留痕 reviewedBy/ByName；
//! - 管理动作：禁用即踢下线且登录 403；重置密码后旧会话/旧密码同时失效；删除后账号与会话一起消失；
//! - 并发登录：多个审核账号互不干扰地同时在线审核；同一账号可多端登录（登出其一不影响另一个）；
//!   某个审核连续登录失败也不会波及别的审核（限流键为「手机号|IP」）。

use actix_web::http::{Method, StatusCode};
use actix_web::test;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::repositories::complaint as complaint_repo;
use crate::tests::common::{self, call, json_request, request, PASSWORD};

/// 满足「证据 >= 20 字」的投诉文案
const EVIDENCE: &str = "面试时承诺的薪资与实际发放不一致，且要求无薪试岗两周。";

/// 通过 HTTP 创建审核账号（admin 令牌）
macro_rules! create_reviewer {
    ($app:expr, $admin:expr, $phone:expr, $name:expr, $password:expr) => {{
        crate::tests::common::call!(
            $app,
            crate::tests::common::json_request(
                actix_web::http::Method::POST,
                "/api/v1/reviewers",
                Some($admin),
                serde_json::json!({"phone": $phone, "name": $name, "password": $password}),
            )
        )
    }};
}

/// 通过 HTTP 登录，返回 (状态码, body)
macro_rules! login {
    ($app:expr, $phone:expr, $password:expr) => {{
        crate::tests::common::call!(
            $app,
            crate::tests::common::json_request(
                actix_web::http::Method::POST,
                "/api/v1/auth/login",
                None,
                serde_json::json!({"phone": $phone, "password": $password}),
            )
        )
    }};
}

/// 直接造一条待审投诉（跳过沟通前置，专注审核账号用例）
async fn pending_complaint(pool: &PgPool, company: &Uuid, seeker: &Uuid) -> String {
    complaint_repo::create(pool, company, seeker, EVIDENCE)
        .await
        .unwrap()
        .id
        .to_string()
}

#[sqlx::test]
async fn admin_creates_reviewer_who_can_login_and_review(pool: PgPool) {
    let (_, admin_token) = common::admin(&pool, "13700000001").await;
    let (seeker_id, _) = common::seeker(&pool, "13800000001").await;
    let company = common::seed_company(&pool, "审核员用例企业", 0).await;
    let complaint_id = pending_complaint(&pool, &company, &seeker_id).await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    // 1) admin 创建审核账号
    let (status, created) = create_reviewer!(app, &admin_token, "13700000002", "审核员小王", PASSWORD);
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["role"], json!("reviewer"));
    assert_eq!(created["phone"], json!("13700000002"));
    assert_eq!(created["isActive"], json!(true));
    let reviewer_id = created["id"].as_str().unwrap().to_string();

    // 2) 审核账号可用手机号 + 初始密码登录
    let (status, body) = login!(app, "13700000002", PASSWORD);
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user"]["role"], json!("reviewer"));
    let reviewer_token = body["token"].as_str().unwrap().to_string();

    // 3) 能看到待审队列，并能审核（留痕 + 企业计数生效）
    let (status, queue) = call!(
        app,
        request(Method::GET, "/api/v1/complaints?status=pending", Some(&reviewer_token))
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(queue.as_array().unwrap().len(), 1);

    let (status, reviewed) = call!(
        app,
        json_request(
            Method::POST,
            &format!("/api/v1/complaints/{complaint_id}/review"),
            Some(&reviewer_token),
            json!({"approved": true, "note": "证据充分"}),
        )
    );
    assert_eq!(status, StatusCode::OK, "{reviewed}");
    assert_eq!(reviewed["status"], json!("approved"));
    assert_eq!(reviewed["reviewedBy"], json!(reviewer_id));
    assert_eq!(reviewed["reviewedByName"], json!("审核员小王"), "多审核并行时需能追溯是谁审的");

    let (_, company_body) = call!(
        app,
        request(Method::GET, &format!("/api/v1/companies/{company}"), Some(&reviewer_token))
    );
    assert_eq!(company_body["complaintsCount"], json!(1));
}

#[sqlx::test]
async fn reviewer_management_requires_admin(pool: PgPool) {
    let (_, seeker_token) = common::seeker(&pool, "13800000001").await;
    let (_, _, recruiter_token) = common::recruiter(&pool, "13900000001", "非管理员企业").await;
    let (_, reviewer_token) = common::reviewer(&pool, "13700000002").await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    for token in [&seeker_token, &recruiter_token, &reviewer_token] {
        let (status, _) = call!(app, request(Method::GET, "/api/v1/reviewers", Some(token)));
        assert_eq!(status, StatusCode::FORBIDDEN, "列表仅 admin");
        let (status, _) = create_reviewer!(app, token, "13700000003", "越权创建", PASSWORD);
        assert_eq!(status, StatusCode::FORBIDDEN, "创建仅 admin");
    }

    // 未认证
    let (status, _) = call!(app, request(Method::GET, "/api/v1/reviewers", None));
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn reviewer_creation_validates_input_and_duplicates(pool: PgPool) {
    let (_, admin_token) = common::admin(&pool, "13700000001").await;
    let (_, _, _) = common::recruiter(&pool, "13900000001", "已存在企业").await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    // 手机号不合法 / 密码过短 / 姓名为空
    for (phone, name, password) in [
        ("1370000", "审核员", PASSWORD),
        ("13700000004", "审核员", "short"),
        ("13700000005", "   ", PASSWORD),
    ] {
        let (status, body) = create_reviewer!(app, &admin_token, phone, name, password);
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }

    // 手机号已被占用（招聘者账号）-> 409
    let (status, body) = create_reviewer!(app, &admin_token, "13900000001", "重复手机号", PASSWORD);
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

#[sqlx::test]
async fn reviewer_list_contains_only_reviewers(pool: PgPool) {
    let (_, admin_token) = common::admin(&pool, "13700000001").await;
    let _ = common::seeker(&pool, "13800000001").await;
    let _ = common::recruiter(&pool, "13900000001", "列表用例企业").await;
    let (first_id, _) = common::reviewer(&pool, "13700000002").await;
    let (second_id, _) = common::reviewer(&pool, "13700000003").await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    let (status, list) = call!(app, request(Method::GET, "/api/v1/reviewers", Some(&admin_token)));
    assert_eq!(status, StatusCode::OK);
    let items = list.as_array().unwrap();
    assert_eq!(items.len(), 2, "只列审核账号：{list}");
    assert!(items.iter().all(|u| u["role"] == json!("reviewer")));
    let ids: Vec<&str> = items.iter().map(|u| u["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&first_id.to_string().as_str()));
    assert!(ids.contains(&second_id.to_string().as_str()));
}

#[sqlx::test]
async fn disabling_reviewer_kicks_sessions_and_blocks_login(pool: PgPool) {
    let (_, admin_token) = common::admin(&pool, "13700000001").await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    let (_, created) = create_reviewer!(app, &admin_token, "13700000002", "待禁用审核", PASSWORD);
    let reviewer_id = created["id"].as_str().unwrap().to_string();
    let (_, body) = login!(app, "13700000002", PASSWORD);
    let token = body["token"].as_str().unwrap().to_string();

    // 禁用：既有会话立即失效 + 重新登录被拒
    let (status, updated) = call!(
        app,
        json_request(
            Method::POST,
            &format!("/api/v1/reviewers/{reviewer_id}/active"),
            Some(&admin_token),
            json!({"isActive": false}),
        )
    );
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["isActive"], json!(false));
    let (status, _) = call!(app, request(Method::GET, "/api/v1/auth/me", Some(&token)));
    assert_eq!(status, StatusCode::UNAUTHORIZED, "禁用即踢下线");
    let (status, _) = login!(app, "13700000002", PASSWORD);
    assert_eq!(status, StatusCode::FORBIDDEN, "禁用账号不可登录");

    // 重新启用后可登录
    let (status, updated) = call!(
        app,
        json_request(
            Method::POST,
            &format!("/api/v1/reviewers/{reviewer_id}/active"),
            Some(&admin_token),
            json!({"isActive": true}),
        )
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["isActive"], json!(true));
    let (status, _) = login!(app, "13700000002", PASSWORD);
    assert_eq!(status, StatusCode::OK, "启用后可再次登录");
}

#[sqlx::test]
async fn password_reset_revokes_sessions_and_switches_password(pool: PgPool) {
    let (_, admin_token) = common::admin(&pool, "13700000001").await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    let (_, created) = create_reviewer!(app, &admin_token, "13700000002", "待改密审核", PASSWORD);
    let reviewer_id = created["id"].as_str().unwrap().to_string();
    let (_, body) = login!(app, "13700000002", PASSWORD);
    let old_token = body["token"].as_str().unwrap().to_string();

    let (status, _) = call!(
        app,
        json_request(
            Method::POST,
            &format!("/api/v1/reviewers/{reviewer_id}/password"),
            Some(&admin_token),
            json!({"password": "new-secret-123"}),
        )
    );
    assert_eq!(status, StatusCode::NO_CONTENT);

    // 旧会话被撤销、旧密码失效、新密码可用
    let (status, _) = call!(app, request(Method::GET, "/api/v1/auth/me", Some(&old_token)));
    assert_eq!(status, StatusCode::UNAUTHORIZED, "改密后旧会话失效");
    let (status, _) = login!(app, "13700000002", PASSWORD);
    assert_eq!(status, StatusCode::UNAUTHORIZED, "旧密码不可用");
    let (status, _) = login!(app, "13700000002", "new-secret-123");
    assert_eq!(status, StatusCode::OK, "新密码可登录");

    // 密码长度不合法 -> 400
    let (status, _) = call!(
        app,
        json_request(
            Method::POST,
            &format!("/api/v1/reviewers/{reviewer_id}/password"),
            Some(&admin_token),
            json!({"password": "short"}),
        )
    );
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn delete_reviewer_removes_account_and_sessions(pool: PgPool) {
    let (_, admin_token) = common::admin(&pool, "13700000001").await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    let (_, created) = create_reviewer!(app, &admin_token, "13700000002", "待删除审核", PASSWORD);
    let reviewer_id = created["id"].as_str().unwrap().to_string();
    let (_, body) = login!(app, "13700000002", PASSWORD);
    let token = body["token"].as_str().unwrap().to_string();

    let (status, _) = call!(
        app,
        request(Method::DELETE, &format!("/api/v1/reviewers/{reviewer_id}"), Some(&admin_token))
    );
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = call!(app, request(Method::GET, "/api/v1/auth/me", Some(&token)));
    assert_eq!(status, StatusCode::UNAUTHORIZED, "账号删除后令牌随之失效");
    let (status, _) = login!(app, "13700000002", PASSWORD);
    assert_eq!(status, StatusCode::UNAUTHORIZED, "账号已不存在");
    let (status, _) = call!(
        app,
        request(Method::DELETE, &format!("/api/v1/reviewers/{reviewer_id}"), Some(&admin_token))
    );
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (_, list) = call!(app, request(Method::GET, "/api/v1/reviewers", Some(&admin_token)));
    assert!(list.as_array().unwrap().is_empty());
}

#[sqlx::test]
async fn reviewer_api_refuses_non_reviewer_accounts(pool: PgPool) {
    let (_, admin_token) = common::admin(&pool, "13700000001").await;
    let (seeker_id, _) = common::seeker(&pool, "13800000001").await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    // 用审核账号管理接口操作普通求职者 -> 一律 404（防误伤与探测）
    let (status, _) = call!(
        app,
        json_request(
            Method::POST,
            &format!("/api/v1/reviewers/{seeker_id}/active"),
            Some(&admin_token),
            json!({"isActive": false}),
        )
    );
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call!(
        app,
        json_request(
            Method::POST,
            &format!("/api/v1/reviewers/{seeker_id}/password"),
            Some(&admin_token),
            json!({"password": "new-secret-123"}),
        )
    );
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call!(
        app,
        request(Method::DELETE, &format!("/api/v1/reviewers/{seeker_id}"), Some(&admin_token))
    );
    assert_eq!(status, StatusCode::NOT_FOUND);

    // 目标账号仍在
    let (status, body) = call!(
        app,
        request(Method::GET, &format!("/api/v1/users/{seeker_id}"), Some(&admin_token))
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["role"], json!("seeker"));
}

#[sqlx::test]
async fn multiple_reviewers_review_in_parallel_with_audit_trail(pool: PgPool) {
    let (_, admin_token) = common::admin(&pool, "13700000001").await;
    let (seeker_id, _) = common::seeker(&pool, "13800000001").await;
    let company = common::seed_company(&pool, "并行审核企业", 0).await;
    let first_complaint = pending_complaint(&pool, &company, &seeker_id).await;
    let second_complaint = pending_complaint(&pool, &company, &seeker_id).await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    // 两个审核账号同时在线（各自独立令牌）
    let (_, a) = create_reviewer!(app, &admin_token, "13700000002", "审核员A", PASSWORD);
    let (_, b) = create_reviewer!(app, &admin_token, "13700000003", "审核员B", PASSWORD);
    let a_id = a["id"].as_str().unwrap().to_string();
    let b_id = b["id"].as_str().unwrap().to_string();
    let (_, body_a) = login!(app, "13700000002", PASSWORD);
    let (_, body_b) = login!(app, "13700000003", PASSWORD);
    let token_a = body_a["token"].as_str().unwrap().to_string();
    let token_b = body_b["token"].as_str().unwrap().to_string();
    assert_ne!(token_a, token_b);

    // 两条令牌同时有效
    for token in [&token_a, &token_b] {
        let (status, _) = call!(app, request(Method::GET, "/api/v1/complaints", Some(token)));
        assert_eq!(status, StatusCode::OK, "两个审核应能同时在线");
    }

    // A 审第一条、B 审第二条：都成功，且各自留痕
    let (status, first) = call!(
        app,
        json_request(
            Method::POST,
            &format!("/api/v1/complaints/{first_complaint}/review"),
            Some(&token_a),
            json!({"approved": true}),
        )
    );
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["reviewedBy"], json!(a_id));
    assert_eq!(first["reviewedByName"], json!("审核员A"));

    let (status, second) = call!(
        app,
        json_request(
            Method::POST,
            &format!("/api/v1/complaints/{second_complaint}/review"),
            Some(&token_b),
            json!({"approved": false, "note": "证据不足"}),
        )
    );
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["reviewedBy"], json!(b_id));
    assert_eq!(second["reviewedByName"], json!("审核员B"));

    // 通过 1 条、驳回 1 条 -> 企业投诉次数为 1
    let (_, company_body) = call!(
        app,
        request(Method::GET, &format!("/api/v1/companies/{company}"), Some(&token_a))
    );
    assert_eq!(company_body["complaintsCount"], json!(1));
}

#[sqlx::test]
async fn same_reviewer_can_hold_multiple_sessions(pool: PgPool) {
    let (_, admin_token) = common::admin(&pool, "13700000001").await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    let (_, created) = create_reviewer!(app, &admin_token, "13700000002", "多端审核", PASSWORD);
    assert_eq!(created["role"], json!("reviewer"));

    // 同一账号登录两次（例如两台设备）
    let (_, first) = login!(app, "13700000002", PASSWORD);
    let (_, second) = login!(app, "13700000002", PASSWORD);
    let token_one = first["token"].as_str().unwrap().to_string();
    let token_two = second["token"].as_str().unwrap().to_string();
    assert_ne!(token_one, token_two);

    for token in [&token_one, &token_two] {
        let (status, _) = call!(app, request(Method::GET, "/api/v1/auth/me", Some(token)));
        assert_eq!(status, StatusCode::OK, "两次登录的令牌应同时有效");
    }

    // 登出其中一个不影响另一个
    let (status, _) = call!(
        app,
        request(Method::POST, "/api/v1/auth/logout", Some(&token_one))
    );
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call!(app, request(Method::GET, "/api/v1/auth/me", Some(&token_one)));
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call!(app, request(Method::GET, "/api/v1/auth/me", Some(&token_two)));
    assert_eq!(status, StatusCode::OK, "登出其一不影响另一会话");
}

#[sqlx::test]
async fn one_reviewer_lockout_does_not_block_others(pool: PgPool) {
    let (_, admin_token) = common::admin(&pool, "13700000001").await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    let _ = create_reviewer!(app, &admin_token, "13700000002", "审核员甲", PASSWORD);
    let _ = create_reviewer!(app, &admin_token, "13700000003", "审核员乙", PASSWORD);

    // 甲连续失败 5 次 -> 甲被冻结
    for _ in 0..5 {
        let (status, _) = login!(app, "13700000002", "wrong-password");
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, _) = login!(app, "13700000002", PASSWORD);
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    // 乙不受影响（限流键含手机号，审核之间互不牵连）
    let (status, body) = login!(app, "13700000003", PASSWORD);
    assert_eq!(status, StatusCode::OK, "另一个审核仍可登录: {body}");
}

#[sqlx::test]
async fn reviewer_cannot_use_seeker_or_recruiter_endpoints(pool: PgPool) {
    let (_, reviewer_token) = common::reviewer(&pool, "13700000002").await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    // 审核账号的职责边界：能看/审投诉，但不能发职位、投递、维护简历
    let (status, _) = call!(
        app,
        json_request(
            Method::POST,
            "/api/v1/jobs",
            Some(&reviewer_token),
            json!({"title": "越权职位", "description": "不应成功"}),
        )
    );
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call!(
        app,
        json_request(
            Method::GET,
            "/api/v1/resumes",
            Some(&reviewer_token),
            json!({}),
        )
    );
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 但投诉队列可读
    let (status, _) = call!(app, request(Method::GET, "/api/v1/complaints", Some(&reviewer_token)));
    assert_eq!(status, StatusCode::OK);
}

#[sqlx::test]
async fn unknown_reviewer_id_returns_not_found(pool: PgPool) {
    let (_, admin_token) = common::admin(&pool, "13700000001").await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    let missing = Uuid::new_v4();
    let (status, _) = call!(
        app,
        json_request(
            Method::POST,
            &format!("/api/v1/reviewers/{missing}/active"),
            Some(&admin_token),
            json!({"isActive": false}),
        )
    );
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call!(
        app,
        request(Method::DELETE, &format!("/api/v1/reviewers/{missing}"), Some(&admin_token))
    );
    assert_eq!(status, StatusCode::NOT_FOUND);
}
