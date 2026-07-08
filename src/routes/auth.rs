use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::jwt;
use crate::models::creator::Creator;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    pub display_name: String,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginCreator {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub creator: LoginCreator,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/google", post(google_auth))
        .route("/github", post(github_auth))
}

pub async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<RegisterResponse>), AppError> {
    if payload.email.trim().is_empty() || !payload.email.contains('@') {
        return Err(AppError::Validation("A valid email is required".to_string()));
    }
    if payload.password.len() < 8 {
        return Err(AppError::Validation(
            "Password must be at least 8 characters".to_string(),
        ));
    }
    if payload.display_name.trim().is_empty() || payload.display_name.len() > 100 {
        return Err(AppError::Validation(
            "display_name is required and must be 1-100 chars".to_string(),
        ));
    }

    // Hash password BEFORE DB query (parallel optimization)
    let password_hash = bcrypt::hash(&payload.password, bcrypt::DEFAULT_COST)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    // Single roundtrip: INSERT + detect conflict
    let creator: Option<Creator> = sqlx::query_as(
        "INSERT INTO creators (email, password_hash, display_name)
         VALUES ($1, $2, $3)
         ON CONFLICT (email) DO NOTHING
         RETURNING *",
    )
    .bind(&payload.email)
    .bind(&password_hash)
    .bind(&payload.display_name)
    .fetch_optional(&state.db)
    .await?;

    match creator {
        Some(creator) => Ok((
            StatusCode::CREATED,
            Json(RegisterResponse {
                id: creator.id,
                email: creator.email,
                display_name: creator.display_name,
                created_at: creator.created_at,
            }),
        )),
        None => Err(AppError::Conflict("Email already registered".to_string())),
    }
}

pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let creator: Option<Creator> = sqlx::query_as("SELECT * FROM creators WHERE email = $1")
        .bind(&payload.email)
        .fetch_optional(&state.db)
        .await?;

    let creator = creator.ok_or(AppError::Unauthorized)?;

    let password_hash = creator
        .password_hash
        .as_deref()
        .ok_or(AppError::Unauthorized)?;

    let valid = bcrypt::verify(&payload.password, password_hash)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    if !valid {
        return Err(AppError::Unauthorized);
    }

    let token = jwt::create_token(creator.id, &state.jwt_secret)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(LoginResponse {
        token,
        creator: LoginCreator {
            id: creator.id,
            email: creator.email,
            display_name: creator.display_name,
        },
    }))
}

// ============================================================
// Google OAuth
// ============================================================

#[derive(Debug, Deserialize)]
pub struct GoogleAuthRequest {
    pub id_token: String,
}

#[derive(Debug, Deserialize)]
struct GoogleTokenInfo {
    email: String,
    name: String,
    sub: String,
    email_verified: Option<bool>,
}

pub async fn google_auth(
    State(state): State<AppState>,
    Json(payload): Json<GoogleAuthRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    // Verify token with Google's tokeninfo endpoint
    let client = reqwest::Client::new();
    let url = format!(
        "https://oauth2.googleapis.com/tokeninfo?id_token={}",
        payload.id_token
    );

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to verify Google token: {}", e)))?;

    if !response.status().is_success() {
        return Err(AppError::Unauthorized);
    }

    let token_info: GoogleTokenInfo = response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse Google token info: {}", e)))?;

    // Find or create creator
    let creator = find_or_create_oauth_creator(
        &state.db,
        "google",
        &token_info.sub,
        &token_info.email,
        &token_info.name,
    )
    .await?;

    let token = jwt::create_token(creator.id, &state.jwt_secret)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(LoginResponse {
        token,
        creator: LoginCreator {
            id: creator.id,
            email: creator.email,
            display_name: creator.display_name,
        },
    }))
}

// ============================================================
// GitHub OAuth
// ============================================================

#[derive(Debug, Deserialize)]
pub struct GitHubAuthRequest {
    pub code: String,
}

#[derive(Debug, Deserialize)]
struct GitHubAccessTokenResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct GitHubUser {
    id: u64,
    email: Option<String>,
    name: Option<String>,
    login: String,
}

pub async fn github_auth(
    State(state): State<AppState>,
    Json(payload): Json<GitHubAuthRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let client_id = std::env::var("GITHUB_CLIENT_ID")
        .map_err(|_| AppError::Internal("GITHUB_CLIENT_ID not set".to_string()))?;
    let client_secret = std::env::var("GITHUB_CLIENT_SECRET")
        .map_err(|_| AppError::Internal("GITHUB_CLIENT_SECRET not set".to_string()))?;

    // Exchange code for access token
    let client = reqwest::Client::new();
    let token_response = client
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "client_id": client_id,
            "client_secret": client_secret,
            "code": payload.code,
        }))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to exchange GitHub code: {}", e)))?;

    if !token_response.status().is_success() {
        return Err(AppError::Unauthorized);
    }

    let token_data: GitHubAccessTokenResponse = token_response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse GitHub token response: {}", e)))?;

    // Fetch user info
    let user_response = client
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {}", token_data.access_token))
        .header("User-Agent", "TrendCart")
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to fetch GitHub user: {}", e)))?;

    if !user_response.status().is_success() {
        return Err(AppError::Unauthorized);
    }

    let github_user: GitHubUser = user_response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse GitHub user: {}", e)))?;

    // If email is not public, fetch from emails endpoint
    let email = match github_user.email {
        Some(email) => email,
        None => {
            let emails_response = client
                .get("https://api.github.com/user/emails")
                .header("Authorization", format!("Bearer {}", token_data.access_token))
                .header("User-Agent", "TrendCart")
                .send()
                .await
                .map_err(|e| AppError::Internal(format!("Failed to fetch GitHub emails: {}", e)))?;

            let emails: Vec<serde_json::Value> = emails_response
                .json()
                .await
                .map_err(|e| AppError::Internal(format!("Failed to parse GitHub emails: {}", e)))?;

            emails
                .iter()
                .find(|e| e["primary"] == true && e["verified"] == true)
                .and_then(|e| e["email"].as_str())
                .ok_or(AppError::Internal("No verified email found on GitHub".to_string()))?
                .to_string()
        }
    };

    let display_name = github_user.name.unwrap_or_else(|| github_user.login.clone());
    let oauth_id = github_user.id.to_string();

    // Find or create creator
    let creator = find_or_create_oauth_creator(
        &state.db,
        "github",
        &oauth_id,
        &email,
        &display_name,
    )
    .await?;

    let token = jwt::create_token(creator.id, &state.jwt_secret)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(LoginResponse {
        token,
        creator: LoginCreator {
            id: creator.id,
            email: creator.email,
            display_name: creator.display_name,
        },
    }))
}

// ============================================================
// Shared OAuth helper
// ============================================================

async fn find_or_create_oauth_creator(
    db: &sqlx::PgPool,
    provider: &str,
    oauth_id: &str,
    email: &str,
    display_name: &str,
) -> Result<Creator, AppError> {
    // First, try to find by oauth_provider + oauth_id
    let existing: Option<Creator> = sqlx::query_as(
        "SELECT * FROM creators WHERE oauth_provider = $1 AND oauth_id = $2",
    )
    .bind(provider)
    .bind(oauth_id)
    .fetch_optional(db)
    .await?;

    if let Some(creator) = existing {
        return Ok(creator);
    }

    // Try to find by email (user may have registered with email/password)
    let by_email: Option<Creator> = sqlx::query_as("SELECT * FROM creators WHERE email = $1")
        .bind(email)
        .fetch_optional(db)
    .await?;

    if let Some(creator) = by_email {
        // Link OAuth to existing account
        sqlx::query("UPDATE creators SET oauth_provider = $1, oauth_id = $2 WHERE id = $3")
            .bind(provider)
            .bind(oauth_id)
            .bind(creator.id)
            .execute(db)
            .await?;

        return Ok(Creator {
            oauth_provider: Some(provider.to_string()),
            oauth_id: Some(oauth_id.to_string()),
            ..creator
        });
    }

    // Create new creator
    let new_creator: Creator = sqlx::query_as(
        "INSERT INTO creators (email, oauth_provider, oauth_id, display_name) VALUES ($1, $2, $3, $4) RETURNING *",
    )
    .bind(email)
    .bind(provider)
    .bind(oauth_id)
    .bind(display_name)
    .fetch_one(db)
    .await?;

    Ok(new_creator)
}
