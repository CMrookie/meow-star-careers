//! 企业端点：公开企业信息 / 招聘者我的企业。

use actix_web::{web, HttpResponse};
use uuid::Uuid;

use crate::auth::AuthenticatedUser;
use crate::error::{ApiResult, AppError, ErrorResponse};
use crate::models::company::Company;
use crate::repositories::{company as company_repo, user as user_repo};
use crate::state::AppState;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/companies")
            .route("/mine", web::get().to(my_company))
            .route("/{id}", web::get().to(get_company))
            .route("/{id}/complaints", web::post().to(crate::handlers::complaints::create_complaint))
            
    );
}

/// 我的企业（仅招聘者）
#[utoipa::path(
    get,
    path = "/companies/mine",
    tag = "company",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "当前招聘者所属企业", body = Company),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 403, description = "非招聘者/未绑定企业", body = ErrorResponse),
    )
)]
pub async fn my_company(
    state: web::Data<AppState>,
    auth: AuthenticatedUser,
) -> ApiResult<HttpResponse> {
    let recruiter = user_repo::require_recruiter(&state.pool, &auth.user_id).await?;
    let company_id = recruiter.company_id.expect("require_recruiter 已校验");
    let company = company_repo::get(&state.pool, &company_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("company `{company_id}`")))?;
    Ok(HttpResponse::Ok().json(company))
}

/// 企业公开信息
#[utoipa::path(
    get,
    path = "/companies/{id}",
    tag = "company",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path, description = "企业 ID")),
    responses(
        (status = 200, description = "企业信息", body = Company),
        (status = 401, description = "未认证", body = ErrorResponse),
        (status = 404, description = "企业不存在", body = ErrorResponse),
    )
)]
pub async fn get_company(
    state: web::Data<AppState>,
    _auth: AuthenticatedUser,
    path: web::Path<Uuid>,
) -> ApiResult<HttpResponse> {
    let id = path.into_inner();
    let company = company_repo::get(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("company `{id}`")))?;
    Ok(HttpResponse::Ok().json(company))
}

