//! Fixed-origin public package metadata requests with bounded bodies and no redirects.
use crate::{ServiceError, outbound, webhooks::delivery};
use rbit_collaboration::security::automation::dependencies::{Package, package_name};
use serde_json::{Value, json};
use std::time::Duration;

pub(super) struct AdvisoryClient {
    client: reqwest::Client,
}
impl AdvisoryClient {
    pub fn new() -> Result<Self, ServiceError> {
        Ok(Self { client: delivery::build_client(false, Duration::from_secs(15), false).map_err(ServiceError::internal)? })
    }
    async fn value(&self, request: reqwest::RequestBuilder) -> Result<Value, ServiceError> {
        let mut response = request.send().await.map_err(|_| unavailable())?;
        if !response.status().is_success() {
            return Err(unavailable());
        }
        let body = outbound::read_prefix(&mut response, 2 * 1024 * 1024).await.map_err(|_| unavailable())?;
        if body.truncated {
            return Err(unavailable());
        }
        serde_json::from_slice(&body.bytes).map_err(|_| unavailable())
    }
    pub async fn vulnerabilities(&self, packages: &[Package]) -> Result<Vec<Vec<String>>, ServiceError> {
        let mut results = Vec::new();
        for batch in packages.chunks(100) {
            let queries: Vec<_> =
                batch.iter().map(|p| json!({"package":{"ecosystem":"npm", "name":p.name}, "version":p.version})).collect();
            let value = self.value(self.client.post("https://api.osv.dev/v1/querybatch").json(&json!({"queries":queries}))).await?;
            results.extend(parse_batch(&value, batch.len())?);
        }
        Ok(results)
    }
    pub async fn advisory(&self, id: &str) -> Result<Value, ServiceError> {
        if !advisory_id(id) {
            return Err(unavailable());
        }
        self.value(self.client.get(format!("https://api.osv.dev/v1/vulns/{id}"))).await
    }
    pub async fn version(&self, name: &str, version: &str) -> Result<Value, ServiceError> {
        if !package_name(name) || version.len() > 100 || !version.bytes().all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b)) {
            return Err(unavailable());
        }
        let mut url = reqwest::Url::parse("https://registry.npmjs.org/").map_err(ServiceError::internal)?;
        url.path_segments_mut().map_err(|()| unavailable())?.push(name).push(version);
        self.value(self.client.get(url)).await
    }
}
fn advisory_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
fn unavailable() -> ServiceError {
    ServiceError::Unavailable { retry_after_seconds: 60 }
}
/// Missing, truncated or paginated provider results never attest a clean inventory.
fn parse_batch(value: &Value, expected: usize) -> Result<Vec<Vec<String>>, ServiceError> {
    let rows = value["results"].as_array().filter(|rows| rows.len() == expected).ok_or_else(unavailable)?;
    rows.iter()
        .map(|row| {
            if row.get("next_page_token").is_some() || row.as_object().is_some_and(|fields| fields.keys().any(|key| key != "vulns")) {
                return Err(unavailable());
            }
            let Some(vulns) = row.get("vulns") else {
                return if row.is_object() { Ok(Vec::new()) } else { Err(unavailable()) };
            };
            let vulns = vulns.as_array().filter(|v| v.len() <= 100).ok_or_else(unavailable)?;
            vulns.iter().map(|v| v["id"].as_str().filter(|id| advisory_id(id)).map(str::to_owned).ok_or_else(unavailable)).collect()
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_omission_and_pagination_fail_closed() {
        assert!(parse_batch(&json!({}), 1).is_err());
        assert!(parse_batch(&json!({"results":[{"next_page_token":"more"}]}), 1).is_err());
        assert!(parse_batch(&json!({"results":[{"vulns":[{"id":"../private"}]}]}), 1).is_err());
        assert_eq!(parse_batch(&json!({"results":[{}]}), 1).unwrap(), vec![Vec::<String>::new()]);
    }
    #[tokio::test]
    async fn provider_transport_rejects_redirects_oversized_and_invalid_bodies() {
        use axum::{
            Router,
            http::{StatusCode, header},
            routing::get,
        };
        let app = Router::new()
            .route("/valid", get(|| async { "{\"results\":[{}]}" }))
            .route("/invalid", get(|| async { "not JSON" }))
            .route("/large", get(|| async { "x".repeat(2 * 1024 * 1024 + 1) }))
            .route("/failure", get(|| async { StatusCode::SERVICE_UNAVAILABLE }))
            .route("/redirect", get(|| async { (StatusCode::FOUND, [(header::LOCATION, "/valid")]) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        // Only this test client can reach the local fixture; production origins stay fixed.
        let client = AdvisoryClient { client: delivery::build_client(true, Duration::from_secs(2), false).unwrap() };
        for path in ["invalid", "large", "failure", "redirect"] {
            assert!(
                client.value(client.client.get(format!("http://{address}/{path}"))).await.is_err(),
                "{path} must not become complete evidence"
            );
        }
        let value = client.value(client.client.get(format!("http://{address}/valid"))).await.unwrap();
        assert_eq!(parse_batch(&value, 1).unwrap(), vec![Vec::<String>::new()]);
        server.abort();
        let _ = server.await;
    }
}
