//! 静态资源：内嵌前端（`rust-embed`），可用 `ALKH_STATIC_DIR` 覆盖以便前端开发。

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

use crate::state::AppState;

/// 构建时把 `web/dist` 拷贝到 `crates/alkh-server/web-dist`（见 Makefile / Dockerfile）。
#[derive(RustEmbed)]
#[folder = "web-dist"]
struct Embedded;

pub async fn serve(State(state): State<AppState>, uri: Uri) -> Response {
    let raw_path = uri.path().trim_start_matches('/');
    let path = if raw_path.is_empty() {
        "index.html"
    } else {
        raw_path
    };

    match &state.config.static_dir {
        Some(dir) => serve_from_dir(dir, path).await,
        None => serve_embedded(path),
    }
}

fn serve_embedded(path: &str) -> Response {
    if let Some(file) = Embedded::get(path) {
        return build_response(path, file.data.into_owned());
    }
    // SPA 回退：非资源路径一律返回 index.html
    if !looks_like_asset(path) {
        if let Some(index) = Embedded::get("index.html") {
            return build_response("index.html", index.data.into_owned());
        }
    }
    not_built()
}

async fn serve_from_dir(dir: &std::path::Path, path: &str) -> Response {
    let candidate = dir.join(path);
    // 防目录穿越
    if candidate
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return (StatusCode::BAD_REQUEST, "invalid path").into_response();
    }
    if candidate.is_file() {
        match tokio::fs::read(&candidate).await {
            Ok(bytes) => return build_response(path, bytes),
            Err(e) => {
                tracing::warn!(error = %e, path = %candidate.display(), "读取静态文件失败");
            }
        }
    }
    if !looks_like_asset(path) {
        let index = dir.join("index.html");
        if let Ok(bytes) = tokio::fs::read(&index).await {
            return build_response("index.html", bytes);
        }
    }
    not_built()
}

fn looks_like_asset(path: &str) -> bool {
    path.starts_with("assets/")
        || path.contains('.')
        || path.ends_with(".js")
        || path.ends_with(".css")
        || path.ends_with(".map")
}

fn build_response(path: &str, bytes: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, mime.as_ref()),
            (header::CACHE_CONTROL, cache),
        ],
        Body::from(bytes),
    )
        .into_response()
}

fn not_built() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "管理后台静态资源未构建。请先执行 `make web`（或 `cd web && pnpm build`）后重新编译，\
         或设置 ALKH_STATIC_DIR 指向 web/dist。",
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_detection() {
        assert!(looks_like_asset("assets/index-abc.js"));
        assert!(looks_like_asset("favicon.ico"));
        assert!(!looks_like_asset("channels"));
        assert!(!looks_like_asset("channels/3f6c"));
    }
}
