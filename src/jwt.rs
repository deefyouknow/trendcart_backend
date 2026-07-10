use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Access token TTL (short-lived — 15 minutes).
pub const ACCESS_TOKEN_TTL_SECS: i64 = 15 * 60;

/// Refresh token TTL (long-lived — 7 days).
pub const REFRESH_TOKEN_TTL_SECS: i64 = 7 * 24 * 3600;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
}

/// Create a short-lived access token (15 min).
pub fn create_access_token(creator_id: Uuid, secret: &str) -> Result<String, jsonwebtoken::errors::Error> {
    let expiration = Utc::now()
        .checked_add_signed(Duration::seconds(ACCESS_TOKEN_TTL_SECS))
        .expect("valid timestamp")
        .timestamp() as usize;

    let claims = Claims {
        sub: creator_id.to_string(),
        exp: expiration,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
}

/// Generate an opaque refresh token (UUID v4).
pub fn create_refresh_token() -> String {
    Uuid::new_v4().to_string()
}

/// Kept for backward-compat during migration — delegates to create_access_token.
pub fn create_token(creator_id: Uuid, secret: &str) -> Result<String, jsonwebtoken::errors::Error> {
    create_access_token(creator_id, secret)
}

pub fn verify_token(token: &str, secret: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )?;
    Ok(token_data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_token_then_verify_token_roundtrips_creator_id() {
        let creator_id = Uuid::new_v4();
        let token = create_token(creator_id, "test_secret").expect("token should be created");
        let claims = verify_token(&token, "test_secret").expect("token should verify");
        assert_eq!(claims.sub, creator_id.to_string());
    }

    #[test]
    fn verify_token_rejects_token_signed_with_different_secret() {
        let creator_id = Uuid::new_v4();
        let token = create_token(creator_id, "secret_a").expect("token should be created");
        let result = verify_token(&token, "secret_b");
        assert!(result.is_err());
    }

    #[test]
    fn verify_token_rejects_garbage_token() {
        let result = verify_token("not.a.valid.jwt", "test_secret");
        assert!(result.is_err());
    }
}
