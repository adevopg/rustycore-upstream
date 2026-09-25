//! In-game browser support: `GET /bnetserver/browser/urlmap/`.
//!
//! Port of `LegionCore` `LoginRESTService::SendBrowserUrlMap`
//! (`src/server/bnetserver/REST/LoginRESTService.cpp`): a plain
//! `{"host":"target",...}` object built from `auth.browser_url_map`, read by the
//! client-side CEF shim (`tools/wow-cef-shim`, `shim_urlmap.c`). No protobuf message
//! describes it, so the JSON is assembled by hand with the C++ escaping rules.

use super::{AppState, HttpResponse, LoginStatements};

/// Route prefix; C++ matches `strstr(path, "/bnetserver/browser/urlmap/") == path`.
pub(super) const BROWSER_URL_MAP_PATH: &str = "/bnetserver/browser/urlmap/";

pub(super) fn is_browser_url_map_path_like_cpp(path: &str) -> bool {
    path.starts_with(BROWSER_URL_MAP_PATH)
}

/// GET /bnetserver/browser/urlmap/ — host rewrites for the in-game browser shim.
///
/// C++ returns `{}` when the query yields no result (no rows or a failed query),
/// so a database error is logged and answered the same way instead of a 500.
pub(super) async fn get_browser_url_map(state: &AppState) -> HttpResponse {
    tracing::debug!("REST: GET {BROWSER_URL_MAP_PATH}");

    let stmt = state.login_db.prepare(LoginStatements::SEL_BROWSER_URL_MAP);
    let entries = match state.login_db.query(&stmt).await {
        Ok(mut result) => {
            let mut entries = Vec::new();
            if !result.is_empty() {
                loop {
                    let host: String = result.try_read::<String>(0).unwrap_or_default();
                    let target: String = result.try_read::<String>(1).unwrap_or_default();
                    entries.push((host, target));
                    if !result.next_row() {
                        break;
                    }
                }
            }
            entries
        }
        Err(error) => {
            tracing::error!("DB error loading browser_url_map: {error}");
            Vec::new()
        }
    };

    browser_url_map_response_like_cpp(&entries)
}

pub(super) fn browser_url_map_response_like_cpp(entries: &[(String, String)]) -> HttpResponse {
    HttpResponse {
        status_code: 200,
        status_text: "OK",
        headers: browser_url_map_headers_like_cpp(),
        body: browser_url_map_json_like_cpp(entries),
    }
}

/// The C++ soap server stamps every response with its JSON content-type plugin.
pub(super) fn browser_url_map_headers_like_cpp() -> Vec<(&'static str, String)> {
    vec![("Content-Type", "application/json;charset=utf-8".to_string())]
}

/// `{"host":"target",...}` in query order, values escaped like the C++ lambda.
pub(super) fn browser_url_map_json_like_cpp(entries: &[(String, String)]) -> String {
    let mut json = String::from("{");
    for (index, (host, target)) in entries.iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        json.push('"');
        json.push_str(&escape_browser_url_map_value_like_cpp(host));
        json.push_str("\":\"");
        json.push_str(&escape_browser_url_map_value_like_cpp(target));
        json.push('"');
    }
    json.push('}');
    json
}

/// C++ `escape`: prefix `"` and `\` with a backslash, drop control characters
/// (`< 0x20`) and pass everything else through unchanged.
pub(super) fn escape_browser_url_map_value_like_cpp(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    for c in input.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        if (c as u32) < 0x20 {
            continue;
        }
        out.push(c);
    }
    out
}
