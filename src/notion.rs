use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::sleep;

const BASE: &str = "https://api.notion.com/v1";
const NOTION_VERSION: &str = "2022-06-28";

pub struct NotionClient {
    client: Client,
}

impl NotionClient {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    async fn request(&self, method: reqwest::Method, url: &str, body: Option<Value>) -> Value {
        loop {
            let token = std::env::var("NOTION_TOKEN").expect("NOTION_TOKEN env var not set");
            let mut req = self
                .client
                .request(method.clone(), url)
                .header("Authorization", format!("Bearer {}", token))
                .header("Notion-Version", NOTION_VERSION)
                .header("Content-Type", "application/json");

            if let Some(ref b) = body {
                req = req.json(b);
            }

            let resp = req.send().await.expect("network error");

            let status_code = resp.status().as_u16();
            if status_code == 429 || (status_code >= 500 && status_code <= 599) {
                eprintln!("Notion API {status_code}, retrying in 2s...");
                sleep(Duration::from_millis(2000)).await;
                continue;
            }

            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                eprintln!("Notion API error {status}: {text}");
                panic!("Notion API call failed");
            }

            sleep(Duration::from_millis(340)).await;
            return serde_json::from_str(&text).unwrap_or(Value::Null);
        }
    }

    pub async fn create_page(&self, parent_id: &str, title: &str) -> String {
        let body = json!({
            "parent": { "type": "page_id", "page_id": parent_id },
            "properties": {
                "title": {
                    "title": [{ "type": "text", "text": { "content": title } }]
                }
            }
        });
        let resp = self
            .request(reqwest::Method::POST, &format!("{BASE}/pages"), Some(body))
            .await;
        resp["id"].as_str().expect("no id in create_page response").to_string()
    }

    pub async fn get_child_block_ids(&self, block_id: &str) -> Vec<String> {
        let mut ids = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let url = match &cursor {
                None => format!("{BASE}/blocks/{block_id}/children?page_size=100"),
                Some(c) => format!("{BASE}/blocks/{block_id}/children?page_size=100&start_cursor={c}"),
            };
            let resp = self.request(reqwest::Method::GET, &url, None).await;
            if let Some(results) = resp["results"].as_array() {
                for block in results {
                    if let Some(id) = block["id"].as_str() {
                        ids.push(id.to_string());
                    }
                }
            }
            if resp["has_more"].as_bool().unwrap_or(false) {
                cursor = resp["next_cursor"].as_str().map(|s| s.to_string());
            } else {
                break;
            }
        }
        ids
    }

    pub async fn delete_block(&self, block_id: &str) {
        let url = format!("{BASE}/blocks/{block_id}");
        self.request(reqwest::Method::DELETE, &url, None).await;
    }

    pub async fn clear_page(&self, page_id: &str) {
        let ids = self.get_child_block_ids(page_id).await;
        for id in ids {
            self.delete_block(&id).await;
        }
    }

    pub async fn append_blocks(&self, block_id: &str, blocks: Vec<Value>) {
        for chunk in blocks.chunks(100) {
            let body = json!({ "children": chunk });
            let url = format!("{BASE}/blocks/{block_id}/children");
            self.request(reqwest::Method::PATCH, &url, Some(body)).await;
        }
    }

    pub async fn archive_page(&self, page_id: &str) {
        let url = format!("{BASE}/pages/{page_id}");
        let body = json!({ "archived": true });
        self.request(reqwest::Method::PATCH, &url, Some(body)).await;
    }
}
