//! 多审核并行锁定（认领锁）测试。
//!
//! 覆盖面：
//! - 认领即锁定：他人再认领 409 且报错里带持锁人，锁状态随视图返回；
//! - 续约：同一人重复认领不改写 lockedAt，只延长到期时间；
//! - 过期可抢：锁到期后他人可重新认领；
//! - 释放规则：本人可释放、他人 403、已过期任何人可清理、管理员可 force 强制释放；
//! - 审结保护：他人持锁期间审结 409；无锁直审会补记接单时间并清空锁；
//! - 并发重复审结：第二个提交拿到 409；
//! - 权限：求职者/招聘者不能认领。

use actix_web::http::{Method, StatusCode};
use actix_web::test;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::repositories::complaint as complaint_repo;
use crate::tests::common::{self, call, json_request, request, PASSWORD};

const EVIDENCE: &str = "面试流程与岗位描述严重不符，承诺的薪资也未兑现，沟通记录已全部保留。";

async fn pending_complaint(pool: &PgPool, company: &Uuid, seeker: &Uuid) -> Uuid {
    complaint_repo::create(pool, company, seeker, EVIDENCE)
        .await
        .unwrap()
        .id
}

/// 把锁到期时间推到过去，模拟「锁已过期」
async fn expire_lock(pool: &PgPool, complaint_id: &Uuid) {
    sqlx::query("UPDATE complaints SET lock_expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(complaint_id)
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test]
async fn claim_locks_and_blocks_other_reviewers(pool: PgPool) {
    let (seeker_id, _) = common::seeker(&pool, "13800000001").await;
    let (reviewer_a, token_a) = common::reviewer_named(&pool, "13700000001", "审核员A").await;
    let (_, token_b) = common::reviewer_named(&pool, "13700000002", "审核员B").await;
    let company = common::seed_company(&pool, "锁用例企业", 0).await;
    let complaint_id = pending_complaint(&pool, &company, &seeker_id).await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;
    let uri = format!("/api/v1/complaints/{complaint_id}/claim");

    // A 认领成功，锁信息随视图返回
    let (status, body) = call!(app, request(Method::POST, &uri, Some(&token_a)));
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["lockedBy"], json!(reviewer_a.to_string()));
    assert_eq!(body["lockedByName"], json!("审核员A"));
    assert_eq!(body["lockActive"], json!(true));
    assert!(body["lockExpiresAt"].is_string());
    assert!(body["reviewStartedAt"].is_string(), "认领即记录接单时间");

    // B 认领被拒，报错里说明是谁在审
    let (status, body) = call!(app, request(Method::POST, &uri, Some(&token_b)));
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        body["message"].as_str().unwrap().contains("审核员A"),
        "报错需指明持锁人: {body}"
    );

    // 列表里也能看到锁状态（前端据此渲染「审核中」）
    let (_, list) = call!(
        app,
        request(Method::GET, "/api/v1/complaints?status=pending", Some(&token_b))
    );
    assert_eq!(list[0]["lockActive"], json!(true));
    assert_eq!(list[0]["lockedByName"], json!("审核员A"));
}

#[sqlx::test]
async fn renew_by_owner_extends_expiry_but_keeps_locked_at(pool: PgPool) {
    let (seeker_id, _) = common::seeker(&pool, "13800000001").await;
    let (_, token) = common::reviewer(&pool, "13700000001").await;
    let company = common::seed_company(&pool, "续约用例企业", 0).await;
    let complaint_id = pending_complaint(&pool, &company, &seeker_id).await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;
    let uri = format!("/api/v1/complaints/{complaint_id}/claim");

    let (_, first) = call!(app, request(Method::POST, &uri, Some(&token)));
    let (status, second) = call!(app, request(Method::POST, &uri, Some(&token)));
    assert_eq!(status, StatusCode::OK, "本人重复认领即续约，不应报错: {second}");
    assert_eq!(second["lockedAt"], first["lockedAt"], "续约不改写获得锁的时间");
    assert!(
        second["lockExpiresAt"].as_str().unwrap() >= first["lockExpiresAt"].as_str().unwrap(),
        "续约后到期时间不应提前"
    );
}

#[sqlx::test]
async fn expired_lock_can_be_reclaimed(pool: PgPool) {
    let (seeker_id, _) = common::seeker(&pool, "13800000001").await;
    let (_, token_a) = common::reviewer_named(&pool, "13700000001", "审核员A").await;
    let (reviewer_b, token_b) = common::reviewer_named(&pool, "13700000002", "审核员B").await;
    let company = common::seed_company(&pool, "过期用例企业", 0).await;
    let complaint_id = pending_complaint(&pool, &company, &seeker_id).await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;
    let uri = format!("/api/v1/complaints/{complaint_id}/claim");

    let (_, claimed) = call!(app, request(Method::POST, &uri, Some(&token_a)));
    assert_eq!(claimed["lockedByName"], json!("审核员A"));

    expire_lock(&pool, &complaint_id).await;

    let (status, body) = call!(app, request(Method::POST, &uri, Some(&token_b)));
    assert_eq!(status, StatusCode::OK, "锁过期后应可被他人重新认领: {body}");
    assert_eq!(body["lockedBy"], json!(reviewer_b.to_string()));
    assert_eq!(body["lockedByName"], json!("审核员B"));
}

#[sqlx::test]
async fn release_rules(pool: PgPool) {
    let (seeker_id, _) = common::seeker(&pool, "13800000001").await;
    let (_, admin_token) = common::admin(&pool, "13700000009").await;
    let (_, token_a) = common::reviewer_named(&pool, "13700000001", "审核员A").await;
    let (_, token_b) = common::reviewer_named(&pool, "13700000002", "审核员B").await;
    let company = common::seed_company(&pool, "释放用例企业", 0).await;
    let complaint_id = pending_complaint(&pool, &company, &seeker_id).await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;
    let claim_uri = format!("/api/v1/complaints/{complaint_id}/claim");
    let release_uri = format!("/api/v1/complaints/{complaint_id}/release");

    // B 未持锁：普通释放与其 force 都被拒
    call!(app, request(Method::POST, &claim_uri, Some(&token_a)));
    let (status, _) = call!(app, request(Method::POST, &release_uri, Some(&token_b)));
    assert_eq!(status, StatusCode::FORBIDDEN, "不能释放他人的锁");
    let (status, _) = call!(
        app,
        request(Method::POST, &format!("{release_uri}?force=true"), Some(&token_b))
    );
    assert_eq!(status, StatusCode::FORBIDDEN, "非管理员不能用 force");

    // 管理员可强制释放
    let (status, body) = call!(
        app,
        request(Method::POST, &format!("{release_uri}?force=true"), Some(&admin_token))
    );
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["lockedBy"].is_null());
    assert_eq!(body["lockActive"], json!(false));

    // 重复释放：已无锁 -> 409
    let (status, _) = call!(app, request(Method::POST, &release_uri, Some(&admin_token)));
    assert_eq!(status, StatusCode::CONFLICT);

    // 本人正常释放
    call!(app, request(Method::POST, &claim_uri, Some(&token_a)));
    let (status, body) = call!(app, request(Method::POST, &release_uri, Some(&token_a)));
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["lockedBy"].is_null());

    // 过期锁：任何人可清理
    call!(app, request(Method::POST, &claim_uri, Some(&token_a)));
    expire_lock(&pool, &complaint_id).await;
    let (status, body) = call!(app, request(Method::POST, &release_uri, Some(&token_b)));
    assert_eq!(status, StatusCode::OK, "过期锁应可被任何人清理: {body}");
    assert!(body["lockedBy"].is_null());
}

#[sqlx::test]
async fn review_is_blocked_while_locked_by_others(pool: PgPool) {
    let (seeker_id, _) = common::seeker(&pool, "13800000001").await;
    let (reviewer_a, token_a) = common::reviewer_named(&pool, "13700000001", "审核员A").await;
    let (_, token_b) = common::reviewer_named(&pool, "13700000002", "审核员B").await;
    let company = common::seed_company(&pool, "持锁审结用例企业", 0).await;
    let complaint_id = pending_complaint(&pool, &company, &seeker_id).await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    call!(
        app,
        request(Method::POST, &format!("/api/v1/complaints/{complaint_id}/claim"), Some(&token_a))
    );

    // B 未持锁 -> 409
    let (status, body) = call!(
        app,
        json_request(
            Method::POST,
            &format!("/api/v1/complaints/{complaint_id}/review"),
            Some(&token_b),
            json!({ "approved": true, "note": "抢审" }),
        )
    );
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["message"].as_str().unwrap().contains("审核中"), "{body}");

    // 持锁人 A 可以审结，且审结后清空锁、保留接单时间
    let (status, body) = call!(
        app,
        json_request(
            Method::POST,
            &format!("/api/v1/complaints/{complaint_id}/review"),
            Some(&token_a),
            json!({ "approved": true, "note": "证据充分" }),
        )
    );
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["reviewedBy"], json!(reviewer_a.to_string()));
    assert!(body["lockedBy"].is_null(), "审结后应释放锁");
    assert_eq!(body["lockActive"], json!(false));
    assert!(body["reviewStartedAt"].is_string(), "接单时间保留用于及时性统计");
}

#[sqlx::test]
async fn review_without_claim_records_start_and_concurrent_repeat_conflicts(pool: PgPool) {
    let (seeker_id, _) = common::seeker(&pool, "13800000001").await;
    let (_, token_a) = common::reviewer_named(&pool, "13700000001", "审核员A").await;
    let (_, token_b) = common::reviewer_named(&pool, "13700000002", "审核员B").await;
    let company = common::seed_company(&pool, "直审用例企业", 0).await;
    let complaint_id = pending_complaint(&pool, &company, &seeker_id).await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;
    let review_uri = format!("/api/v1/complaints/{complaint_id}/review");

    // 未认领直接审结也允许（兼容管理员代审），但会补记接单时间
    let (status, body) = call!(
        app,
        json_request(Method::POST, &review_uri, Some(&token_a), json!({ "approved": false, "note": "证据不足" }))
    );
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["reviewStartedAt"].is_string());

    // 第二个审核重复提交 -> 409（已审结）
    let (status, body) = call!(
        app,
        json_request(Method::POST, &review_uri, Some(&token_b), json!({ "approved": true }))
    );
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

#[sqlx::test]
async fn claim_and_release_require_reviewer_role(pool: PgPool) {
    let (seeker_id, _) = common::seeker(&pool, "13800000001").await;
    let (_, seeker_token) = common::seeker(&pool, "13800000002").await;
    let (_, _, recruiter_token) = common::recruiter(&pool, "13900000001", "非审核企业").await;
    let company = common::seed_company(&pool, "权限用例企业", 0).await;
    let complaint_id = pending_complaint(&pool, &company, &seeker_id).await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    for token in [&seeker_token, &recruiter_token] {
        let (status, _) = call!(
            app,
            request(Method::POST, &format!("/api/v1/complaints/{complaint_id}/claim"), Some(token))
        );
        assert_eq!(status, StatusCode::FORBIDDEN, "业务角色不能认领");
        let (status, _) = call!(
            app,
            request(Method::POST, &format!("/api/v1/complaints/{complaint_id}/release"), Some(token))
        );
        assert_eq!(status, StatusCode::FORBIDDEN, "业务角色不能释放");
    }

    let (status, _) = call!(
        app,
        request(Method::POST, &format!("/api/v1/complaints/{complaint_id}/claim"), None)
    );
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let _ = PASSWORD; // 保持与其它用例一致的常量引用
}
