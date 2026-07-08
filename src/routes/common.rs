use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct PaginatedResponse<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: i64,
    pub limit: i64,
}

impl<T> PaginatedResponse<T> {
    pub fn new(items: Vec<T>, total: i64, page: i64, limit: i64) -> Self {
        Self {
            items,
            total,
            page,
            limit,
        }
    }
}

pub fn parse_pagination(params: &PaginationParams) -> (i64, i64, i64) {
    let page = params.page.unwrap_or(1).max(1);
    let limit = params.limit.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * limit;
    (page, limit, offset)
}
