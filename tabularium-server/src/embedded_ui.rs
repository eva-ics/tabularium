//! Embedded Vite `ui/dist` — served after API routes so `/rpc` never becomes HTML.

use std::path::Path;

use axum::body::Body;
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use include_dir::{Dir, include_dir};

static UI_DIST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../ui/dist");

fn content_type(path: &str) -> &'static str {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "application/javascript; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("json" | "map") => "application/json; charset=utf-8",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn file_response(path: &str, file: &include_dir::File<'_>) -> Response {
    let ct = content_type(path);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, ct)
        .body(Body::from(file.contents().to_vec()))
        .unwrap()
}

pub async fn serve(uri: Uri) -> impl IntoResponse {
    let raw = uri.path();
    let mut p = raw.trim_start_matches('/');
    if p.is_empty() {
        p = "index.html";
    }
    if p.contains("..") {
        return StatusCode::NOT_FOUND.into_response();
    }

    if let Some(file) = UI_DIST.get_file(p) {
        return file_response(p, file);
    }

    match UI_DIST.get_file("index.html") {
        Some(file) => file_response("index.html", file),
        None => Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("UI dist not embedded — build ui/ first"))
            .unwrap(),
    }
}
