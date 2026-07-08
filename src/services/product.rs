use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::models::merchant_link::MerchantLink;
use crate::models::product::Product;
use crate::models::product_variant::ProductVariant;

#[derive(Debug, sqlx::FromRow, Serialize)]
pub struct AdminProductListItem {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub category: String,
    pub images: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub merchant_links_count: i64,
}

pub struct ProductService;

impl ProductService {
    pub async fn list_admin_products(
        db: &PgPool,
        creator_id: Uuid,
        category: Option<String>,
        search: Option<String>,
        page: i64,
        limit: i64,
    ) -> Result<(Vec<AdminProductListItem>, i64), AppError> {
        let offset = (page - 1) * limit;

        let products: Vec<AdminProductListItem> = sqlx::query_as(
            r#"
            SELECT p.id, p.title, p.description, p.category, p.images,
                   p.created_at, p.updated_at,
                   COUNT(ml.id) AS merchant_links_count
            FROM products p
            LEFT JOIN merchant_links ml ON ml.product_id = p.id
            WHERE p.creator_id = $1
              AND p.deleted_at IS NULL
              AND ($2::text IS NULL OR p.category = $2)
              AND ($3::text IS NULL OR p.title ILIKE '%' || $3 || '%')
            GROUP BY p.id
            ORDER BY p.created_at DESC
            LIMIT $4 OFFSET $5
            "#,
        )
        .bind(creator_id)
        .bind(&category)
        .bind(&search)
        .bind(limit)
        .bind(offset)
        .fetch_all(db)
        .await?;

        let total: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)
            FROM products
            WHERE creator_id = $1
              AND deleted_at IS NULL
              AND ($2::text IS NULL OR category = $2)
              AND ($3::text IS NULL OR title ILIKE '%' || $3 || '%')
            "#,
        )
        .bind(creator_id)
        .bind(&category)
        .bind(&search)
        .fetch_one(db)
        .await?;

        Ok((products, total))
    }

    pub async fn create_product(
        db: &PgPool,
        creator_id: Uuid,
        title: String,
        description: String,
        category: String,
        images: serde_json::Value,
    ) -> Result<Product, AppError> {
        let product: Product = sqlx::query_as(
            r#"
            INSERT INTO products (creator_id, title, description, category, images)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING *
            "#,
        )
        .bind(creator_id)
        .bind(&title)
        .bind(&description)
        .bind(&category)
        .bind(&images)
        .fetch_one(db)
        .await?;

        Ok(product)
    }

    pub async fn get_admin_product(
        db: &PgPool,
        creator_id: Uuid,
        product_id: Uuid,
    ) -> Result<(Product, Vec<serde_json::Value>), AppError> {
        let product: Option<Product> =
            sqlx::query_as("SELECT * FROM products WHERE id = $1 AND deleted_at IS NULL")
                .bind(product_id)
                .fetch_optional(db)
                .await?;

        let product = product.ok_or_else(|| AppError::NotFound("Product not found".to_string()))?;

        if product.creator_id != creator_id {
            return Err(AppError::Forbidden);
        }

        let merchant_links: Vec<MerchantLink> = sqlx::query_as(
            "SELECT * FROM merchant_links WHERE product_id = $1 ORDER BY updated_at",
        )
        .bind(product_id)
        .fetch_all(db)
        .await?;

        let mut merchant_links_response = Vec::new();
        for link in merchant_links {
            let variants: Vec<ProductVariant> = sqlx::query_as(
                "SELECT * FROM product_variants WHERE merchant_link_id = $1 ORDER BY variant_name",
            )
            .bind(link.id)
            .fetch_all(db)
            .await?;

            merchant_links_response.push(serde_json::json!({
                "id": link.id,
                "platform": link.platform,
                "store_name": link.store_name,
                "affiliate_url": link.affiliate_url,
                "is_price_estimated": link.is_price_estimated,
                "price_checked_at": link.price_checked_at,
                "variants": variants,
            }));
        }

        Ok((product, merchant_links_response))
    }

    pub async fn update_product(
        db: &PgPool,
        creator_id: Uuid,
        product_id: Uuid,
        title: Option<String>,
        description: Option<String>,
        category: Option<String>,
        images: Option<serde_json::Value>,
    ) -> Result<Product, AppError> {
        let existing: Option<Product> =
            sqlx::query_as("SELECT * FROM products WHERE id = $1 AND deleted_at IS NULL")
                .bind(product_id)
                .fetch_optional(db)
                .await?;

        let existing = existing.ok_or_else(|| AppError::NotFound("Product not found".to_string()))?;

        if existing.creator_id != creator_id {
            return Err(AppError::Forbidden);
        }

        let title = title.unwrap_or(existing.title);
        let description = description.unwrap_or(existing.description);
        let category = category.unwrap_or(existing.category);
        let images = images.unwrap_or(existing.images);

        let updated: Product = sqlx::query_as(
            r#"
            UPDATE products
            SET title = $1, description = $2, category = $3, images = $4
            WHERE id = $5
            RETURNING *
            "#,
        )
        .bind(&title)
        .bind(&description)
        .bind(&category)
        .bind(&images)
        .bind(product_id)
        .fetch_one(db)
        .await?;

        Ok(updated)
    }

    pub async fn delete_product(
        db: &PgPool,
        creator_id: Uuid,
        product_id: Uuid,
    ) -> Result<(), AppError> {
        let existing: Option<Product> =
            sqlx::query_as("SELECT * FROM products WHERE id = $1 AND deleted_at IS NULL")
                .bind(product_id)
                .fetch_optional(db)
                .await?;

        let existing = existing.ok_or_else(|| AppError::NotFound("Product not found".to_string()))?;

        if existing.creator_id != creator_id {
            return Err(AppError::Forbidden);
        }

        sqlx::query("UPDATE products SET deleted_at = NOW() WHERE id = $1")
            .bind(product_id)
            .execute(db)
            .await?;

        Ok(())
    }
}
