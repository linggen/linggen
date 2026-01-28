use axum::{extract::Query, http::StatusCode, response::IntoResponse, Json};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct SkillsShQuery {
    pub q: String,
    pub limit: Option<u32>,
}

pub async fn search_skills_sh(Query(params): Query<SkillsShQuery>) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(10).min(50);
    let url = format!(
        "https://skills.sh/api/search?q={}&limit={}",
        urlencoding::encode(&params.q),
        limit
    );

    match reqwest::Client::new().get(url).send().await {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16())
                .unwrap_or(StatusCode::BAD_GATEWAY);
            match resp.json::<serde_json::Value>().await {
                Ok(json) => (status, Json(json)).into_response(),
                Err(err) => (
                    axum::http::StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({
                        "error": "Failed to parse skills.sh response",
                        "details": err.to_string()
                    })),
                )
                    .into_response(),
            }
        }
        Err(err) => (
            axum::http::StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": "Failed to fetch skills.sh",
                "details": err.to_string()
            })),
        )
            .into_response(),
    }
}
