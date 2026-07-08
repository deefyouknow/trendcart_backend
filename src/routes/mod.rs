use axum::Router;

use crate::state::AppState;

pub mod admin_conversions;
pub mod admin_merchant_links;
pub mod admin_products;
pub mod admin_scrape;
pub mod auth;
pub mod common;
pub mod redirect;
pub mod store_products;
pub mod upload;

pub fn router(state: AppState) -> Router {
    Router::new()
        .nest("/api/auth", auth::router())
        .nest(
            "/api/admin",
            admin_products::router()
                .merge(admin_merchant_links::router())
                .merge(upload::router())
                .merge(admin_scrape::router())
                .merge(admin_conversions::router()),
        )
        .nest("/api/store/products", store_products::router())
        .nest("/api", admin_conversions::postback_router())
        .merge(redirect::router())
        .with_state(state)
}
