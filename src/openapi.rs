//! OpenAPI 文档装配：把各处理器上 `#[utoipa::path]` 生成的路径与 schema 组件
//! 汇总为一个 `ApiDoc`，由 Swagger UI 在 /swagger-ui/ 提供交互式文档。
//!
//! 注意：`paths(...)` 必须使用「模块全路径」，因为 `#[utoipa::path]` 会在
//! 处理器所在模块内生成隐藏的 `__path_<fn>` 项，仅通过 `use` 引入函数名无法解析。

use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};
use utoipa::{Modify, OpenApi};

use crate::error::ErrorResponse;
use crate::models::application::{
    ApplicationPage, ApplicationView, ApplyRequest, UpdateApplicationStatus,
};
use crate::models::auth::{AuthResponse, LoginRequest, RegisterRequest, Role};
use crate::models::chat::{ConversationSummary, Message, NewMessage, StartConversationRequest};
use crate::models::complaint::{ComplaintView, CreateComplaint, ReviewComplaint};
use crate::models::company::{Company, NewCompany};
use crate::models::interview::{InterviewView, NewInterview};
use crate::models::job::{JobPage, JobView, NewJob, UpdateJob};
use crate::models::resume::{Resume, ResumePage, ResumeWrite};
use crate::models::user::{NewUser, UpdateUser, User, UserPublic};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "招聘求职系统 API",
        description = "基于 actix-web + PostgreSQL 的招聘求职平台服务端：注册登录（Bearer Token）、职位发布与搜索、简历管理、在线投递与状态流转、职位收藏；另含一对一私聊（REST + WebSocket）。统一错误处理、结构化日志、自动生成 API 文档。",
        version = "0.3.0",
    ),
    paths(
        crate::handlers::health::index,
        crate::handlers::health::health_check,
        // 认证
        crate::handlers::auth::register,
        crate::handlers::auth::login,
        crate::handlers::auth::logout,
        crate::handlers::auth::me,
        // 用户
        crate::handlers::users::list_users,
        crate::handlers::users::create_user,
        crate::handlers::users::get_user,
        crate::handlers::users::update_user,
        crate::handlers::users::delete_user,
        // 企业
        crate::handlers::companies::my_company,
        crate::handlers::companies::get_company,
        // 职位与投递/收藏
        crate::handlers::jobs::list_jobs,
        crate::handlers::jobs::create_job,
        crate::handlers::jobs::my_jobs,
        crate::handlers::jobs::get_job,
        crate::handlers::jobs::update_job,
        crate::handlers::jobs::delete_job,
        crate::handlers::jobs::apply_job,
        crate::handlers::jobs::set_job_active,
        crate::handlers::jobs::save_job,
        crate::handlers::jobs::unsave_job,
        crate::handlers::jobs::saved_jobs,
        crate::handlers::applications::list_applications,
        crate::handlers::applications::get_application,
        crate::handlers::applications::update_application_status,
        // 简历
        crate::handlers::resumes::create_resume,
        crate::handlers::resumes::my_resumes,
        crate::handlers::resumes::search_resumes,
        crate::handlers::resumes::get_resume,
        crate::handlers::resumes::update_resume,
        crate::handlers::resumes::delete_resume,
        // 私聊
        crate::handlers::chat::start_conversation,
        crate::handlers::chat::list_conversations,
        crate::handlers::chat::get_conversation,
        crate::handlers::chat::list_messages,
        crate::handlers::chat::send_message,
        crate::handlers::chat::mark_read,
        // 线上面试
        crate::handlers::interviews::create_interview,
        crate::handlers::interviews::list_mine,
        crate::handlers::interviews::get_interview,
        crate::handlers::interviews::start_interview,
        crate::handlers::interviews::finish_interview,
        crate::handlers::interviews::cancel_interview,
        // 投诉（举报）与管理员审核
        crate::handlers::complaints::create_complaint,
        crate::handlers::complaints::my_complaints,
        crate::handlers::complaints::list_complaints,
        crate::handlers::complaints::review_complaint,
    ),
    components(
        schemas(
            User,
            UserPublic,
            NewUser,
            UpdateUser,
            Role,
            RegisterRequest,
            LoginRequest,
            AuthResponse,
            Company,
            NewCompany,
            JobView,
            JobPage,
            NewJob,
            UpdateJob,
            Resume,
            ResumePage,
            ResumeWrite,
            ApplicationView,
            ApplicationPage,
            ApplyRequest,
            UpdateApplicationStatus,
            ConversationSummary,
            InterviewView,
            NewInterview,
            Message,
            StartConversationRequest,
            NewMessage,
            ComplaintView,
            CreateComplaint,
            ReviewComplaint,
            ErrorResponse,
        ),
    ),
    tags(
        (name = "system", description = "系统 / 健康检查"),
        (name = "auth", description = "注册 / 登录 / 令牌"),
        (name = "users", description = "用户管理（仅平台管理员；本人可查改自己）"),
        (name = "company", description = "企业信息"),
        (name = "job", description = "职位发布 / 搜索 / 收藏"),
        (name = "application", description = "在线投递与状态流转"),
        (name = "resume", description = "简历管理（求职者）/ 简历检索（招聘者）"),
        (name = "chat", description = "一对一私聊（REST；实时通道见 /ws 与 README）"),
        (name = "interview", description = "线上视频面试（房间/状态/信令经 WS 转发）"),
        (name = "complaint", description = "投诉（举报）与平台管理员审核"),
    ),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;

/// 为文档注册 Bearer Token 安全方案（登录/注册返回的 token）
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearerAuth",
                SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
            );
        }
    }
}
