use async_trait::async_trait;
use gclaw_core::traits::Tool;
use gclaw_core::types::{ToolDefinition, ToolResult};
use gclaw_core::Result;
use serde_json::json;
use std::path::{Path, PathBuf};

/// Expand a leading `~` to the user's home directory.
fn expand_home(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if p == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(p)
}

// === ShellExecTool === (keep existing)

pub struct ShellExecTool;

#[async_trait]
impl Tool for ShellExecTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "shell_exec".to_string(),
            description: "Execute a shell command and return its output.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute"
                    }
                },
                "required": ["command"]
            }),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolResult> {
        let command = input
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                gclaw_core::GclawError::ToolExecution("Missing 'command' parameter".to_string())
            })?;

        match tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .await
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let content = if stderr.is_empty() {
                    stdout.to_string()
                } else {
                    format!("{stdout}\nSTDERR:\n{stderr}")
                };
                Ok(ToolResult {
                    content,
                    is_error: !output.status.success(),
                })
            }
            Err(e) => Ok(ToolResult {
                content: format!("Failed to execute command: {e}"),
                is_error: true,
            }),
        }
    }
}

// === FileReadTool ===

const FILE_READ_DEFAULT_LIMIT: usize = 2000;
const FILE_READ_MAX_LINE_LEN: usize = 2000;

pub struct FileReadTool;

#[async_trait]
impl Tool for FileReadTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "file_read".to_string(),
            description: "Read a file from the local filesystem. Returns content prefixed with line numbers (1-indexed) so it can be referenced by other tools. By default reads up to 2000 lines from the start of the file; use 'offset' and 'limit' to page through larger files. Long lines are truncated to 2000 chars.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative path to the file. Supports ~ for home directory."
                    },
                    "offset": {
                        "type": "integer",
                        "description": "1-indexed line number to start reading from (default: 1).",
                        "minimum": 1
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of lines to return (default: 2000).",
                        "minimum": 1
                    }
                },
                "required": ["path"]
            }),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolResult> {
        let path = input.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            gclaw_core::GclawError::ToolExecution("Missing 'path' parameter".to_string())
        })?;
        let path = expand_home(path);

        let offset = input
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|n| n.max(1) as usize)
            .unwrap_or(1);
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n.max(1) as usize)
            .unwrap_or(FILE_READ_DEFAULT_LIMIT);

        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult {
                    content: format!("Failed to read file: {e}"),
                    is_error: true,
                })
            }
        };

        if content.is_empty() {
            return Ok(ToolResult {
                content: format!("(file is empty: {})", path.display()),
                is_error: false,
            });
        }

        let lines: Vec<&str> = content.split('\n').collect();
        let total = lines.len();
        let start_idx = offset.saturating_sub(1).min(total);
        let end_idx = start_idx.saturating_add(limit).min(total);

        let mut out = String::with_capacity(content.len().min(64 * 1024));
        for (i, line) in lines[start_idx..end_idx].iter().enumerate() {
            let lineno = start_idx + i + 1;
            let truncated = if line.len() > FILE_READ_MAX_LINE_LEN {
                format!(
                    "{}…[line truncated, {} bytes]",
                    &line[..FILE_READ_MAX_LINE_LEN],
                    line.len()
                )
            } else {
                (*line).to_string()
            };
            out.push_str(&format!("{lineno:>6}\t{truncated}\n"));
        }
        if end_idx < total {
            out.push_str(&format!(
                "[showing lines {}-{} of {}; use offset to read more]\n",
                start_idx + 1,
                end_idx,
                total
            ));
        }

        Ok(ToolResult {
            content: out,
            is_error: false,
        })
    }
}

// === FileWriteTool ===

pub struct FileWriteTool;

#[async_trait]
impl Tool for FileWriteTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "file_write".to_string(),
            description: "Write content to a file. Creates the file if it doesn't exist, overwrites if it does.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to write"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file"
                    }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolResult> {
        let path = input.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            gclaw_core::GclawError::ToolExecution("Missing 'path' parameter".to_string())
        })?;
        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                gclaw_core::GclawError::ToolExecution("Missing 'content' parameter".to_string())
            })?;
        let path = expand_home(path);

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    return Ok(ToolResult {
                        content: format!("Failed to create directories: {e}"),
                        is_error: true,
                    });
                }
            }
        }

        match tokio::fs::write(&path, content).await {
            Ok(()) => Ok(ToolResult {
                content: format!("Wrote {} bytes to {}", content.len(), path.display()),
                is_error: false,
            }),
            Err(e) => Ok(ToolResult {
                content: format!("Failed to write file: {e}"),
                is_error: true,
            }),
        }
    }
}

// === WebFetchTool ===
// Fetches a URL and returns its text content.

pub struct WebFetchTool {
    client: reqwest::Client,
}

impl WebFetchTool {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "web_fetch".to_string(),
            description: "Fetch the content of a URL. Returns the response body as text."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to fetch"
                    }
                },
                "required": ["url"]
            }),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolResult> {
        let url = input.get("url").and_then(|v| v.as_str()).ok_or_else(|| {
            gclaw_core::GclawError::ToolExecution("Missing 'url' parameter".to_string())
        })?;

        match self.client.get(url).send().await {
            Ok(resp) => {
                let status = resp.status();
                match resp.text().await {
                    Ok(body) => {
                        // Truncate to 50KB to avoid blowing up context
                        let truncated = if body.len() > 50_000 {
                            format!(
                                "{}...\n[truncated, {} total bytes]",
                                &body[..50_000],
                                body.len()
                            )
                        } else {
                            body
                        };
                        Ok(ToolResult {
                            content: format!("HTTP {status}\n\n{truncated}"),
                            is_error: !status.is_success(),
                        })
                    }
                    Err(e) => Ok(ToolResult {
                        content: format!("Failed to read response body: {e}"),
                        is_error: true,
                    }),
                }
            }
            Err(e) => Ok(ToolResult {
                content: format!("Request failed: {e}"),
                is_error: true,
            }),
        }
    }
}

// === ListDirTool ===

pub struct ListDirTool;

#[async_trait]
impl Tool for ListDirTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_dir".to_string(),
            description: "List the contents of a directory.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the directory (default: current directory)"
                    }
                },
                "required": []
            }),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolResult> {
        let path_in = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let path = expand_home(path_in);

        match tokio::fs::read_dir(&path).await {
            Ok(mut entries) => {
                let mut items = Vec::new();
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let meta = entry.metadata().await;
                    let suffix = match meta {
                        Ok(m) if m.is_dir() => "/",
                        Ok(m) if m.is_symlink() => "@",
                        _ => "",
                    };
                    items.push(format!("{name}{suffix}"));
                }
                items.sort();
                Ok(ToolResult {
                    content: items.join("\n"),
                    is_error: false,
                })
            }
            Err(e) => Ok(ToolResult {
                content: format!("Failed to list directory: {e}"),
                is_error: true,
            }),
        }
    }
}

// === FileEditTool ===
// Performs an exact string replacement within a single file.

pub struct FileEditTool;

#[async_trait]
impl Tool for FileEditTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "file_edit".to_string(),
            description: "Make an exact string replacement in a file. By default fails if 'old_string' appears more than once — pass enough surrounding context to make it unique, or set 'replace_all' to true. Preserves file formatting; use this instead of file_write to modify part of an existing file.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to edit. Supports ~ for home directory."
                    },
                    "old_string": {
                        "type": "string",
                        "description": "Exact text to find. Must match the file byte-for-byte (including indentation)."
                    },
                    "new_string": {
                        "type": "string",
                        "description": "Replacement text. Must differ from old_string."
                    },
                    "replace_all": {
                        "type": "boolean",
                        "description": "Replace every occurrence (default: false)."
                    }
                },
                "required": ["path", "old_string", "new_string"]
            }),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolResult> {
        let path = input.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            gclaw_core::GclawError::ToolExecution("Missing 'path' parameter".to_string())
        })?;
        let old_string = input
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                gclaw_core::GclawError::ToolExecution("Missing 'old_string' parameter".to_string())
            })?;
        let new_string = input
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                gclaw_core::GclawError::ToolExecution("Missing 'new_string' parameter".to_string())
            })?;
        let replace_all = input
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if old_string == new_string {
            return Ok(ToolResult {
                content: "old_string and new_string are identical — nothing to do".to_string(),
                is_error: true,
            });
        }
        if old_string.is_empty() {
            return Ok(ToolResult {
                content: "old_string must not be empty".to_string(),
                is_error: true,
            });
        }

        let path = expand_home(path);
        let original = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult {
                    content: format!("Failed to read file: {e}"),
                    is_error: true,
                })
            }
        };

        let count = original.matches(old_string).count();
        if count == 0 {
            return Ok(ToolResult {
                content: format!("old_string not found in {}", path.display()),
                is_error: true,
            });
        }
        if count > 1 && !replace_all {
            return Ok(ToolResult {
                content: format!(
                    "old_string is not unique in {} ({count} matches). Add surrounding context to make it unique, or set replace_all=true.",
                    path.display()
                ),
                is_error: true,
            });
        }

        let updated = if replace_all {
            original.replace(old_string, new_string)
        } else {
            original.replacen(old_string, new_string, 1)
        };

        match tokio::fs::write(&path, &updated).await {
            Ok(()) => Ok(ToolResult {
                content: format!(
                    "Edited {} ({} replacement{})",
                    path.display(),
                    count.min(if replace_all { count } else { 1 }),
                    if replace_all && count > 1 { "s" } else { "" }
                ),
                is_error: false,
            }),
            Err(e) => Ok(ToolResult {
                content: format!("Failed to write file: {e}"),
                is_error: true,
            }),
        }
    }
}

// === GlobTool ===
// Find files matching a glob pattern, sorted by most-recently-modified first.

pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "glob".to_string(),
            description: "Find files by glob pattern (e.g. '**/*.rs', 'src/**/*.toml'). Returns matching paths sorted by modification time (newest first). Use this instead of shell 'find' for fast file discovery.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern. Supports **, *, ?, and [..] character classes."
                    },
                    "path": {
                        "type": "string",
                        "description": "Base directory to search from (default: current directory)."
                    }
                },
                "required": ["pattern"]
            }),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolResult> {
        let pattern = input
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                gclaw_core::GclawError::ToolExecution("Missing 'pattern' parameter".to_string())
            })?;
        let base = input
            .get("path")
            .and_then(|v| v.as_str())
            .map(expand_home)
            .unwrap_or_else(|| PathBuf::from("."));

        let pattern_owned = pattern.to_string();
        let result = tokio::task::spawn_blocking(move || run_glob(&base, &pattern_owned))
            .await
            .map_err(|e| gclaw_core::GclawError::ToolExecution(format!("glob task failed: {e}")))?;

        match result {
            Ok(matches) => {
                if matches.is_empty() {
                    Ok(ToolResult {
                        content: "(no matches)".to_string(),
                        is_error: false,
                    })
                } else {
                    let body = matches
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join("\n");
                    Ok(ToolResult {
                        content: body,
                        is_error: false,
                    })
                }
            }
            Err(e) => Ok(ToolResult {
                content: format!("Glob failed: {e}"),
                is_error: true,
            }),
        }
    }
}

fn run_glob(base: &Path, pattern: &str) -> std::result::Result<Vec<PathBuf>, String> {
    // If the pattern is absolute, use it as-is; otherwise join under base.
    let full = if Path::new(pattern).is_absolute() {
        pattern.to_string()
    } else {
        base.join(pattern).to_string_lossy().into_owned()
    };

    let entries = glob::glob(&full).map_err(|e| e.to_string())?;
    let mut items: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    for entry in entries.flatten().take(1000) {
        let mtime = std::fs::metadata(&entry)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        items.push((entry, mtime));
    }
    items.sort_by(|a, b| b.1.cmp(&a.1));
    Ok(items.into_iter().map(|(p, _)| p).collect())
}

// === GrepTool ===
// Regex content search across files (recursive).

pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "grep".to_string(),
            description: "Search file contents using a regular expression. Recursive by default. Returns 'path:line:match' lines. Use this for code search instead of shell 'grep'.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Rust-style regular expression to search for."
                    },
                    "path": {
                        "type": "string",
                        "description": "File or directory to search (default: current directory)."
                    },
                    "glob": {
                        "type": "string",
                        "description": "Optional filename glob filter (e.g. '*.rs')."
                    },
                    "case_insensitive": {
                        "type": "boolean",
                        "description": "Case-insensitive match (default: false)."
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum number of matching lines to return (default: 200).",
                        "minimum": 1
                    }
                },
                "required": ["pattern"]
            }),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolResult> {
        let pattern = input
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                gclaw_core::GclawError::ToolExecution("Missing 'pattern' parameter".to_string())
            })?;
        let base = input
            .get("path")
            .and_then(|v| v.as_str())
            .map(expand_home)
            .unwrap_or_else(|| PathBuf::from("."));
        let case_insensitive = input
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| n.max(1) as usize)
            .unwrap_or(200);
        let name_glob = input
            .get("glob")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        let pattern_owned = pattern.to_string();
        let result = tokio::task::spawn_blocking(move || {
            run_grep(
                &base,
                &pattern_owned,
                name_glob.as_deref(),
                case_insensitive,
                max_results,
            )
        })
        .await
        .map_err(|e| gclaw_core::GclawError::ToolExecution(format!("grep task failed: {e}")))?;

        match result {
            Ok((lines, total, capped)) => {
                let header = if capped {
                    format!(
                        "[{} matches shown of {}+ — refine with glob/path/max_results]\n",
                        lines.len(),
                        total
                    )
                } else if lines.is_empty() {
                    "(no matches)".to_string()
                } else {
                    format!(
                        "[{} match{}]\n",
                        lines.len(),
                        if lines.len() == 1 { "" } else { "es" }
                    )
                };
                Ok(ToolResult {
                    content: format!("{header}{}", lines.join("\n")),
                    is_error: false,
                })
            }
            Err(e) => Ok(ToolResult {
                content: format!("Grep failed: {e}"),
                is_error: true,
            }),
        }
    }
}

fn run_grep(
    base: &Path,
    pattern: &str,
    name_glob: Option<&str>,
    case_insensitive: bool,
    max_results: usize,
) -> std::result::Result<(Vec<String>, usize, bool), String> {
    let regex = regex::RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|e| format!("invalid regex: {e}"))?;
    let glob_pat = name_glob
        .map(|p| glob::Pattern::new(p).map_err(|e| format!("invalid glob: {e}")))
        .transpose()?;

    let mut out: Vec<String> = Vec::new();
    let mut total: usize = 0;
    let mut capped = false;

    let walker = walkdir::WalkDir::new(base)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            // Skip common heavy/uninteresting directories.
            let name = e.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                ".git" | "node_modules" | "target" | ".venv" | "dist" | "build" | "__pycache__"
            )
        });

    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        if let Some(g) = &glob_pat {
            let fname = entry.file_name().to_string_lossy();
            if !g.matches(&fname) {
                continue;
            }
        }

        let path = entry.path();
        // Read file as bytes; skip on error or if not valid UTF-8 (binary files).
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let text = match std::str::from_utf8(&bytes) {
            Ok(s) => s,
            Err(_) => continue,
        };

        for (i, line) in text.lines().enumerate() {
            if regex.is_match(line) {
                total += 1;
                if out.len() >= max_results {
                    capped = true;
                    continue;
                }
                let display = if line.len() > 400 {
                    format!("{}…", &line[..400])
                } else {
                    line.to_string()
                };
                out.push(format!("{}:{}:{}", path.display(), i + 1, display));
            }
        }
    }

    Ok((out, total, capped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[tokio::test]
    async fn file_read_returns_line_numbers() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        tokio::fs::write(&path, "alpha\nbeta\ngamma\n")
            .await
            .unwrap();

        let result = FileReadTool
            .execute(json!({"path": path.to_str().unwrap()}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("     1\talpha"));
        assert!(result.content.contains("     2\tbeta"));
        assert!(result.content.contains("     3\tgamma"));
    }

    #[tokio::test]
    async fn file_read_respects_offset_and_limit() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nums.txt");
        let body = (1..=10)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        tokio::fs::write(&path, body).await.unwrap();

        let result = FileReadTool
            .execute(json!({"path": path.to_str().unwrap(), "offset": 4, "limit": 2}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("     4\t4"));
        assert!(result.content.contains("     5\t5"));
        assert!(!result.content.contains("     3\t3"));
        assert!(!result.content.contains("     6\t6"));
        assert!(result.content.contains("[showing lines 4-5"));
    }

    #[tokio::test]
    async fn file_edit_replaces_unique_match() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.txt");
        tokio::fs::write(&path, "hello world").await.unwrap();

        let result = FileEditTool
            .execute(json!({
                "path": path.to_str().unwrap(),
                "old_string": "world",
                "new_string": "rust",
            }))
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.content);
        let after = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(after, "hello rust");
    }

    #[tokio::test]
    async fn file_edit_rejects_ambiguous_match() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.txt");
        tokio::fs::write(&path, "x x x").await.unwrap();

        let result = FileEditTool
            .execute(json!({
                "path": path.to_str().unwrap(),
                "old_string": "x",
                "new_string": "y",
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not unique"));

        // replace_all bypasses uniqueness check
        let result = FileEditTool
            .execute(json!({
                "path": path.to_str().unwrap(),
                "old_string": "x",
                "new_string": "y",
                "replace_all": true,
            }))
            .await
            .unwrap();
        assert!(!result.is_error);
        let after = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(after, "y y y");
    }

    #[tokio::test]
    async fn file_edit_errors_when_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.txt");
        tokio::fs::write(&path, "hello").await.unwrap();

        let result = FileEditTool
            .execute(json!({
                "path": path.to_str().unwrap(),
                "old_string": "missing",
                "new_string": "x",
            }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn glob_finds_matching_files() {
        let dir = tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.rs"), "").await.unwrap();
        tokio::fs::write(dir.path().join("b.rs"), "").await.unwrap();
        tokio::fs::write(dir.path().join("c.txt"), "")
            .await
            .unwrap();

        let result = GlobTool
            .execute(json!({"pattern": "*.rs", "path": dir.path().to_str().unwrap()}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("a.rs"));
        assert!(result.content.contains("b.rs"));
        assert!(!result.content.contains("c.txt"));
    }

    #[tokio::test]
    async fn grep_matches_regex() {
        let dir = tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.txt"), "foo\nBAR\nbaz\n")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("b.txt"), "qux\n")
            .await
            .unwrap();

        let result = GrepTool
            .execute(json!({
                "pattern": "^bar$",
                "path": dir.path().to_str().unwrap(),
                "case_insensitive": true,
            }))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("a.txt:2:BAR"));
        assert!(!result.content.contains("qux"));
    }
}
