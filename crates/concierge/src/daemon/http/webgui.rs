//! Serves the embedded WebGUI (built with `--features webgui`, after
//! `npm run build` in webgui/). Without the feature a plain placeholder page
//! keeps the route alive so the pipeline is always exercised.

use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};

#[cfg(feature = "webgui")]
#[derive(rust_embed::RustEmbed)]
#[folder = "../../webgui/dist"]
struct Assets;

#[cfg(feature = "webgui")]
pub async fn spa_fallback(uri: Uri) -> Response {
    let requested = uri.path().trim_start_matches('/');
    // Unknown non-asset paths fall back to index.html: client-side routing.
    let path = if Assets::get(requested).is_some() {
        requested
    } else {
        "index.html"
    };
    let Some(asset) = Assets::get(path) else {
        return (
            StatusCode::NOT_FOUND,
            "webgui assets missing from this build (run `npm run build` in webgui/ and rebuild)",
        )
            .into_response();
    };
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    (
        [(axum::http::header::CONTENT_TYPE, mime.as_ref())],
        asset.data,
    )
        .into_response()
}

#[cfg(not(feature = "webgui"))]
pub async fn spa_fallback(_uri: Uri) -> Response {
    (
        StatusCode::OK,
        axum::response::Html(concat!(
            "<h1>foyer-concierge</h1>",
            "<p>This build does not embed the WebGUI. ",
            "Build with <code>cargo build --features webgui</code> ",
            "after <code>npm run build</code> in webgui/.</p>"
        )),
    )
        .into_response()
}
