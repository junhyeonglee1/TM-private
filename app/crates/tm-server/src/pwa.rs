use axum::{
    Router,
    body::Body,
    http::{
        HeaderValue, StatusCode,
        header::{CONTENT_TYPE, LOCATION},
    },
    response::{IntoResponse, Response},
    routing::get,
};

use crate::AppState;

const INDEX: &str = include_str!("pwa/index.html");
const APP_JS: &str = include_str!("pwa/app.js");
const STYLES: &str = include_str!("pwa/styles.css");
const MANIFEST: &str = include_str!("pwa/manifest.webmanifest");
const ICON: &str = include_str!("pwa/icon.svg");
const SERVICE_WORKER: &str = include_str!("pwa/sw.js");
const ICON_256: &[u8] = include_bytes!("../../../src-tauri/icons/128x128@2x.png");
const ICON_512: &[u8] = include_bytes!("../../../src-tauri/icons/icon.png");

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/mobile", get(redirect_to_mobile))
        .route("/mobile/", get(index))
        .route("/mobile/app.js", get(app_js))
        .route("/mobile/styles.css", get(styles))
        .route("/mobile/manifest.webmanifest", get(manifest))
        .route("/mobile/icon.svg", get(icon))
        .route("/mobile/icon-256.png", get(icon_256))
        .route("/mobile/icon-512.png", get(icon_512))
        .route("/mobile/sw.js", get(service_worker))
}

async fn redirect_to_mobile() -> Response {
    let mut response = StatusCode::PERMANENT_REDIRECT.into_response();
    response
        .headers_mut()
        .insert(LOCATION, HeaderValue::from_static("/mobile/"));
    response
}

async fn index() -> Response {
    content(INDEX, "text/html; charset=utf-8")
}

async fn app_js() -> Response {
    content(APP_JS, "text/javascript; charset=utf-8")
}

async fn styles() -> Response {
    content(STYLES, "text/css; charset=utf-8")
}

async fn manifest() -> Response {
    content(MANIFEST, "application/manifest+json; charset=utf-8")
}

async fn icon() -> Response {
    content(ICON, "image/svg+xml; charset=utf-8")
}

async fn icon_256() -> Response {
    binary_content(ICON_256, "image/png")
}

async fn icon_512() -> Response {
    binary_content(ICON_512, "image/png")
}

async fn service_worker() -> Response {
    content(SERVICE_WORKER, "text/javascript; charset=utf-8")
}

fn content(body: &'static str, content_type: &'static str) -> Response {
    let mut response = body.into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

fn binary_content(body: &'static [u8], content_type: &'static str) -> Response {
    let mut response = Response::new(Body::from(body));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}
