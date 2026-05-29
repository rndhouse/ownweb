use super::error::ApiError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SiteScope {
    XCom,
}

impl SiteScope {
    pub(super) fn from_param(site: Option<&str>) -> Result<Self, ApiError> {
        match site.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) if is_x_site(value) => Ok(Self::XCom),
            Some(value) => Err(ApiError::bad_request(format!(
                "unsupported site query parameter: {value}"
            ))),
            None => Err(ApiError::bad_request(
                "missing required site query parameter",
            )),
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::XCom => "x.com",
        }
    }
}

fn is_x_site(site: &str) -> bool {
    site.eq_ignore_ascii_case("x.com") || site.eq_ignore_ascii_case("twitter.com")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn site_scope_accepts_supported_site_query_values() {
        assert_eq!(
            SiteScope::from_param(Some("x.com")).unwrap(),
            SiteScope::XCom
        );
        assert_eq!(
            SiteScope::from_param(Some(" twitter.com ")).unwrap(),
            SiteScope::XCom
        );
    }

    #[test]
    fn site_scope_rejects_missing_site_query() {
        let error = SiteScope::from_param(None).unwrap_err();

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(error.message, "missing required site query parameter");
    }

    #[test]
    fn site_scope_rejects_unsupported_site_query() {
        let error = SiteScope::from_param(Some("example.com")).unwrap_err();

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(
            error.message,
            "unsupported site query parameter: example.com"
        );
    }
}
