//! Embedded dashboard static assets served by the HTTP server.

use axum::body::Body;
use axum::http::{HeaderValue, Response, StatusCode, Uri, header};
use axum::response::IntoResponse;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../dashboard/dist/"]
struct DashboardAssets;

/// Axum handler that serves the embedded dashboard bundle.
///
/// Unknown paths fall back to `index.html` so the React router can
/// handle client-side routes.
pub async fn handler(uri: Uri) -> Response<Body> {
    let path = uri.path().trim_start_matches('/');
    let candidate = if path.is_empty() { "index.html" } else { path };

    if let Some(resp) = lookup(candidate) {
        return resp;
    }

    if let Some(resp) = lookup("index.html") {
        return resp;
    }

    (StatusCode::NOT_FOUND, "not found").into_response()
}

fn lookup(path: &str) -> Option<Response<Body>> {
    let file = DashboardAssets::get(path)?;
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let mut resp = Response::new(Body::from(file.data.into_owned()));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime.as_ref()).unwrap(),
    );
    Some(resp)
}
