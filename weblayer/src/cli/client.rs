use super::{CliError, CliResult};
use reqwest::{Client as HttpClient, Url};
use serde_json::Value;

pub(super) struct DaemonClient {
    origin: String,
    http: HttpClient,
}

impl DaemonClient {
    pub(super) fn new(origin: String) -> Self {
        Self {
            origin,
            http: HttpClient::new(),
        }
    }

    pub(super) fn origin(&self) -> &str {
        &self.origin
    }

    pub(super) async fn get_json(&self, path: &str, query: &[(&str, String)]) -> CliResult<Value> {
        let mut url = self.endpoint(path)?;
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }

        self.json_response(self.http.get(url).send().await?).await
    }

    pub(super) async fn post_json(
        &self,
        path: &str,
        query: &[(&str, String)],
        body: Value,
    ) -> CliResult<Value> {
        let mut url = self.endpoint(path)?;
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }

        self.json_response(self.http.post(url).json(&body).send().await?)
            .await
    }

    async fn json_response(&self, response: reqwest::Response) -> CliResult<Value> {
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(CliError::message(format!(
                "daemon returned HTTP {status}: {}",
                error_body_message(&text)
            )));
        }

        Ok(serde_json::from_str(&text)?)
    }

    fn endpoint(&self, path: &str) -> CliResult<Url> {
        let mut url =
            Url::parse(&self.origin).map_err(|error| CliError::message(error.to_string()))?;
        url.set_path(path);
        url.set_query(None);
        Ok(url)
    }
}

pub(super) fn normalize_origin(origin: String) -> CliResult<String> {
    let url = Url::parse(origin.trim().trim_end_matches('/'))
        .map_err(|error| CliError::message(error.to_string()))?;
    if url.scheme() != "http" {
        return Err(CliError::message("daemon origin must use http"));
    }
    if url.host_str().is_none() {
        return Err(CliError::message("daemon origin must include a host"));
    }

    Ok(url.origin().ascii_serialization())
}

pub(super) fn push_optional_query(
    query: &mut Vec<(&str, String)>,
    key: &'static str,
    value: Option<String>,
) {
    if let Some(value) = value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        query.push((key, value));
    }
}

fn error_body_message(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| text.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_origin_strips_paths_and_trailing_slashes() {
        assert_eq!(
            normalize_origin("http://127.0.0.1:17891/path/".into()).unwrap(),
            "http://127.0.0.1:17891"
        );
    }
}
