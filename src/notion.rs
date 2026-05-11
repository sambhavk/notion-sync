use reqwest::Client;
use serde_json::{json, Value};
use std::path::Path;
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

    // Returns the file_upload ID on success, None on any failure.
    pub async fn upload_image(&self, path: &Path) -> Option<String> {
        let name = path.file_name()?.to_string_lossy().to_string();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        let content_type = match ext.as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "png"          => "image/png",
            "gif"          => "image/gif",
            "webp"         => "image/webp",
            _              => return None,
        };

        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => { eprintln!("Cannot read image {name}: {e}"); return None; }
        };

        let token = std::env::var("NOTION_TOKEN").ok()?;

        // Step 1 — create the file upload slot
        let create_resp = self.client
            .post(format!("{BASE}/file_uploads"))
            .header("Authorization", format!("Bearer {token}"))
            .header("Notion-Version", NOTION_VERSION)
            .header("Content-Type", "application/json")
            .json(&json!({ "name": name, "content_type": content_type }))
            .send()
            .await
            .ok()?;

        if !create_resp.status().is_success() {
            eprintln!("Image upload slot failed ({}): {}", create_resp.status(), create_resp.text().await.unwrap_or_default());
            return None;
        }

        let meta: Value = create_resp.json().await.ok()?;
        let upload_url = meta["upload_url"].as_str()?.to_string();
        let file_id    = meta["id"].as_str()?.to_string();

        // Step 2 — POST multipart to the /send endpoint (Notion API, needs auth)
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(name.clone())
            .mime_str(content_type)
            .ok()?;
        let form = reqwest::multipart::Form::new().part("file", part);

        let send_resp = self.client
            .post(&upload_url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Notion-Version", NOTION_VERSION)
            .multipart(form)
            .send()
            .await
            .ok()?;

        if !send_resp.status().is_success() {
            let body = send_resp.text().await.unwrap_or_default();
            eprintln!("Image upload /send failed: {body}");
            return None;
        }

        sleep(Duration::from_millis(340)).await;
        Some(file_id)
    }
}
