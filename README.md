# notion-sync

Syncs a local folder to a Notion page nightly. Written in Rust.

## Supported file types

- `.md` — rendered as Notion blocks (headings, lists, tables, code blocks, inline formatting)
- `.md.yaml` — parsed as structured schema files; rendered as Notion headings, key-value pairs, and sections

All other file types are skipped.

## Build

```bash
cargo build --release
```

## Usage

```bash
NOTION_TOKEN=<your_integration_token> ./target/release/notion_sync <local_dir> <notion_page_id>
```

- `local_dir` — absolute path to the folder to sync
- `notion_page_id` — ID of the Notion page to sync into (share it with your integration first)

State is tracked in `<local_dir>/.notion_sync_state.json` (gitignored). On each run, only changed files make API calls.

## Nightly cron

```
0 0 * * * NOTION_TOKEN=<token> /path/to/notion_sync /path/to/local_dir <notion_page_id> >> /tmp/notion_sync.log 2>&1
```

## What gets synced

| Event | Behaviour |
|-------|-----------|
| New file/folder | Creates Notion page under the correct parent |
| File changed | Clears and re-renders the page |
| File unchanged | Skipped — no API calls |
| File/folder deleted | Notion page archived |
| Unsupported file type | Skipped with a log line |
