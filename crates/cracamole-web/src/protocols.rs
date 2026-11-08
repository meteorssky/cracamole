//! Protocol catalogue: the single place the web layer learns which protocols exist.

use crate::{ApiError, App};
use axum::{extract::State, http::HeaderMap, Json};
use guac_session::ProtocolInfo;
use guacd::Registry;
use serde_json::{json, Value};
use std::sync::{Arc, OnceLock};

/// Protocols compiled into this binary.
pub fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Registry::builtin)
}

pub fn info(name: &str) -> Option<ProtocolInfo> {
    registry().get(name).map(|plugin| plugin.info())
}

/// Describes every protocol so the front end can build its forms without hard-coded names.
pub async fn list(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    app.auth.authorize(&headers).await?;
    let protocols: Vec<Value> = registry()
        .plugins()
        .map(|plugin| {
            let info = plugin.info();
            json!({
                "name": plugin.name(),
                "label": info.label,
                "default_port": info.default_port,
                "identity": info.identity.as_str(),
                "args": plugin.arg_names(),
            })
        })
        .collect();
    Ok(Json(json!(protocols)))
}
