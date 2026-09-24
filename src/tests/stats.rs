//! 平台统计测试：审核及时性口径、可见范围、用人单位优劣、求职用户分析。

use actix_web::http::{Method, StatusCode};
use actix_web::test;
use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::repositories::complaint as complaint_repo;
use crate::tests::common::{self, call, json_request, request};

const EVIDENCE: &str = "岗位描述与实际工作内容严重不符，且未按约定支付试用期薪资。";

async fn pending_complaint(pool: &PgPool, company: &Uuid, seeker: &Uuid) -> Uuid {
    complaint_repo::create(pool, company, seeker, EVIDENCE)
        .await
        .unwrap()
        .id
}

#[sqlx::test]
async fn review_stats_are_scoped_by_role(pool: PgPool) {
    let (_, admin_token) = common::admin(&pool, "13700000009").await;
    let (_, token_a) = common::reviewer_named(&pool, "13700000001", "审核员A").await;
    common::reviewer_named(&pool, "13700000002", "审核员B").await;
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    // 管理员：全部审核账号
    let (status, body) = call!(app, request(Method::GET, "/api/v1/stats/review", Some(&admin_token)));
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["reviewers"].as_array().unwrap().len(), 2, "管理员看全部审核");
    assert_eq!(body["slaSeconds"], json!(86400), "及时线 24h 由后端下发");

    // 审核账号：只有自己，看不到同事
    let (status, body) = call!(app, request(Method::GET, "/api/v1/stats/review", Some(&token_a)));
    assert_eq!(status, StatusCode::OK, "{body}");
    let reviewers = body["reviewers"].as_array().unwrap();
    assert_eq!(reviewers.len(), 1, "审核账号只能看到自己");
    assert_eq!(reviewers[0]["reviewerName"], json!("审核员A"));

    // 队列概览包含自己持有的锁数量
    assert!(body["queue"]["pendingTotal"].is_number());

    // 业务角色不可访问
    let (_, seeker_token) = common::seeker(&pool, "13800000001").await;
    let (status, _) = call!(app, request(Method::GET, "/api/v1/stats/review", Some(&seeker_token)));
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call!(app, request(Method::GET, "/api/v1/stats/review", None));
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn review_stats_measure_timeliness_only(pool: PgPool) {
    let (_, admin_token) = common::admin(&pool, "13700000009").await;
    let (_, reviewer_token) = common::reviewer_named(&pool, "13700000001", "审核员A").await;
    let (seeker_id, _) = common::seeker(&pool, "13800000001").await;
    let company = common::seed_company(&pool, "及时性用例企业", 0).await;

    // 一条 2 小时前提交（及时），一条 48 小时前提交（超时）
    let fast = pending_complaint(&pool, &company, &seeker_id).await;
    let slow = pending_complaint(&pool, &company, &seeker_id).await;
    sqlx::query("UPDATE complaints SET created_at = now() - interval '2 hours' WHERE id = $1")
        .bind(fast)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE complaints SET created_at = now() - interval '48 hours' WHERE id = $1")
        .bind(slow)
        .execute(&pool)
        .await
        .unwrap();

    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;
    for id in [fast, slow] {
        let (status, body) = call!(
            app,
            json_request(
                Method::POST,
                &format!("/api/v1/complaints/{id}/review"),
                Some(&reviewer_token),
                json!({ "approved": true, "note": "及时性用例" }),
            )
        );
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    let (status, body) = call!(app, request(Method::GET, "/api/v1/stats/review", Some(&admin_token)));
    assert_eq!(status, StatusCode::OK, "{body}");
    let me = &body["reviewers"][0];
    assert_eq!(me["reviewedTotal"], json!(2));
    assert_eq!(me["onTimeTotal"], json!(1), "只有 2 小时那条算及时");
    assert_eq!(me["onTimeRate"], json!(0.5));
    let avg = me["avgTotalSeconds"].as_f64().unwrap();
    assert!((89000.0..91000.0).contains(&avg), "平均总时长应约 25 小时，实际 {avg}");
    assert!(me["medianTotalSeconds"].as_f64().is_some());

    // 契约：统计里不含任何通过/驳回结论指标，避免用结论评价审核人员
    for key in ["approvedTotal", "rejectedTotal", "approvalRate"] {
        assert!(me.get(key).is_none(), "审核统计不应包含结论指标 {key}");
    }
}

#[sqlx::test]
async fn company_stats_are_admin_only_and_worst_first(pool: PgPool) {
    let (_, admin_token) = common::admin(&pool, "13700000009").await;
    let (_, reviewer_token) = common::reviewer(&pool, "13700000001").await;
    let (seeker_id, _) = common::seeker(&pool, "13800000001").await;

    let good = common::seed_company(&pool, "优质企业", 0).await;
    let bad = common::seed_company(&pool, "风险企业", 12).await;
    common::seed_job(&pool, &good, "优质岗位", Utc::now()).await;
    common::seed_job(&pool, &bad, "风险岗位", Utc::now()).await;

    // 优质企业收一条待审投诉（影响 qualityScore 但不影响等级）
    pending_complaint(&pool, &good, &seeker_id).await;

    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    // 权限：审核账号不可访问
    let (status, _) = call!(app, request(Method::GET, "/api/v1/stats/companies", Some(&reviewer_token)));
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, body) = call!(app, request(Method::GET, "/api/v1/stats/companies", Some(&admin_token)));
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = body.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["name"], json!("风险企业"), "最差的企业排在最前");
    assert_eq!(rows[0]["levelRank"], json!(4), "12 次已核实投诉 = 严重");
    assert_eq!(rows[1]["levelRank"], json!(0), "无已核实投诉 = 优秀");
    assert_eq!(rows[1]["complaintsPending"], json!(1));
    // 质量分公式：100 - 已核实*10 - 待核实*2
    assert_eq!(rows[1]["qualityScore"], json!(98));
    assert_eq!(rows[0]["qualityScore"], json!(0), "下限为 0");
}

#[sqlx::test]
async fn seeker_stats_are_admin_only(pool: PgPool) {
    let (_, admin_token) = common::admin(&pool, "13700000009").await;
    let (_, reviewer_token) = common::reviewer(&pool, "13700000001").await;
    let (seeker_id, _) = common::seeker(&pool, "13800000001").await;
    let _ = common::seeker(&pool, "13800000002").await;
    let (_, company, _) = common::recruiter(&pool, "13900000001", "求职统计企业").await;
    let job = common::seed_job(&pool, &company, "统计岗位", Utc::now()).await;

    // 一个求职者有简历 + 一次投递，另一个只有账号（用于参与度与分档）
    sqlx::query("INSERT INTO resumes (user_id, full_name, title) VALUES ($1, '张三', '后端工程师')")
        .bind(seeker_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO applications (job_id, seeker_id, status) VALUES ($1, $2, 'pending')")
        .bind(job)
        .bind(seeker_id)
        .execute(&pool)
        .await
        .unwrap();

    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    let (status, _) = call!(app, request(Method::GET, "/api/v1/stats/seekers", Some(&reviewer_token)));
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, body) = call!(app, request(Method::GET, "/api/v1/stats/seekers", Some(&admin_token)));
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["overview"]["total"], json!(2));
    assert_eq!(body["overview"]["withResume"], json!(1));
    assert_eq!(body["overview"]["resumesTotal"], json!(1));
    assert_eq!(body["overview"]["withApplication"], json!(1));
    assert_eq!(body["overview"]["applicationsTotal"], json!(1));
    assert_eq!(body["applicationsByStatus"][0]["status"], json!("pending"));
    // 分档：1 人 0 次投递、1 人 1-2 次
    let buckets = body["applicationBuckets"].as_array().unwrap();
    assert_eq!(buckets.len(), 2, "两个分档各 1 人: {buckets:?}");
    assert!(body["engagementRate"].as_f64().unwrap() > 0.0);
}
