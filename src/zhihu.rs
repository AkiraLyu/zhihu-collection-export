use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use regex::Regex;
use reqwest::{
    Client, StatusCode,
    header::{ACCEPT, ACCEPT_LANGUAGE, COOKIE, HeaderMap, HeaderValue, REFERER, USER_AGENT},
};
use serde::Deserialize;
use serde_json::Value;
use tokio::time::sleep;
use url::Url;

pub(crate) const ZHIHU_HOST: &str = "https://www.zhihu.com";
pub(crate) const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36";

#[derive(Debug, Deserialize)]
pub(crate) struct CollectionPage {
    #[serde(default)]
    pub(crate) data: Vec<CollectionItem>,
    pub(crate) paging: Option<Paging>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Paging {
    pub(crate) totals: Option<u64>,
    pub(crate) is_end: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CollectionItem {
    pub(crate) content: Option<Content>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Content {
    pub(crate) id: Option<Value>,
    #[serde(rename = "type")]
    pub(crate) kind: Option<String>,
    pub(crate) url: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) excerpt_title: Option<String>,
    pub(crate) content: Option<Value>,
    pub(crate) excerpt: Option<String>,
    pub(crate) question: Option<Question>,
    pub(crate) author: Option<Author>,
    #[serde(alias = "created")]
    pub(crate) created_time: Option<i64>,
    #[serde(alias = "updated")]
    pub(crate) updated_time: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Question {
    pub(crate) id: Option<Value>,
    pub(crate) title: Option<String>,
    pub(crate) url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Author {
    pub(crate) name: Option<String>,
    pub(crate) headline: Option<String>,
    pub(crate) url_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CollectionInfo {
    pub(crate) title: Option<String>,
    collection: Option<CollectionInfoBody>,
}

#[derive(Debug, Deserialize)]
struct CollectionInfoBody {
    pub(crate) title: Option<String>,
}

pub(crate) async fn fetch_page(
    client: &Client,
    collection_id: &str,
    offset: u32,
    limit: u32,
    retries: u32,
    delay_ms: u64,
) -> Result<CollectionPage> {
    let url = format!(
        "{ZHIHU_HOST}/api/v4/collections/{collection_id}/items?offset={offset}&limit={limit}"
    );
    fetch_json(client, &url, retries, delay_ms).await
}

pub(crate) async fn fetch_collection_title(client: &Client, collection_id: &str) -> Option<String> {
    let url = format!("{ZHIHU_HOST}/api/v4/collections/{collection_id}");
    let info = fetch_json::<CollectionInfo>(client, &url, 1, 300)
        .await
        .ok()?;
    let title = info
        .title
        .or_else(|| info.collection.and_then(|collection| collection.title))?
        .trim()
        .to_string();
    if title.is_empty() { None } else { Some(title) }
}

async fn fetch_json<T>(client: &Client, url: &str, retries: u32, delay_ms: u64) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let mut last_error = None;

    for attempt in 0..=retries {
        match client.get(url).send().await {
            Ok(response) => {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                if status.is_success() {
                    return serde_json::from_str(&text).with_context(|| {
                        format!("解析 JSON 失败，响应前 200 字符: {}", excerpt(&text, 200))
                    });
                }

                let error = anyhow!(
                    "HTTP {}，响应前 200 字符: {}",
                    status.as_u16(),
                    excerpt(&text, 200)
                );
                if should_retry(status) && attempt < retries {
                    last_error = Some(error);
                } else {
                    return Err(error);
                }
            }
            Err(error) => {
                if attempt < retries {
                    last_error = Some(anyhow!(error));
                } else {
                    return Err(error).context("请求失败");
                }
            }
        }

        let backoff = delay_ms.saturating_mul((attempt + 1) as u64).max(300);
        sleep(Duration::from_millis(backoff)).await;
    }

    Err(last_error.unwrap_or_else(|| anyhow!("请求失败")))
}

fn should_retry(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

pub(crate) fn zhihu_client(cookie_header: &str, collection_id: &str) -> Result<Client> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(BROWSER_USER_AGENT));
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/json, text/plain, */*"),
    );
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"),
    );
    headers.insert(
        REFERER,
        HeaderValue::from_str(&format!("{ZHIHU_HOST}/collection/{collection_id}"))?,
    );
    headers.insert(COOKIE, HeaderValue::from_str(cookie_header)?);

    Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(30))
        .build()
        .context("创建 HTTP client 失败")
}

pub(crate) fn parse_collection_id(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        return Ok(trimmed.to_string());
    }

    if let Ok(url) = Url::parse(trimmed) {
        for pair in url
            .path_segments()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .windows(2)
        {
            if pair[0] == "collection" && pair[1].chars().all(|ch| ch.is_ascii_digit()) {
                return Ok(pair[1].to_string());
            }
        }
    }

    let re = Regex::new(r"/collection/(\d+)").expect("valid regex");
    re.captures(trimmed)
        .and_then(|captures| captures.get(1))
        .map(|matched| matched.as_str().to_string())
        .ok_or_else(|| anyhow!("无法从参数中解析收藏夹 ID: {input}"))
}

pub(crate) fn normalize_zhihu_url(raw: &str) -> String {
    let raw = raw.trim();
    if raw.starts_with("//") {
        format!("https:{raw}")
    } else if raw.starts_with('/') {
        format!("{ZHIHU_HOST}{raw}")
    } else {
        raw.to_string()
    }
}

fn excerpt(input: &str, max_chars: usize) -> String {
    let cleaned = input.replace(['\n', '\r', '\t'], " ");
    let mut output = cleaned.chars().take(max_chars).collect::<String>();
    if cleaned.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_collection_id_from_url() {
        assert_eq!(
            parse_collection_id("https://www.zhihu.com/collection/997879559").unwrap(),
            "997879559"
        );
        assert_eq!(parse_collection_id("997879559").unwrap(), "997879559");
    }

    #[test]
    fn parses_nested_collection_title_response() {
        let info: CollectionInfo =
            serde_json::from_str(r#"{"collection":{"title":"Linux"}}"#).unwrap();
        assert_eq!(info.collection.unwrap().title.unwrap(), "Linux");
    }

    #[test]
    fn parses_pin_with_structured_content() {
        let page: CollectionPage = serde_json::from_str(
            r#"{
                "data": [{
                    "content": {
                        "id": "123",
                        "type": "pin",
                        "excerpt_title": "想法标题",
                        "content": [
                            {"type": "text", "content": "<p>想法正文</p>"},
                            {"type": "link_card", "url": "https://www.zhihu.com/question/1"}
                        ],
                        "created": 10,
                        "updated": 20
                    }
                }]
            }"#,
        )
        .unwrap();

        let content = page.data[0].content.as_ref().unwrap();
        assert_eq!(content.excerpt_title.as_deref(), Some("想法标题"));
        assert!(content.content.as_ref().unwrap().is_array());
        assert_eq!(content.created_time, Some(10));
        assert_eq!(content.updated_time, Some(20));
    }
}
