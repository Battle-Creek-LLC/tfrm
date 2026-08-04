//! One error type mapping onto the spec §1 exit-code table. Every CLI
//! error path funnels through this so exit codes stay consistent.

/// Errors carrying their spec §1 exit code.
///
/// | code | meaning |
/// |------|---------|
/// | 1 | unexpected error, or apply/discard ended in `errored` |
/// | 2 | usage error (including unresolvable workspace) |
/// | 3 | authentication/authorization failure |
/// | 4 | run, workspace, or plan not found |
/// | 6 | apply refused: run not confirmable (state, policy, queue) |
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Exit 2 — usage error, including an unresolvable workspace.
    #[error("{0}")]
    Usage(String),

    /// Exit 3 — authentication or authorization failure.
    #[error("{0}")]
    Auth(String),

    /// Exit 4 — run, workspace, or plan not found.
    #[error("{0}")]
    NotFound(String),

    /// Exit 6 — action refused: run not confirmable (state, policy, queue).
    #[error("{0}")]
    Refused(String),

    /// API error carrying the HTTP status and the TFC error detail (R8.3).
    /// Never constructed with request headers — the Authorization header
    /// must not appear in `detail`.
    #[error("HTTP {status}: {detail}")]
    Api { status: u16, detail: String },

    /// Exit 1 — anything unexpected.
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Build an API error from a response status and the TFC error detail
    /// (R8.3). Callers must pass only response-derived text, never request
    /// headers.
    pub fn api(status: u16, detail: impl Into<String>) -> Self {
        Error::Api {
            status,
            detail: detail.into(),
        }
    }

    /// The spec §1 exit code for this error.
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Usage(_) => 2,
            Error::Auth(_) => 3,
            Error::NotFound(_) => 4,
            Error::Refused(_) => 6,
            Error::Api { status, .. } => match status {
                401 | 403 => 3,
                404 => 4,
                409 => 6,
                _ => 1,
            },
            Error::Other(_) => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_exit_codes_match_the_spec_table() {
        assert_eq!(Error::Usage("u".into()).exit_code(), 2);
        assert_eq!(Error::Auth("a".into()).exit_code(), 3);
        assert_eq!(Error::NotFound("n".into()).exit_code(), 4);
        assert_eq!(Error::Refused("r".into()).exit_code(), 6);
        assert_eq!(Error::Other("o".into()).exit_code(), 1);
    }

    #[test]
    fn api_status_mapping() {
        assert_eq!(Error::api(401, "unauthorized").exit_code(), 3);
        assert_eq!(Error::api(403, "forbidden").exit_code(), 3);
        assert_eq!(Error::api(404, "not found").exit_code(), 4);
        assert_eq!(Error::api(409, "conflict").exit_code(), 6);
        assert_eq!(Error::api(500, "boom").exit_code(), 1);
        assert_eq!(Error::api(422, "unprocessable").exit_code(), 1);
    }

    #[test]
    fn api_display_includes_status_and_detail() {
        let e = Error::api(404, "run svc-xxx not found");
        assert_eq!(e.to_string(), "HTTP 404: run svc-xxx not found");
    }

    /// R8.3: an error built from a request that carried a bearer token must
    /// never leak that token through Display. Models the J1.2 client's
    /// conversion: only status + response body reach the error.
    #[test]
    fn error_from_authorized_request_omits_the_token() {
        struct FakeRequest {
            #[allow(dead_code)]
            authorization: String,
            status: u16,
            body_detail: String,
        }
        let req = FakeRequest {
            authorization: "Bearer super-secret-token-value".into(),
            status: 401,
            body_detail: "unauthorized".into(),
        };
        let err = Error::api(req.status, req.body_detail);
        let shown = err.to_string();
        assert!(!shown.contains("super-secret-token-value"), "{shown}");
        assert!(!shown.contains("Bearer"), "{shown}");
        let debug = format!("{err:?}");
        assert!(!debug.contains("super-secret-token-value"), "{debug}");
    }
}
