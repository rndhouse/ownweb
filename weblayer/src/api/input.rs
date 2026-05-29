use super::error::ApiError;

pub(super) fn clean_query_value(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(super) fn required_text(value: String, field: &str) -> Result<String, ApiError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(ApiError::bad_request(format!("{field} must not be empty")));
    }

    Ok(value)
}

pub(super) fn validate_confidence(confidence: Option<f64>) -> Result<Option<f64>, ApiError> {
    let Some(confidence) = confidence else {
        return Ok(None);
    };

    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        return Err(ApiError::bad_request(
            "confidence must be between 0.0 and 1.0",
        ));
    }

    Ok(Some(confidence))
}
