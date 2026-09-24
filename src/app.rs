//! App 装配：把中间件、共享状态、路由与 Swagger UI 组合成 actix App。
//! main.rs 只负责读取配置/初始化数据库/拉起服务，保持精简。

use actix_cors::Cors;
use actix_web::body::BoxBody;
use actix_web::dev::{ServiceFactory, ServiceRequest, ServiceResponse};
use actix_web::http::{header, Method};
use actix_web::web::{self, Data};
use actix_web::{App, Error};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::error::json_error_handler;
use crate::handlers;
use crate::openapi::ApiDoc;
use crate::state::AppState;

/// 根据配置生成 CORS 中间件：
/// - 未配置来源（默认）→ 不带跨域头，浏览器仅允许同源访问；
/// - 配置了来源 → 只允许列表内的来源（仍禁止携带凭据之外的其他来源）。
pub fn build_cors(origins: &[String]) -> Cors {
    let mut cors = Cors::default()
        .allowed_methods(vec![Method::GET, Method::POST, Method::PUT, Method::DELETE, Method::OPTIONS])
        .allowed_headers(vec![
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
        ])
        .max_age(3600);
    for origin in origins {
        cors = cors.allowed_origin(origin);
    }
    cors
}

/// 创建应用（每个 worker 都会调用一次）。请求日志 `TracingLogger` 与跨域
/// `CORS` 中间件由调用方（main）通过 `.wrap()` 挂载（它们会改写响应体类型，
/// 不便塞进这里的返回类型）。返回类型必须是具体 `App<T>`（T 隐藏），
/// 这样 `HttpServer` 才能通过 `IntoServiceFactory` 使用它。
pub fn create_app(
    state: Data<AppState>,
) -> App<
    impl ServiceFactory<
        ServiceRequest,
        Config = (),
        Response = ServiceResponse<BoxBody>,
        Error = Error,
        InitError = (),
    > + 'static,
> {
    App::new()
        // 共享状态与统一 JSON 校验错误（非法/超限 JSON -> 统一 400）
        .app_data(Data::clone(&state))
        .app_data(
            web::JsonConfig::default()
                .limit(1024 * 1024) // 请求体上限 1 MiB
                .error_handler(|err, req| json_error_handler(err.into(), req)),
        )
        // 业务路由
        .service(
            web::scope("/api/v1")
                .configure(handlers::users::configure)
                .configure(handlers::auth::configure)
                .configure(handlers::chat::configure)
                .configure(handlers::companies::configure)
                .configure(handlers::interviews::configure)
                .configure(handlers::push::configure)
                .configure(handlers::complaints::configure)
                .configure(handlers::stats::configure)
                // 审核专用账号管理（仅平台管理员）
                .configure(handlers::reviewers::configure)
                .configure(handlers::jobs::configure)
                .configure(handlers::resumes::configure)
                .configure(handlers::applications::configure)
                .route("/saved-jobs", web::get().to(handlers::jobs::saved_jobs)),
        )
        // WebSocket 实时通道（token 经 query 校验）
        .route("/ws", web::get().to(crate::ws::ws_upgrade))
        // 自动生成的 API 文档
        .service(
            SwaggerUi::new("/swagger-ui/{_:.*}").url("/api-docs/openapi.json", ApiDoc::openapi()),
        )
        // 系统端点
        .route("/", web::get().to(handlers::health::index))
        .route("/healthz", web::get().to(handlers::health::health_check))
        // 未匹配路由 -> 统一 404 JSON
        .default_service(web::route().to(handlers::not_found_route))
}
