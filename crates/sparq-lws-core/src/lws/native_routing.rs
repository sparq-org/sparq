//! Opt-in base-path mount used by the native binary; the library router itself keeps accepting storage-relative paths.

use axum::extract::{Request, State};
use axum::http::{StatusCode, Uri};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Router;

use super::discovery;

/// Parse the native mount switch; unset or empty values leave mounting disabled.
pub fn parse_mount_base_path(value: Option<&str>) -> Result<bool, String> {
    match value.map(str::trim) {
        None | Some("" | "0" | "false" | "False" | "FALSE") => Ok(false),
        Some("1" | "true" | "True" | "TRUE") => Ok(true),
        Some(value) => Err(format!(
            "invalid SOLID_SERVER_LWS_MOUNT_BASE_PATH value {value:?}: expected \
             1, true, True, TRUE, 0, false, False, FALSE, or an empty value"
        )),
    }
}

/// Opt in to native base-path mounting around a storage-relative LWS router.
pub fn mount_lws_router(app: Router, base_url: &str, enabled: bool) -> Result<Router, String> {
    if !enabled {
        return Ok(app);
    }
    // Use the HTTP spelling of the path: URL parsing would normalize stored resource names.
    let base: Uri = base_url
        .parse()
        .map_err(|e| format!("invalid LWS mount base URL: {e}"))?;
    let prefix = base.path().trim_end_matches('/');
    if prefix.is_empty() {
        return Ok(app);
    }
    Ok(app.layer(from_fn_with_state(prefix.to_owned(), strip_base_path)))
}

async fn strip_base_path(State(prefix): State<String>, mut req: Request, next: Next) -> Response {
    let path = req.uri().path();
    // RFC 9728 metadata lives at the origin, before the storage path. Forward it verbatim.
    const METADATA: &str = "/.well-known/oauth-protected-resource";
    if path == METADATA || path.starts_with(&format!("{METADATA}/")) {
        return next.run(req).await;
    }
    let Some(suffix) = path.strip_prefix(&prefix) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if suffix.is_empty() {
        let location = match req.uri().query() {
            Some(query) => format!("{prefix}/?{query}"),
            None => format!("{prefix}/"),
        };
        return Redirect::permanent(&location).into_response();
    }
    if !suffix.starts_with('/')
        || suffix.starts_with(METADATA)
        || discovery::has_dot_segments(suffix)
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    // Strip exactly once, preserving encoded octets, slash distinctions and the query.
    let path_and_query = match req.uri().query() {
        Some(query) => format!("{suffix}?{query}"),
        None => suffix.to_owned(),
    };
    let mut parts = req.uri().clone().into_parts();
    let Ok(path_and_query) = path_and_query.parse() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    parts.path_and_query = Some(path_and_query);
    let Ok(uri) = Uri::from_parts(parts) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    *req.uri_mut() = uri;
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::header;
    use tower::ServiceExt;

    fn echo() -> Router {
        Router::new().fallback(|req: Request| async move { req.uri().to_string() })
    }

    async fn call(base: &str, enabled: bool, path: &str) -> Response {
        mount_lws_router(echo(), base, enabled)
            .unwrap()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[test]
    fn parse_mount_switch() {
        assert!(!parse_mount_base_path(None).unwrap());
        for value in ["", " ", "0", "false", "False", "FALSE", " false "] {
            assert!(!parse_mount_base_path(Some(value)).unwrap(), "{value:?}");
        }
        for value in ["1", "true", "True", "TRUE", " true "] {
            assert!(parse_mount_base_path(Some(value)).unwrap(), "{value:?}");
        }
        for value in ["on", "yes", "off", "no", "2", "tRuE", "fAlSe"] {
            let message = parse_mount_base_path(Some(value)).unwrap_err();
            assert!(message.contains("SOLID_SERVER_LWS_MOUNT_BASE_PATH"));
            assert!(message.contains(value));
            assert!(message.contains("expected"));
        }
    }

    #[test]
    fn parse_mount_base_url() {
        for base in ["", "https://[", "https://storage.example/\n"] {
            let result = mount_lws_router(echo(), base, true);
            assert!(
                matches!(result, Err(message) if message.contains("invalid LWS mount base URL"))
            );
        }
        assert!(mount_lws_router(echo(), "https://[", false).is_ok());
    }

    #[tokio::test]
    async fn origin_and_disabled_mount_preserve_paths() {
        for (base, enabled) in [
            ("https://storage.example", true),
            ("https://storage.example/", true),
            ("https://storage.example/alice/", false),
        ] {
            let path = "/alice//child?q=a%2Fb";
            let response = call(base, enabled, path).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(to_bytes(response.into_body(), 1024).await.unwrap(), path);
        }
    }

    #[tokio::test]
    async fn bare_mount_root_redirects_with_query_for_every_method() {
        for base in [
            "https://storage.example/alice",
            "https://storage.example/alice/",
        ] {
            for method in ["GET", "HEAD", "POST"] {
                for (path, location) in [
                    ("/alice", "/alice/"),
                    ("/alice?q", "/alice/?q"),
                    ("/alice?q=a%2Fb&x=1", "/alice/?q=a%2Fb&x=1"),
                ] {
                    let response = mount_lws_router(echo(), base, true)
                        .unwrap()
                        .oneshot(
                            Request::builder()
                                .method(method)
                                .uri(path)
                                .body(Body::empty())
                                .unwrap(),
                        )
                        .await
                        .unwrap();
                    assert_eq!(
                        response.status(),
                        StatusCode::PERMANENT_REDIRECT,
                        "{method} {path}"
                    );
                    assert_eq!(response.headers()[header::LOCATION], location);
                    assert!(!response.headers().contains_key(header::WWW_AUTHENTICATE));
                }
            }
        }
    }

    #[tokio::test]
    async fn mount_preserves_suffix_spelling_and_strips_once() {
        for (base, path, expected) in [
            ("https://storage.example/alice", "/alice/", "/"),
            ("https://storage.example/alice/", "/alice//child", "//child"),
            (
                "https://storage.example/alice",
                "/alice/alice/child",
                "/alice/child",
            ),
            (
                "https://storage.example/alice",
                "/alice/a%2Fb%20/?q=x%2Fy",
                "/a%2Fb%20/?q=x%2Fy",
            ),
            (
                "https://storage.example/%61lice/",
                "/%61lice/child",
                "/child",
            ),
            (
                "https://storage.example/team/alice/",
                "/team/alice/child",
                "/child",
            ),
        ] {
            let response = call(base, true, path).await;
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(
                to_bytes(response.into_body(), 1024).await.unwrap(),
                expected
            );
        }
    }

    #[tokio::test]
    async fn mount_rejects_outside_and_reserved_suffixes() {
        for path in [
            "/",
            "/alicex",
            "/alicex/child",
            "/%61lice/child",
            "/alice/.well-known/oauth-protected-resource",
            "/alice/.well-known/oauth-protected-resource/alice/child",
            "/alice/.well-known/oauth-protected-resource-other",
            "/alice/./child",
            "/alice/../child",
            "/alice/child/..",
            "/alice/%2e/child",
            "/alice/.%2E/child",
            "/alice/%2e./child",
            "/alice/%2E%2e/child",
            "/alice//../child?q=1",
        ] {
            let response = call("https://storage.example/alice", true, path).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
            assert!(!response.headers().contains_key(header::WWW_AUTHENTICATE));
        }
    }

    #[tokio::test]
    async fn origin_metadata_passes_unchanged_and_other_well_known_paths_mount() {
        for (path, expected) in [
            (
                "/.well-known/oauth-protected-resource",
                "/.well-known/oauth-protected-resource",
            ),
            (
                "/.well-known/oauth-protected-resource/alice/a%2Fb/?q=x",
                "/.well-known/oauth-protected-resource/alice/a%2Fb/?q=x",
            ),
            (
                "/alice/.well-known/lws-configuration",
                "/.well-known/lws-configuration",
            ),
        ] {
            let response = call("https://storage.example/alice", true, path).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                to_bytes(response.into_body(), 1024).await.unwrap(),
                expected
            );
        }
    }
}
