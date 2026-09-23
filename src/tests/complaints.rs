//! 举报（投诉）审核测试：发起条件、管理员审核、状态流转与对排名的影响。
//!
//! 覆盖面：
//! - 发起：仅求职者、需与该企业有过实际沟通、证据 20-5000 字；
//! - 审核权限：仅 admin（求职者/招聘者 403）；重复审核 409；不存在 404；备注超长 400；
//! - 审核生效：通过 -> 企业 complaints_count +1 且职位列表里该企业**降档**；
//!   驳回 -> 计数与顺序都不变；
//! - 可见范围：管理员=全部（可 ?status= 过滤，非法取值 400）、招聘者=本企业、求职者=本人。

use actix_web::http::{Method, StatusCode};
use actix_web::test;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::tests::common::{self, call, json_request, request};

/// 满足「证据 >= 20 字」的投诉文案
const EVIDENCE: &str = "面试时承诺的薪资与实际发放不一致，且要求无薪试岗两周。";

/// 通过 HTTP 发起投诉（走真实路由 /companies/{id}/complaints）
macro_rules! file_complaint {
    ($app:expr, $token:expr, $company:expr, $evidence:expr) => {{
        let uri = format!("/api/v1/companies/{}/complaints", $company);
        crate::tests::common::call!(
            $app,
            crate::tests::common::json_request(
                actix_web::http::Method::POST,
                &uri,
                Some($token),
                serde_json::json!({"evidence": $evidence}),
            )
        )
    }};
}

/// 通过 HTTP 审核投诉
macro_rules! review {
    ($app:expr, $token:expr, $id:expr, $body:expr) => {{
        let uri = format!("/api/v1/complaints/{}/review", $id);
        crate::tests::common::call!(
            $app,
            crate::tests::common::json_request(
                actix_web::http::Method::POST,
                &uri,
                Some($token),
                $body,
            )
        )
    }};
}

/// 审核场景：企业 A（有招聘者、职位较新）+ 企业 B（职位较旧）+ 求职者（已与 A 沟通）+ 管理员
struct Scene {
    seeker_id: Uuid,
    seeker_token: String,
    recruiter_token: String,
    admin_id: Uuid,
    admin_token: String,
    company_a: Uuid,
    company_b: Uuid,
}

async fn scene(pool: &PgPool) -> Scene {
    let (recruiter_id, company_a, recruiter_token) =
        common::recruiter(pool, "13900000001", "审核-企业A").await;
    common::seed_job(pool, &company_a, "职位A（较新）", common::days_ago(1)).await;

    let company_b = common::seed_company(pool, "审核-企业B", 0).await;
    common::seed_job(pool, &company_b, "职位B（较旧）", common::days_ago(30)).await;

    let (seeker_id, seeker_token) = common::seeker(pool, "13800000001").await;
    common::make_exchange(pool, &seeker_id, &recruiter_id).await;
    let (admin_id, admin_token) = common::admin(pool, "13700000001").await;

    Scene {
        seeker_id,
        seeker_token,
        recruiter_token,
        admin_id,
        admin_token,
        company_a,
        company_b,
    }
}

#[sqlx::test]
async fn complaint_requires_exchange_and_seeker_role(pool: PgPool) {
    let (seeker_id, seeker_token) = common::seeker(&pool, "13800000001").await;
    let (recruiter_id, company_id, recruiter_token) =
        common::recruiter(&pool, "13900000001", "被投诉企业").await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    // 1) 没有实际沟通记录 -> 403（这是平台的「必须先沟通过」规则）
    let (status, body) = file_complaint!(app, &seeker_token, company_id, EVIDENCE);
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // 2) 有过沟通，但证据不足 20 字 -> 400
    common::make_exchange(&pool, &seeker_id, &recruiter_id).await;
    let (status, body) = file_complaint!(app, &seeker_token, company_id, "太坑了");
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // 3) 招聘者不能发起投诉 -> 403
    let (status, body) = file_complaint!(app, &recruiter_token, company_id, EVIDENCE);
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // 4) 企业不存在 -> 404
    let (status, _) = file_complaint!(app, &seeker_token, Uuid::new_v4(), EVIDENCE);
    assert_eq!(status, StatusCode::NOT_FOUND);

    // 5) 正常发起 -> 201 pending，企业投诉次数此时仍为 0（审核通过才生效）
    let (status, created) = file_complaint!(app, &seeker_token, company_id, EVIDENCE);
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["status"], json!("pending"));
    assert_eq!(created["companyId"], json!(company_id.to_string()));
    assert_eq!(created["complainantId"], json!(seeker_id.to_string()));

    let (status, company) = call!(
        app,
        request(Method::GET, &format!("/api/v1/companies/{company_id}"), Some(&seeker_token))
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(company["complaintsCount"], json!(0), "待审核不计数");
}

#[sqlx::test]
async fn approve_increments_count_and_downranks_company(pool: PgPool) {
    let s = scene(&pool).await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    // 审核前：两家都是 0 次（优秀档），按发布时间倒序 -> A 在前
    let (_, list) = call!(
        app,
        request(Method::GET, "/api/v1/jobs", Some(&s.seeker_token))
    );
    let titles: Vec<&str> = list["items"].as_array().unwrap().iter().map(|j| j["title"].as_str().unwrap()).collect();
    assert_eq!(titles, vec!["职位A（较新）", "职位B（较旧）"]);

    // 求职者投诉企业 A
    let (status, created) = file_complaint!(app, &s.seeker_token, s.company_a, EVIDENCE);
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let complaint_id = created["id"].as_str().unwrap().to_string();

    // 审核权限：求职者与招聘者都无权审核
    for token in [&s.seeker_token, &s.recruiter_token] {
        let (status, body) = review!(app, token, complaint_id, json!({"approved": true}));
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    }

    // 管理员通过审核
    let (status, reviewed) = review!(
        app,
        &s.admin_token,
        complaint_id,
        json!({"approved": true, "note": "证据充分，属实"})
    );
    assert_eq!(status, StatusCode::OK, "{reviewed}");
    assert_eq!(reviewed["status"], json!("approved"));
    assert_eq!(reviewed["reviewedBy"], json!(s.admin_id.to_string()));
    assert_eq!(reviewed["reviewNote"], json!("证据充分，属实"));
    assert!(!reviewed["reviewedAt"].is_null());

    // 重复审核 -> 409
    let (status, body) = review!(app, &s.admin_token, complaint_id, json!({"approved": true}));
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    // 投诉次数 +1（企业详情可见）
    let (status, company) = call!(
        app,
        request(Method::GET, &format!("/api/v1/companies/{}", s.company_a), Some(&s.seeker_token))
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(company["complaintsCount"], json!(1), "审核通过应累计 1 次");

    // 职位列表：企业 A 降档（0 -> 1），排到企业 B 之后
    let (_, list) = call!(
        app,
        request(Method::GET, "/api/v1/jobs", Some(&s.seeker_token))
    );
    let items = list["items"].as_array().unwrap();
    let titles: Vec<&str> = items.iter().map(|j| j["title"].as_str().unwrap()).collect();
    let counts: Vec<i64> = items.iter().map(|j| j["complaintsCount"].as_i64().unwrap()).collect();
    assert_eq!(titles, vec!["职位B（较旧）", "职位A（较新）"], "被投诉企业在等级上降档");
    assert_eq!(counts, vec![0, 1]);
    assert_eq!(
        items[0]["companyId"],
        json!(s.company_b.to_string()),
        "降档后排在企业 B 之后"
    );
}

#[sqlx::test]
async fn reject_keeps_count_and_order(pool: PgPool) {
    let s = scene(&pool).await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    let (status, created) = file_complaint!(app, &s.seeker_token, s.company_a, EVIDENCE);
    assert_eq!(status, StatusCode::CREATED);
    let complaint_id = created["id"].as_str().unwrap().to_string();

    let (status, reviewed) = review!(
        app,
        &s.admin_token,
        complaint_id,
        json!({"approved": false, "note": "证据不足，无法核实"})
    );
    assert_eq!(status, StatusCode::OK, "{reviewed}");
    assert_eq!(reviewed["status"], json!("rejected"));

    let (_, company) = call!(
        app,
        request(Method::GET, &format!("/api/v1/companies/{}", s.company_a), Some(&s.seeker_token))
    );
    assert_eq!(company["complaintsCount"], json!(0), "驳回不累计投诉次数");

    let (_, list) = call!(
        app,
        request(Method::GET, "/api/v1/jobs", Some(&s.seeker_token))
    );
    let titles: Vec<&str> = list["items"].as_array().unwrap().iter().map(|j| j["title"].as_str().unwrap()).collect();
    assert_eq!(titles, vec!["职位A（较新）", "职位B（较旧）"], "驳回后顺序不变");
}

#[sqlx::test]
async fn review_validates_input(pool: PgPool) {
    let s = scene(&pool).await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    let (_, created) = file_complaint!(app, &s.seeker_token, s.company_a, EVIDENCE);
    let complaint_id = created["id"].as_str().unwrap().to_string();

    // 不存在的投诉 -> 404
    let (status, _) = review!(app, &s.admin_token, Uuid::new_v4(), json!({"approved": true}));
    assert_eq!(status, StatusCode::NOT_FOUND);

    // 备注超过 500 字 -> 400
    let long_note = "证".repeat(501);
    let (status, body) = review!(
        app,
        &s.admin_token,
        complaint_id,
        json!({"approved": true, "note": long_note})
    );
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // 未认证 -> 401
    let (status, _) = call!(
        app,
        json_request(
            Method::POST,
            &format!("/api/v1/complaints/{complaint_id}/review"),
            None,
            json!({"approved": true}),
        )
    );
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn complaint_scope_and_status_filter(pool: PgPool) {
    let s = scene(&pool).await;
    // 另一家企业的招聘者：不应看到企业 A 的投诉
    let (_, _, other_recruiter_token) = common::recruiter(&pool, "13900000099", "审核-企业C").await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    let (_, created) = file_complaint!(app, &s.seeker_token, s.company_a, EVIDENCE);
    let complaint_id = created["id"].as_str().unwrap().to_string();

    // 求职者：自己的投诉
    let (status, mine) = call!(
        app,
        request(Method::GET, "/api/v1/complaints/mine", Some(&s.seeker_token))
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mine.as_array().unwrap().len(), 1);
    assert_eq!(mine[0]["id"], json!(complaint_id));

    // 招聘者：仅本企业
    let (status, own) = call!(
        app,
        request(Method::GET, "/api/v1/complaints", Some(&s.recruiter_token))
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(own.as_array().unwrap().len(), 1);
    let (status, others) = call!(
        app,
        request(Method::GET, "/api/v1/complaints", Some(&other_recruiter_token))
    );
    assert_eq!(status, StatusCode::OK);
    assert!(others.as_array().unwrap().is_empty(), "不得看到其他企业的投诉");

    // 管理员：全部 + 状态过滤；非法状态 -> 400
    let (status, all) = call!(
        app,
        request(Method::GET, "/api/v1/complaints", Some(&s.admin_token))
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(all.as_array().unwrap().len(), 1);
    let (status, pending) = call!(
        app,
        request(
            Method::GET,
            "/api/v1/complaints?status=pending",
            Some(&s.admin_token)
        )
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pending.as_array().unwrap().len(), 1);
    let (status, approved) = call!(
        app,
        request(
            Method::GET,
            "/api/v1/complaints?status=approved",
            Some(&s.admin_token)
        )
    );
    assert_eq!(status, StatusCode::OK);
    assert!(approved.as_array().unwrap().is_empty());
    let (status, body) = call!(
        app,
        request(
            Method::GET,
            "/api/v1/complaints?status=bogus",
            Some(&s.admin_token)
        )
    );
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let _ = s.seeker_id;
}
