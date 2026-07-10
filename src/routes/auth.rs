use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::jwt;
use crate::models::creator::Creator;
use crate::state::AppState;

/// SameSite value based on environment (localhost = Lax, prod = None+Secure).
fn cookie_samesite() -> &'static str {
    let origin = std::env::var("FRONTEND_ORIGIN").unwrap_or_default();
    if origin.contains("localhost") {
        "Lax"
    } else {
        "None"
    }
}

/// Build Set-Cookie header for the access token (tc_token).
/// non-httpOnly — frontend JS can read it if needed.
fn build_access_cookie(token: &str) -> String {
    let ss = cookie_samesite();
    let secure = if ss == "None" { "; Secure" } else { "" };
    format!(
        "tc_token={}; Path=/; SameSite={}; Max-Age={}{}",
        token,
        ss,
        jwt::ACCESS_TOKEN_TTL_SECS,
        secure,
    )
}

/// Build Set-Cookie header for the refresh token (tc_refresh).
/// httpOnly — not accessible from JS, only sent automatically by the browser.
fn build_refresh_cookie(token: &str) -> String {
    let ss = cookie_samesite();
    let secure = if ss == "None" { "; Secure" } else { "" };
    format!(
        "tc_refresh={}; Path=/; HttpOnly; SameSite={}; Max-Age={}{}",
        token,
        ss,
        jwt::REFRESH_TOKEN_TTL_SECS,
        secure,
    )
}

/// Build Set-Cookie header to clear a cookie by name.
fn build_clear_cookie(name: &str) -> String {
    let ss = cookie_samesite();
    let secure = if ss == "None" { "; Secure" } else { "" };
    format!(
        "{name}=; Path=/; HttpOnly; SameSite={ss}; Max-Age=0{secure}",
    )
}

/// Issue an access token + refresh token pair.
/// Stores the refresh token in Redis (forward + reverse lookup) and returns cookie header values.
async fn issue_token_pair(
    state: &AppState,
    creator: &Creator,
) -> Result<(String, String), AppError> {
    let access_token = jwt::create_access_token(creator.id, &state.jwt_secret)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let refresh_token = jwt::create_refresh_token();
    let user_id = creator.id.to_string();
    let ttl = jwt::REFRESH_TOKEN_TTL_SECS as u64;

    // Forward: session:refresh:{user_id} → refresh_token
    state.redis.set_session(&user_id, &refresh_token, ttl).await;
    // Reverse: session:rt:{refresh_token} → user_id (for lookup during refresh)
    state.redis.set(&format!("session:rt:{}", refresh_token), &user_id, ttl).await;

    Ok((build_access_cookie(&access_token), build_refresh_cookie(&refresh_token)))
}

/// Build a response with multiple Set-Cookie headers + JSON body.
/// Axum's tuple return type deduplicates headers with the same name,
/// so we use HeaderMap::append() to keep both cookies.
fn cookie_response<T: serde::Serialize>(
    status: StatusCode,
    cookies: Vec<String>,
    body: T,
) -> Result<Response, AppError> {
    let value = serde_json::to_vec(&body)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    for cookie in cookies {
        let val = HeaderValue::from_str(&cookie)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        headers.append(header::SET_COOKIE, val);
    }
    let mut resp = Response::new(axum::body::Body::from(value));
    *resp.status_mut() = status;
    *resp.headers_mut() = headers;
    Ok(resp)
}

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
    pub creator: LoginCreator,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/refresh", post(refresh))
        .route("/logout", post(logout))
        .route("/google", post(google_auth))
        .route("/github", post(github_auth))
}

pub async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterRequest>,
) -> Result<Response, AppError> {
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
        Some(creator) => {
            let (access_cookie, refresh_cookie) = issue_token_pair(&state, &creator).await?;
            cookie_response(
                StatusCode::CREATED,
                vec![access_cookie, refresh_cookie],
                RegisterResponse {
                    id: creator.id,
                    email: creator.email,
                    display_name: creator.display_name,
                    created_at: creator.created_at,
                },
            )
        }
        None => Err(AppError::Conflict("Email already registered".to_string())),
    }
}

pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<Response, AppError> {
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

    let (access_cookie, refresh_cookie) = issue_token_pair(&state, &creator).await?;
    cookie_response(
        StatusCode::OK,
        vec![access_cookie, refresh_cookie],
        LoginResponse {
            creator: LoginCreator {
                id: creator.id,
                email: creator.email,
                display_name: creator.display_name,
            },
        },
    )
}

// ============================================================
// Refresh Token
// ============================================================

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    /// The refresh token. Accepts from body (frontend JS can read tc_refresh cookie
    /// and send it, or we read it from the httpOnly cookie via axum extract).
    pub refresh_token: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RefreshResponse {
    pub creator: LoginCreator,
}

/// POST /api/auth/refresh — exchange a valid refresh token for a new access + refresh pair.
/// The old refresh token is rotated (invalidated) on every use.
pub async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<RefreshRequest>,
) -> Result<Response, AppError> {
    // 1. Get refresh token: prefer body, fallback to cookie
    let refresh_token = if let Some(token) = payload.refresh_token {
        token
    } else {
        // Try reading from cookie header
        let cookie_header = headers
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        parse_cookie_value(cookie_header, "tc_refresh")
            .ok_or(AppError::Unauthorized)?
            .to_string()
    };

    // Try reverse lookup: rt:{refresh_token} → user_id
    let user_id_str = state
        .redis
        .get(&format!("session:rt:{}", refresh_token))
        .await
        .ok_or(AppError::Unauthorized)?;

    // 3. Verify the stored session matches
    let stored_token = state.redis.get_session(&user_id_str).await;
    match stored_token {
        Some(stored) if stored == refresh_token => { /* valid — continue */ }
        _ => return Err(AppError::Unauthorized),
    }

    // 4. Look up the user to issue new tokens
    let user_uuid = Uuid::parse_str(&user_id_str)
        .map_err(|_| AppError::Internal("Invalid user id in session".to_string()))?;
    let creator: Creator = sqlx::query_as("SELECT * FROM creators WHERE id = $1")
        .bind(user_uuid)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::Unauthorized)?;

    // 5. Rotate: delete old session, issue new pair
    state.redis.delete_session(&user_id_str).await;
    state.redis.delete(&format!("session:rt:{}", refresh_token)).await;

    let (access_cookie, refresh_cookie) = issue_token_pair(&state, &creator).await?;
    cookie_response(
        StatusCode::OK,
        vec![access_cookie, refresh_cookie],
        RefreshResponse {
            creator: LoginCreator {
                id: creator.id,
                email: creator.email,
                display_name: creator.display_name,
            },
        },
    )
}

// ============================================================
// Logout
// ============================================================

/// POST /api/auth/logout — revoke session from Redis + clear cookies.
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // Try to get user_id from access token (cookie) to clean up session
    let cookie_header = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if let Some(access_token) = parse_cookie_value(cookie_header, "tc_token") {
        if let Ok(claims) = jwt::verify_token(access_token, &state.jwt_secret) {
            // Delete session + reverse lookup
            state.redis.delete_session(&claims.sub).await;
            // Also scan for rt: keys for this user (we can't easily reverse, but try common patterns)
            // For now, just delete the session — the orphaned rt: keys will expire via TTL.
        }
    }

    // Clear both cookies
    let clear_access = build_clear_cookie("tc_token");
    let clear_refresh = build_clear_cookie("tc_refresh");

    cookie_response(
        StatusCode::OK,
        vec![clear_access, clear_refresh],
        serde_json::json!({ "message": "Logged out" }),
    )
}

/// Parse a cookie value from a raw `Cookie` header string.
fn parse_cookie_value<'a>(cookie_header: &'a str, name: &str) -> Option<&'a str> {
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some(val) = part.strip_prefix(&format!("{}=", name)) {
            return Some(val);
        }
    }
    None
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
) -> Result<Response, AppError> {
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

    let (access_cookie, refresh_cookie) = issue_token_pair(&state, &creator).await?;
    cookie_response(
        StatusCode::OK,
        vec![access_cookie, refresh_cookie],
        LoginResponse {
            creator: LoginCreator {
                id: creator.id,
                email: creator.email,
                display_name: creator.display_name,
            },
        },
    )
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
) -> Result<Response, AppError> {
    let client = reqwest::Client::new();
    let token_data = exchange_github_code(&client, &payload.code).await?;
    let github_user = fetch_github_user(&client, &token_data.access_token).await?;
    let email = resolve_github_email(&client, &token_data.access_token, &github_user).await?;

    let display_name = github_user.name.unwrap_or_else(|| github_user.login.clone());
    let oauth_id = github_user.id.to_string();

    let creator = find_or_create_oauth_creator(
        &state.db, "github", &oauth_id, &email, &display_name,
    )
    .await?;

    let (access_cookie, refresh_cookie) = issue_token_pair(&state, &creator).await?;
    cookie_response(
        StatusCode::OK,
        vec![access_cookie, refresh_cookie],
        LoginResponse {
            creator: LoginCreator {
                id: creator.id,
                email: creator.email,
                display_name: creator.display_name,
            },
        },
    )
}

// ── GitHub OAuth helpers ──────────────────────────────────────

async fn exchange_github_code(
    client: &reqwest::Client,
    code: &str,
) -> Result<GitHubAccessTokenResponse, AppError> {
    let client_id = std::env::var("GITHUB_CLIENT_ID")
        .map_err(|_| AppError::Internal("GITHUB_CLIENT_ID not set".to_string()))?;
    let client_secret = std::env::var("GITHUB_CLIENT_SECRET")
        .map_err(|_| AppError::Internal("GITHUB_CLIENT_SECRET not set".to_string()))?;

    let response = client
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "client_id": client_id,
            "client_secret": client_secret,
            "code": code,
        }))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to exchange GitHub code: {}", e)))?;

    if !response.status().is_success() {
        return Err(AppError::Unauthorized);
    }

    response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse GitHub token response: {}", e)))
}

async fn fetch_github_user(
    client: &reqwest::Client,
    access_token: &str,
) -> Result<GitHubUser, AppError> {
    let response = client
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {}", access_token))
        .header("User-Agent", "TrendCart")
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to fetch GitHub user: {}", e)))?;

    if !response.status().is_success() {
        return Err(AppError::Unauthorized);
    }

    response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse GitHub user: {}", e)))
}

async fn resolve_github_email(
    client: &reqwest::Client,
    access_token: &str,
    github_user: &GitHubUser,
) -> Result<String, AppError> {
    if let Some(ref email) = github_user.email {
        return Ok(email.clone());
    }

    let response = client
        .get("https://api.github.com/user/emails")
        .header("Authorization", format!("Bearer {}", access_token))
        .header("User-Agent", "TrendCart")
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to fetch GitHub emails: {}", e)))?;

    let emails: Vec<serde_json::Value> = response
        .json()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to parse GitHub emails: {}", e)))?;

    emails
        .iter()
        .find(|e| e["primary"] == true && e["verified"] == true)
        .and_then(|e| e["email"].as_str())
        .map(String::from)
        .ok_or_else(|| AppError::Internal("No verified email found on GitHub".to_string()))
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
