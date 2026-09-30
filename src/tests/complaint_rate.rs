//! 投诉定级 v2（投诉率 × 公司规模）测试 —— 与求职 App `lib/ui/complaint_level.dart` 同一口径。
//!
//! 规则（改一处必须两处同改）：
//! - 分母 `staff_size` 由企业申报；只计**管理员审核通过**的投诉次数；
//! - 0 次 → 优秀；
//! - 规模 >= 50 人 → 每百人投诉率：≤0.5% 轻微 / ≤1.5% 预警 / ≤3.0% 警告 / >3.0% 严重；
//! - 规模未申报或 < 50 人 → 退回次数口径：1-2 轻微 / 3-5 预警 / 6-9 警告 / ≥10 严重。
//!
//! 覆盖：SQL 函数边界（与服务端排序同源）、排序呈现（大公司不再被次数冤枉）、
//! 规模缺失/过小的兜底、字段透出（JobView.companyStaffSize / Company.staffSize）、注册校验。

use actix_web::http::{Method, StatusCode};
use actix_web::test;
use serde_json::json;
use sqlx::PgPool;

use crate::tests::common::{self, call, json_request, request, PASSWORD};

/// 直接问数据库要等级序（与服务端 ORDER BY 用的是同一个函数）
async fn rank(pool: &PgPool, complaints: i32, staff_size: Option<i32>) -> i16 {
    sqlx::query_scalar::<_, i16>("SELECT complaint_level_rank($1::int, $2::int)")
        .bind(complaints)
        .bind(staff_size)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// 直接问数据库要每百人投诉率（%；规模不可用时为 NULL）
async fn rate(pool: &PgPool, complaints: i32, staff_size: Option<i32>) -> Option<f64> {
    sqlx::query_scalar::<_, Option<f64>>(
        "SELECT complaint_rate_percent($1::int, $2::int)::float8",
    )
    .bind(complaints)
    .bind(staff_size)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test]
async fn grading_boundaries_match_client_rule(pool: PgPool) {
    // (投诉次数, 公司规模, 期望等级序 0..4)
    let cases: &[(i32, Option<i32>, i16)] = &[
        // 优秀：0 次（有/无规模都一样）
        (0, Some(2000), 0),
        (0, None, 0),
        // 按率：2000 人 → 0.5% = 10 次、1.5% = 30 次、3% = 60 次
        (1, Some(2000), 1),    // 0.05% 轻微
        (10, Some(2000), 1),   // 0.50% 边界 -> 轻微
        (11, Some(2000), 2),   // 0.55% -> 预警
        (20, Some(2000), 2),   // 1.00% -> 预警
        (30, Some(2000), 2),   // 1.50% 边界 -> 预警
        (31, Some(2000), 3),   // 1.55% -> 警告
        (60, Some(2000), 3),   // 3.00% 边界 -> 警告
        (61, Some(2000), 4),   // 3.05% -> 严重
        // 规模越大越宽容：同样 3 次投诉
        (3, Some(5000), 1),    // 0.06% 轻微
        (3, Some(500), 2),     // 0.60% -> 预警
        (3, Some(60), 4),      // 5.00% 严重
        (20, Some(60), 4),     // 33.3% 严重
        // 小样本（< 50 人）退回次数口径
        (1, Some(49), 1),      // 次数 1 -> 轻微
        (2, Some(49), 1),      // 次数 2 -> 轻微
        (3, Some(49), 2),      // 次数 3 -> 预警
        (9, Some(49), 3),      // 次数 9 -> 警告
        (10, Some(49), 4),     // 次数 10 -> 严重
        (50, Some(49), 4),     // 率高但不采纳
        // 未申报规模：完全按次数
        (1, None, 1),
        (2, None, 1),
        (3, None, 2),
        (6, None, 3),
        (10, None, 4),
        (99, None, 4),
    ];

    for (complaints, staff_size, expected) in cases {
        let actual = rank(&pool, *complaints, *staff_size).await;
        assert_eq!(
            actual, *expected,
            "complaint_level_rank({complaints}, {staff_size:?}) 期望 {expected} 实际 {actual}"
        );
    }

    // 上表第 12 行：3 次 / 500 人 = 0.6% 应为「预警」而不是「轻微」
    assert_eq!(rank(&pool, 3, Some(500)).await, 2);
}

#[sqlx::test]
async fn rate_percent_is_null_without_usable_size(pool: PgPool) {
    assert_eq!(rate(&pool, 20, Some(2000)).await, Some(1.0));
    assert_eq!(rate(&pool, 3, Some(60)).await, Some(5.0));
    assert_eq!(rate(&pool, 10, Some(2000)).await, Some(0.5));
    // 未申报 / 规模过小：不按率（NULL），调用方应退回次数口径
    assert_eq!(rate(&pool, 1, None).await, None);
    assert_eq!(rate(&pool, 5, Some(49)).await, None);
    assert_eq!(rate(&pool, 0, Some(0)).await, None);
}

#[sqlx::test]
async fn large_company_is_judged_gentler_than_small_one(pool: PgPool) {
    // 就是这次修正要解决的问题：20 起投诉在 2000 人公司（1.0% 预警）
    // 不该和 60 人公司的 3 起投诉（5% 严重）同等看待；
    // 而 5000 人公司的 3 起投诉（0.06%）应该是最轻的「轻微」。
    let huge = common::seed_company_sized(&pool, "超大规模-3次", 3, Some(5000)).await;
    let big = common::seed_company_sized(&pool, "大公司-20次", 20, Some(2000)).await;
    let small = common::seed_company_sized(&pool, "小公司-3次", 3, Some(60)).await;
    common::seed_job(&pool, &huge, "超大规模职位", common::days_ago(1)).await;
    common::seed_job(&pool, &big, "大公司职位", common::days_ago(2)).await;
    common::seed_job(&pool, &small, "小公司职位", common::days_ago(3)).await;

    let (_, token) = common::seeker(&pool, "13800000001").await;
    let app = test::init_service(crate::app::create_app(common::state(pool))).await;

    let (status, body) = call!(app, request(Method::GET, "/api/v1/jobs", Some(&token)));
    assert_eq!(status, StatusCode::OK);
    let titles: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["title"].as_str().unwrap())
        .collect();
    let counts: Vec<i64> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["complaintsCount"].as_i64().unwrap())
        .collect();
    let sizes: Vec<Option<i64>> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["companyStaffSize"].as_i64())
        .collect();

    assert_eq!(
        titles,
        vec!["超大规模职位", "大公司职位", "小公司职位"],
        "按率定级：大公司的 20 次投诉应排在小公司 3 次之前",
    );
    assert_eq!(counts, vec![3, 20, 3], "投诉次数本身不再是唯一排序依据");
    assert_eq!(sizes, vec![Some(5000), Some(2000), Some(60)]);
}

#[sqlx::test]
async fn unknown_or_small_size_falls_back_to_count_basis(pool: PgPool) {
    // 未申报规模：按次数（4 次 -> 预警）；规模 20 人的 20 次投诉也应落到「严重」（次数口径）
    let unknown = common::seed_company_sized(&pool, "未申报规模-4次", 4, None).await;
    let tiny = common::seed_company_sized(&pool, "微型企业-20次", 20, Some(20)).await;
    let rated = common::seed_company_sized(&pool, "有规模-20次", 20, Some(2000)).await;
    common::seed_job(&pool, &unknown, "未申报规模职位", common::days_ago(5)).await;
    common::seed_job(&pool, &tiny, "微型企业职位", common::days_ago(5)).await;
    common::seed_job(&pool, &rated, "有规模职位", common::days_ago(5)).await;

    let (_, token) = common::seeker(&pool, "13800000001").await;
    let app = test::init_service(crate::app::create_app(common::state(pool))).await;

    let (_, body) = call!(app, request(Method::GET, "/api/v1/jobs", Some(&token)));
    let titles: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["title"].as_str().unwrap())
        .collect();

    // 有规模折算的「大公司 20 次」（预警，1.0%）排在无规模折算的「未申报规模 4 次」（预警，次数口径）之前；
    // 微型企业 20 次按次数口径是「严重」，沉到最后。
    assert_eq!(
        titles,
        vec!["有规模职位", "未申报规模职位", "微型企业职位"],
        "同等级内有规模折算的优先；规模过小退回次数口径"
    );
}

#[sqlx::test]
async fn staff_size_is_exposed_on_job_and_company(pool: PgPool) {
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    // 招聘者注册时申报规模（求职 App 的 `company.staffSize`）
    let (status, reg) = call!(
        app,
        json_request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            json!({
                "phone": "13900000001", "name": "HR", "password": PASSWORD, "role": "recruiter",
                "company": {"name": "规模透出企业", "industry": "互联网", "location": "北京", "staffSize": 2000}
            }),
        )
    );
    assert_eq!(status, StatusCode::CREATED, "{reg}");
    let token = reg["token"].as_str().unwrap().to_string();
    let company_id = reg["user"]["companyId"].as_str().unwrap().to_string();

    let (status, job) = call!(
        app,
        json_request(
            Method::POST,
            "/api/v1/jobs",
            Some(&token),
            json!({"title": "规模透出职位", "description": "测试描述"}),
        )
    );
    assert_eq!(status, StatusCode::CREATED, "{job}");
    // 职位视图必须带出分母（客户端 `JobView.companyStaffSize`）
    assert_eq!(job["companyStaffSize"], json!(2000));

    let (_, list) = call!(app, request(Method::GET, "/api/v1/jobs", Some(&token)));
    assert_eq!(list["items"][0]["companyStaffSize"], json!(2000));

    // 企业视图带出规模（客户端 `Company.staffSize`）
    let (status, company) = call!(
        app,
        request(Method::GET, &format!("/api/v1/companies/{company_id}"), Some(&token))
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(company["staffSize"], json!(2000));
    let (_, mine) = call!(app, request(Method::GET, "/api/v1/companies/mine", Some(&token)));
    assert_eq!(mine["staffSize"], json!(2000));

    // 未申报规模的另一家公司：字段为 null（客户端会优雅退回次数口径）
    let (status, reg2) = call!(
        app,
        json_request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            json!({
                "phone": "13900000002", "name": "HR2", "password": PASSWORD, "role": "recruiter",
                "company": {"name": "未申报规模企业"}
            }),
        )
    );
    assert_eq!(status, StatusCode::CREATED, "{reg2}");
    assert!(reg2["user"]["companyId"].is_string());
    let token2 = reg2["token"].as_str().unwrap().to_string();
    let (_, list2) = call!(
        app,
        json_request(
            Method::POST,
            "/api/v1/jobs",
            Some(&token2),
            json!({"title": "未申报规模职位", "description": "测试描述"}),
        )
    );
    let _ = list2;
    let (_, list2) = call!(app, request(Method::GET, "/api/v1/jobs/my", Some(&token2)));
    assert!(
        list2["items"][0]["companyStaffSize"].is_null(),
        "未申报规模应为 null，而不是 0：{list2}"
    );
}

#[sqlx::test]
async fn staff_size_validation_on_recruiter_registration(pool: PgPool) {
    let app = test::init_service(crate::app::create_app(common::state(pool.clone()))).await;

    for (phone, staff_size) in [("13900000001", -1), ("13900000002", 20_000_000)] {
        let (status, body) = call!(
            app,
            json_request(
                Method::POST,
                "/api/v1/auth/register",
                None,
                json!({
                    "phone": phone, "name": "HR", "password": PASSWORD, "role": "recruiter",
                    "company": {"name": "非法规模企业", "staffSize": staff_size}
                }),
            )
        );
        assert_eq!(status, StatusCode::BAD_REQUEST, "staffSize={staff_size} 应被拒: {body}");
    }

    // 0 与上限内的值合法（0 表示规模极小/未知，定级会自动退回次数口径）
    let (status, body) = call!(
        app,
        json_request(
            Method::POST,
            "/api/v1/auth/register",
            None,
            json!({
                "phone": "13900000003", "name": "HR", "password": PASSWORD, "role": "recruiter",
                "company": {"name": "零规模企业", "staffSize": 0}
            }),
        )
    );
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(rank(&pool, 1, Some(0)).await, 1, "规模 0 走次数口径：1 次 -> 轻微");
}
