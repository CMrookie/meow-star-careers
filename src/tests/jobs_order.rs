//! 职位列表排序测试：一律按用人单位投诉等级「从优到劣」，且分页全局有序。
//!
//! 等级阈值：0 优秀 / 1-2 轻微 / 3-5 预警 / 6-9 警告 / >=10 严重
//! （complaint_level_rank，见 migrations/0013_complaint_level_order.sql）

use std::time::Duration;

use actix_web::http::{Method, StatusCode};
use actix_web::test;
use serde_json::json;
use sqlx::PgPool;

use crate::tests::common::{self, call, json_request, request};

#[sqlx::test]
async fn jobs_list_is_globally_sorted_by_complaint_level(pool: PgPool) {
    // 六家企业覆盖五档 + 同级内次数差异；同级时发布时间新的在前
    let zero_old = common::seed_company(&pool, "排序-0次-旧", 0).await;
    let zero_new = common::seed_company(&pool, "排序-0次-新", 0).await;
    let minor_1 = common::seed_company(&pool, "排序-1次", 1).await;
    let minor_2 = common::seed_company(&pool, "排序-2次", 2).await;
    let alert = common::seed_company(&pool, "排序-3次", 3).await;
    let warning = common::seed_company(&pool, "排序-7次", 7).await;
    let severe = common::seed_company(&pool, "排序-12次", 12).await;

    common::seed_job(&pool, &zero_old, "0次-旧", common::days_ago(30)).await;
    common::seed_job(&pool, &zero_new, "0次-新", common::days_ago(1)).await;
    common::seed_job(&pool, &minor_1, "1次", common::days_ago(20)).await;
    common::seed_job(&pool, &minor_2, "2次", common::days_ago(10)).await;
    common::seed_job(&pool, &alert, "3次", common::days_ago(25)).await;
    common::seed_job(&pool, &warning, "7次", common::days_ago(5)).await;
    common::seed_job(&pool, &severe, "12次", common::days_ago(2)).await;

    let (_, token) = common::seeker(&pool, "13800000001").await;
    let app = test::init_service(crate::app::create_app(common::state(pool))).await;

    // pageSize=2 连翻四页：跨页拼接后必须仍全局有序（服务端排序 + id 兜底保证稳定分页）
    let mut titles: Vec<String> = Vec::new();
    let mut counts: Vec<i64> = Vec::new();
    for page in 1..=4 {
        let uri = format!("/api/v1/jobs?pageSize=2&page={page}");
        let (status, body) = call!(app, request(Method::GET, &uri, Some(&token)));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["total"], json!(7));
        for item in body["items"].as_array().unwrap() {
            titles.push(item["title"].as_str().unwrap().to_string());
            counts.push(item["complaintsCount"].as_i64().unwrap());
        }
    }

    assert!(
        counts.windows(2).all(|pair| pair[0] <= pair[1]),
        "跨页必须按等级从优到劣（投诉次数单调不减）：{counts:?}"
    );
    assert_eq!(
        titles,
        vec!["0次-新", "0次-旧", "1次", "2次", "3次", "7次", "12次"],
        "同级内次数少者优先，再按发布时间倒序"
    );
}

#[sqlx::test]
async fn saved_jobs_follow_level_not_save_time(pool: PgPool) {
    let best = common::seed_company(&pool, "收藏-优秀", 0).await;
    let worst = common::seed_company(&pool, "收藏-严重", 15).await;
    let best_job = common::seed_job(&pool, &best, "收藏-优秀职位", common::days_ago(10)).await;
    let worst_job = common::seed_job(&pool, &worst, "收藏-严重职位", common::days_ago(1)).await;

    let (_, token) = common::seeker(&pool, "13800000001").await;
    let app = test::init_service(crate::app::create_app(common::state(pool))).await;

    // 先收藏「优秀」，隔 1 秒再收藏「严重」：若按收藏时间排则严重在前，按等级排则优秀在前
    let (status, _) = call!(
        app,
        request(Method::POST, &format!("/api/v1/jobs/{best_job}/save"), Some(&token))
    );
    assert_eq!(status, StatusCode::NO_CONTENT);
    std::thread::sleep(Duration::from_millis(1100));
    let (status, _) = call!(
        app,
        request(Method::POST, &format!("/api/v1/jobs/{worst_job}/save"), Some(&token))
    );
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = call!(
        app,
        request(Method::GET, "/api/v1/saved-jobs", Some(&token))
    );
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    let titles: Vec<&str> = items.iter().map(|j| j["title"].as_str().unwrap()).collect();
    assert_eq!(titles, vec!["收藏-优秀职位", "收藏-严重职位"], "收藏列表按等级而非收藏时间");
}

#[sqlx::test]
async fn recruiter_my_jobs_ordered_by_recency(pool: PgPool) {
    let (_, _, token) = common::recruiter(&pool, "13900000001", "我的职位企业").await;
    let app = test::init_service(crate::app::create_app(common::state(pool))).await;

    for title in ["我的职位-旧", "我的职位-新"] {
        let (status, body) = call!(
            app,
            json_request(
                Method::POST,
                "/api/v1/jobs",
                Some(&token),
                json!({"title": title, "description": "测试职位描述"}),
            )
        );
        assert_eq!(status, StatusCode::CREATED, "{body}");
        std::thread::sleep(Duration::from_millis(1100));
    }

    let (status, body) = call!(
        app,
        request(Method::GET, "/api/v1/jobs/my", Some(&token))
    );
    assert_eq!(status, StatusCode::OK);
    let titles: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, vec!["我的职位-新", "我的职位-旧"], "本企业等级相同 -> 按发布时间倒序");
}
