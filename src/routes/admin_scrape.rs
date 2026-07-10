use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::auth::AuthCreator;
use crate::models::scrape_job::ScrapeJob;
use crate::models::scrape_result::ScrapeResult;
use crate::models::scrape_source::ScrapeSource;
use crate::routes::common::PaginatedResponse;
use crate::state::AppState;
use crate::worker::JobCommand;

// --- Request/Response Types ---

#[derive(Debug, Deserialize)]
pub struct ScrapeListParams {
    pub page: Option<i64>,
    pub limit: Option<i64>,
    pub platform: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSourceRequest {
    pub name: String,
    pub platform: String,
    pub source_url: String,
    pub scrape_config: Option<serde_json::Value>,
    pub scrape_interval_hours: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSourceRequest {
    pub name: Option<String>,
    pub platform: Option<String>,
    pub source_url: Option<String>,
    pub scrape_config: Option<serde_json::Value>,
    pub is_active: Option<bool>,
    pub scrape_interval_hours: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct SourceResponse {
    pub id: Uuid,
    pub name: String,
    pub platform: String,
    pub source_url: String,
    pub scrape_config: serde_json::Value,
    pub is_active: bool,
    pub scrape_interval_hours: i32,
    pub last_scraped_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct JobResponse {
    pub id: Uuid,
    pub source_id: Uuid,
    pub status: String,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
    pub items_found: i32,
    pub items_ingested: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct JobDetailResponse {
    pub id: Uuid,
    pub source_id: Uuid,
    pub source_name: String,
    pub status: String,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
    pub items_found: i32,
    pub items_ingested: i32,
    pub created_at: DateTime<Utc>,
    pub results: Vec<ScrapeResult>,
}

// --- Router ---

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/scrape-sources", get(list_sources).post(create_source))
        .route(
            "/scrape-sources/{id}",
            get(get_source).put(update_source).delete(delete_source),
        )
        .route("/scrape-sources/{id}/trigger", post(trigger_job))
        .route("/scrape-jobs", get(list_jobs))
        .route("/scrape-jobs/{id}", get(get_job))
}

// --- Source Handlers ---

async fn list_sources(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Query(params): Query<ScrapeListParams>,
) -> Result<(StatusCode, Json<PaginatedResponse<SourceResponse>>), AppError> {
    let pagination = crate::routes::common::PaginationParams {
        page: params.page,
        limit: params.limit,
    };
    let (page, limit, offset) = crate::routes::common::parse_pagination(&pagination);

    let sources: Vec<ScrapeSource> = sqlx::query_as(
        r#"
        SELECT * FROM scrape_sources
        WHERE creator_id = $1
          AND ($2::text IS NULL OR platform = $2)
        ORDER BY created_at DESC
        LIMIT $3 OFFSET $4
        "#,
    )
    .bind(creator_id)
    .bind(&params.platform)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    let total: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM scrape_sources
        WHERE creator_id = $1
          AND ($2::text IS NULL OR platform = $2)
        "#,
    )
    .bind(creator_id)
    .bind(&params.platform)
    .fetch_one(&state.db)
    .await?;

    let items: Vec<SourceResponse> = sources
        .into_iter()
        .map(|s| SourceResponse {
            id: s.id,
            name: s.name,
            platform: s.platform,
            source_url: s.source_url,
            scrape_config: s.scrape_config,
            is_active: s.is_active,
            scrape_interval_hours: s.scrape_interval_hours,
            last_scraped_at: s.last_scraped_at,
            created_at: s.created_at,
        })
        .collect();

    Ok((
        StatusCode::OK,
        Json(PaginatedResponse::new(items, total, page, limit)),
    ))
}

async fn create_source(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Json(payload): Json<CreateSourceRequest>,
) -> Result<(StatusCode, Json<SourceResponse>), AppError> {
    if payload.name.trim().is_empty() || payload.name.len() > 255 {
        return Err(AppError::Validation(
            "name is required and must be at most 255 chars".to_string(),
        ));
    }

    if payload.source_url.trim().is_empty() {
        return Err(AppError::Validation("source_url is required".to_string()));
    }

    let scrape_config = payload.scrape_config.unwrap_or_default();
    let scrape_interval_hours = payload.scrape_interval_hours.unwrap_or(24).clamp(1, 720);

    let source: ScrapeSource = sqlx::query_as(
        r#"
        INSERT INTO scrape_sources (creator_id, name, platform, source_url, scrape_config, scrape_interval_hours)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING *
        "#,
    )
    .bind(creator_id)
    .bind(&payload.name)
    .bind(&payload.platform)
    .bind(&payload.source_url)
    .bind(&scrape_config)
    .bind(scrape_interval_hours)
    .fetch_one(&state.db)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(SourceResponse {
            id: source.id,
            name: source.name,
            platform: source.platform,
            source_url: source.source_url,
            scrape_config: source.scrape_config,
            is_active: source.is_active,
            scrape_interval_hours: source.scrape_interval_hours,
            last_scraped_at: source.last_scraped_at,
            created_at: source.created_at,
        }),
    ))
}

async fn get_source(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(source_id): Path<Uuid>,
) -> Result<Json<SourceResponse>, AppError> {
    let source: Option<ScrapeSource> =
        sqlx::query_as("SELECT * FROM scrape_sources WHERE id = $1")
            .bind(source_id)
            .fetch_optional(&state.db)
            .await?;

    let source = source.ok_or_else(|| AppError::NotFound("Source not found".to_string()))?;

    if source.creator_id != creator_id {
        return Err(AppError::Forbidden);
    }

    Ok(Json(SourceResponse {
        id: source.id,
        name: source.name,
        platform: source.platform,
        source_url: source.source_url,
        scrape_config: source.scrape_config,
        is_active: source.is_active,
        scrape_interval_hours: source.scrape_interval_hours,
        last_scraped_at: source.last_scraped_at,
        created_at: source.created_at,
    }))
}

async fn update_source(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(source_id): Path<Uuid>,
    Json(payload): Json<UpdateSourceRequest>,
) -> Result<Json<SourceResponse>, AppError> {
    let existing: Option<ScrapeSource> =
        sqlx::query_as("SELECT * FROM scrape_sources WHERE id = $1")
            .bind(source_id)
            .fetch_optional(&state.db)
            .await?;

    let existing = existing.ok_or_else(|| AppError::NotFound("Source not found".to_string()))?;

    if existing.creator_id != creator_id {
        return Err(AppError::Forbidden);
    }

    let name = payload.name.unwrap_or(existing.name);
    let platform = payload.platform.unwrap_or(existing.platform);
    let source_url = payload.source_url.unwrap_or(existing.source_url);
    let scrape_config = payload.scrape_config.unwrap_or(existing.scrape_config);
    let is_active = payload.is_active.unwrap_or(existing.is_active);
    let scrape_interval_hours = payload
        .scrape_interval_hours
        .unwrap_or(existing.scrape_interval_hours)
        .clamp(1, 720);

    let updated: ScrapeSource = sqlx::query_as(
        r#"
        UPDATE scrape_sources
        SET name = $1, platform = $2, source_url = $3, scrape_config = $4,
            is_active = $5, scrape_interval_hours = $6
        WHERE id = $7
        RETURNING *
        "#,
    )
    .bind(&name)
    .bind(&platform)
    .bind(&source_url)
    .bind(&scrape_config)
    .bind(is_active)
    .bind(scrape_interval_hours)
    .bind(source_id)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(SourceResponse {
        id: updated.id,
        name: updated.name,
        platform: updated.platform,
        source_url: updated.source_url,
        scrape_config: updated.scrape_config,
        is_active: updated.is_active,
        scrape_interval_hours: updated.scrape_interval_hours,
        last_scraped_at: updated.last_scraped_at,
        created_at: updated.created_at,
    }))
}

async fn delete_source(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(source_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let existing: Option<ScrapeSource> =
        sqlx::query_as("SELECT * FROM scrape_sources WHERE id = $1")
            .bind(source_id)
            .fetch_optional(&state.db)
            .await?;

    let existing = existing.ok_or_else(|| AppError::NotFound("Source not found".to_string()))?;

    if existing.creator_id != creator_id {
        return Err(AppError::Forbidden);
    }

    sqlx::query("UPDATE scrape_sources SET is_active = false WHERE id = $1")
        .bind(source_id)
        .execute(&state.db)
        .await?;

    Ok(Json(serde_json::json!({ "message": "Source deactivated" })))
}

// --- Job Handlers ---

async fn trigger_job(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(source_id): Path<Uuid>,
) -> Result<(StatusCode, Json<JobResponse>), AppError> {
    let source: Option<ScrapeSource> =
        sqlx::query_as("SELECT * FROM scrape_sources WHERE id = $1")
            .bind(source_id)
            .fetch_optional(&state.db)
            .await?;

    let source = source.ok_or_else(|| AppError::NotFound("Source not found".to_string()))?;

    if source.creator_id != creator_id {
        return Err(AppError::Forbidden);
    }

    // Create job record
    let job: ScrapeJob = sqlx::query_as(
        r#"
        INSERT INTO scrape_jobs (source_id, creator_id, status)
        VALUES ($1, $2, 'pending')
        RETURNING *
        "#,
    )
    .bind(source_id)
    .bind(creator_id)
    .fetch_one(&state.db)
    .await?;

    // Send job to worker if available
    if let Some(ref sender) = state.job_sender {
        if let Err(e) = sender.send(JobCommand::RunJob(job.id)).await {
            tracing::error!("Failed to send job to worker: {}", e);
            // Update job status to failed
            sqlx::query(
                "UPDATE scrape_jobs SET status = 'failed', error_message = $1 WHERE id = $2",
            )
            .bind(format!("Failed to queue job: {}", e))
            .bind(job.id)
            .execute(&state.db)
            .await?;
        }
    } else {
        tracing::warn!("No job worker available, job {} will not be processed", job.id);
    }

    Ok((
        StatusCode::CREATED,
        Json(JobResponse {
            id: job.id,
            source_id: job.source_id,
            status: job.status,
            started_at: job.started_at,
            completed_at: job.completed_at,
            error_message: job.error_message,
            items_found: job.items_found,
            items_ingested: job.items_ingested,
            created_at: job.created_at,
        }),
    ))
}

async fn list_jobs(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Query(params): Query<ScrapeListParams>,
) -> Result<(StatusCode, Json<PaginatedResponse<JobResponse>>), AppError> {
    let pagination = crate::routes::common::PaginationParams {
        page: params.page,
        limit: params.limit,
    };
    let (page, limit, offset) = crate::routes::common::parse_pagination(&pagination);

    let jobs: Vec<ScrapeJob> = sqlx::query_as(
        r#"
        SELECT * FROM scrape_jobs
        WHERE creator_id = $1
          AND ($2::text IS NULL OR status = $2)
        ORDER BY created_at DESC
        LIMIT $3 OFFSET $4
        "#,
    )
    .bind(creator_id)
    .bind(&params.status)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;

    let total: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM scrape_jobs
        WHERE creator_id = $1
          AND ($2::text IS NULL OR status = $2)
        "#,
    )
    .bind(creator_id)
    .bind(&params.status)
    .fetch_one(&state.db)
    .await?;

    let items: Vec<JobResponse> = jobs
        .into_iter()
        .map(|j| JobResponse {
            id: j.id,
            source_id: j.source_id,
            status: j.status,
            started_at: j.started_at,
            completed_at: j.completed_at,
            error_message: j.error_message,
            items_found: j.items_found,
            items_ingested: j.items_ingested,
            created_at: j.created_at,
        })
        .collect();

    Ok((
        StatusCode::OK,
        Json(PaginatedResponse::new(items, total, page, limit)),
    ))
}

async fn get_job(
    State(state): State<AppState>,
    AuthCreator(creator_id): AuthCreator,
    Path(job_id): Path<Uuid>,
) -> Result<Json<JobDetailResponse>, AppError> {
    let job: Option<ScrapeJob> = sqlx::query_as("SELECT * FROM scrape_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_optional(&state.db)
        .await?;

    let job = job.ok_or_else(|| AppError::NotFound("Job not found".to_string()))?;

    if job.creator_id != creator_id {
        return Err(AppError::Forbidden);
    }

    // Fetch source name
    let source: Option<ScrapeSource> =
        sqlx::query_as("SELECT * FROM scrape_sources WHERE id = $1")
            .bind(job.source_id)
            .fetch_optional(&state.db)
            .await?;

    let source_name = source
        .map(|s| s.name)
        .unwrap_or_else(|| "Unknown".to_string());

    // Fetch results
    let results: Vec<ScrapeResult> =
        sqlx::query_as("SELECT * FROM scrape_results WHERE job_id = $1 ORDER BY created_at")
            .bind(job_id)
            .fetch_all(&state.db)
            .await?;

    Ok(Json(JobDetailResponse {
        id: job.id,
        source_id: job.source_id,
        source_name,
        status: job.status,
        started_at: job.started_at,
        completed_at: job.completed_at,
        error_message: job.error_message,
        items_found: job.items_found,
        items_ingested: job.items_ingested,
        created_at: job.created_at,
        results,
    }))
}
