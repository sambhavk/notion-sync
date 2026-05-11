mod md_blocks;
mod notion;
mod state;
mod yaml_blocks;

use notion::NotionClient;
use sha2::{Digest, Sha256};
use state::{FileEntry, State};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: notion_sync <local_dir> <notion_page_id>");
        std::process::exit(1);
    }
    let root = PathBuf::from(&args[1]);
    let root_notion_page = &args[2];

    // state file always lives inside the notion_sync project dir (3 levels up from binary)
    // binary: notion_sync/target/release/notion_sync → notion_sync/
    let state_path = std::env::current_exe()
        .expect("cannot resolve binary path")
        .parent().expect("no parent").to_path_buf() // target/release/
        .parent().expect("no parent").to_path_buf() // target/
        .parent().expect("no parent").to_path_buf() // notion_sync/
        .join(".sync_state.json");

    let client = NotionClient::new();
    let mut st = state::load(&state_path);

    // collect all current dirs and files on disk (relative paths, forward slashes)
    let mut disk_dirs: Vec<(String, PathBuf)> = Vec::new(); // (rel_path, abs_path)
    let mut disk_files: Vec<(String, PathBuf)> = Vec::new();

    for entry in WalkDir::new(&root).min_depth(1).sort_by_file_name() {
        let entry = entry.expect("walkdir error");
        let abs = entry.path().to_path_buf();
        let rel = to_rel(&root, &abs);

        // skip hidden entries and the notion_sync subdirectory
        let entry_name = abs.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if entry_name.starts_with('.') || rel.starts_with("notion_sync") {
            continue;
        }

        if abs.is_dir() {
            disk_dirs.push((rel, abs));
        } else if abs.is_file() {
            if is_supported(&abs) {
                disk_files.push((rel, abs));
            } else {
                println!("Skipping unsupported file type: {rel}");
            }
        }
    }

    let disk_dir_keys: HashSet<String> = disk_dirs.iter().map(|(r, _)| r.clone()).collect();
    let disk_file_keys: HashSet<String> = disk_files.iter().map(|(r, _)| r.clone()).collect();

    // purge deleted dirs (cascade children)
    let deleted_dirs: Vec<String> = st
        .dirs
        .keys()
        .filter(|k| !disk_dir_keys.contains(*k))
        .cloned()
        .collect();

    for dir_rel in &deleted_dirs {
        println!("Archiving deleted dir: {dir_rel}");
        let page_id = st.dirs[dir_rel].clone();
        client.archive_page(&page_id).await;

        // cascade: archive all state entries under this dir
        let prefix = format!("{dir_rel}/");
        let child_dir_keys: Vec<String> = st
            .dirs
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        for child in child_dir_keys {
            let id = st.dirs[&child].clone();
            client.archive_page(&id).await;
            st.dirs.remove(&child);
        }
        let child_file_keys: Vec<String> = st
            .files
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        for child in child_file_keys {
            let id = st.files[&child].page_id.clone();
            client.archive_page(&id).await;
            st.files.remove(&child);
        }

        st.dirs.remove(dir_rel);
    }

    // purge deleted files
    let deleted_files: Vec<String> = st
        .files
        .keys()
        .filter(|k| !disk_file_keys.contains(*k))
        .cloned()
        .collect();

    for file_rel in &deleted_files {
        println!("Archiving deleted file: {file_rel}");
        let page_id = st.files[file_rel].page_id.clone();
        client.archive_page(&page_id).await;
        st.files.remove(file_rel);
    }

    // sync dirs top-down (walkdir already gives BFS/depth-first order)
    for (rel, abs) in &disk_dirs {
        let parent_id = parent_notion_id(&rel, &st, root_notion_page);
        if !st.dirs.contains_key(rel) {
            let title = abs
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            println!("Creating dir page: {rel}");
            let page_id = client.create_page(&parent_id, &title).await;
            st.dirs.insert(rel.clone(), page_id);
        }
    }

    // sync files
    for (rel, abs) in &disk_files {
        if is_image(abs) {
            let bytes = match std::fs::read(abs) {
                Ok(b) => b,
                Err(e) => { eprintln!("Cannot read {rel}: {e}"); continue; }
            };
            let hash = sha256_bytes(&bytes);
            let title = abs.file_name().unwrap_or_default().to_string_lossy().to_string();

            if let Some(entry) = st.files.get(rel) {
                if entry.hash == hash {
                    println!("Unchanged: {rel}");
                    continue;
                }
                println!("Updating image: {rel}");
                client.clear_page(&entry.page_id).await;
                let block = make_image_block(&client, abs, &title).await;
                client.append_blocks(&entry.page_id, vec![block]).await;
                st.files.get_mut(rel).unwrap().hash = hash;
            } else {
                let parent_id = parent_notion_id(rel, &st, root_notion_page);
                println!("Creating image page: {rel}");
                let page_id = client.create_page(&parent_id, &title).await;
                let block = make_image_block(&client, abs, &title).await;
                client.append_blocks(&page_id, vec![block]).await;
                st.files.insert(rel.clone(), FileEntry { page_id, hash });
            }
        } else {
            let content = match std::fs::read_to_string(abs) {
                Ok(c) => c,
                Err(e) => { eprintln!("Cannot read {rel}: {e}"); continue; }
            };
            let hash = sha256(&content);

            if let Some(entry) = st.files.get(rel) {
                if entry.hash == hash {
                    println!("Unchanged: {rel}");
                    continue;
                }
                println!("Updating: {rel}");
                client.clear_page(&entry.page_id).await;
                let blocks = render_file(abs, &content);
                client.append_blocks(&entry.page_id, blocks).await;
                st.files.get_mut(rel).unwrap().hash = hash;
            } else {
                let parent_id = parent_notion_id(rel, &st, root_notion_page);
                let title = abs.file_name().unwrap_or_default().to_string_lossy().to_string();
                println!("Creating file page: {rel}");
                let page_id = client.create_page(&parent_id, &title).await;
                let blocks = render_file(abs, &content);
                client.append_blocks(&page_id, blocks).await;
                st.files.insert(rel.clone(), FileEntry { page_id, hash });
            }
        }
    }

    state::save(&state_path, &st);
    println!("Sync complete.");
}

fn is_supported(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
    name.ends_with(".md")
        || name.ends_with(".md.yaml")
        || name.ends_with(".png")
        || name.ends_with(".jpg")
        || name.ends_with(".jpeg")
        || name.ends_with(".gif")
        || name.ends_with(".webp")
}

fn is_image(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
    name.ends_with(".png")
        || name.ends_with(".jpg")
        || name.ends_with(".jpeg")
        || name.ends_with(".gif")
        || name.ends_with(".webp")
}

fn render_file(path: &Path, content: &str) -> Vec<serde_json::Value> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.ends_with(".md.yaml") {
        yaml_blocks::parse(content)
    } else {
        md_blocks::parse(content)
    }
}

fn parent_notion_id(rel: &str, st: &State, root_page: &str) -> String {
    let parts: Vec<&str> = rel.split('/').collect();
    if parts.len() <= 1 {
        return root_page.to_string();
    }
    for end in (1..parts.len()).rev() {
        let parent_rel = parts[..end].join("/");
        if let Some(id) = st.dirs.get(&parent_rel) {
            return id.clone();
        }
    }
    root_page.to_string()
}

fn to_rel(root: &Path, abs: &Path) -> String {
    abs.strip_prefix(root)
        .unwrap_or(abs)
        .to_string_lossy()
        .replace('\\', "/")
}

fn sha256(content: &str) -> String {
    sha256_bytes(content.as_bytes())
}

fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

async fn make_image_block(client: &NotionClient, path: &Path, name: &str) -> serde_json::Value {
    match client.upload_image(path).await {
        Some(file_id) => serde_json::json!({
            "type": "image",
            "image": {
                "type": "file_upload",
                "file_upload": { "id": file_id }
            }
        }),
        None => {
            eprintln!("Image upload failed for {name}, inserting placeholder");
            serde_json::json!({
                "type": "paragraph",
                "paragraph": {
                    "rich_text": [{ "type": "text", "text": { "content": format!("[Image: {name} — upload failed]") } }]
                }
            })
        }
    }
}
