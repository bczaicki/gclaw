use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// Maximum characters per file when injected into system prompt.
const MAX_FILE_CHARS: usize = 20_000;

/// Maximum total characters for all workspace files combined.
const MAX_TOTAL_CHARS: usize = 150_000;

/// Ordered list of workspace files to load and inject.
/// Order matters — earlier files take priority if the total limit is hit.
const WORKSPACE_FILES: &[(&str, &str)] = &[
    ("IDENTITY.md", "Identity"),
    ("SOUL.md", "Soul"),
    ("AGENTS.md", "Operating Instructions"),
    ("USER.md", "User Context"),
    ("TOOLS.md", "Environment"),
    ("MEMORY.md", "Memory"),
    ("HEARTBEAT.md", "Periodic Tasks"),
];

/// Loaded workspace content, ready to be assembled into a system prompt.
#[derive(Debug, Clone)]
pub struct Workspace {
    sections: Vec<WorkspaceSection>,
    pub bootstrap: Option<String>,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
struct WorkspaceSection {
    label: String,
    content: String,
}

impl Workspace {
    /// Load workspace files from the given directory.
    /// Missing files are silently skipped. Files exceeding the per-file
    /// limit are truncated. If the total exceeds the global limit,
    /// later files are dropped.
    pub fn load(workspace_dir: &Path) -> Self {
        let mut sections = Vec::new();
        let mut total_chars = 0;

        for &(filename, label) in WORKSPACE_FILES {
            let file_path = workspace_dir.join(filename);
            match std::fs::read_to_string(&file_path) {
                Ok(raw) => {
                    let content = raw.trim().to_string();
                    if content.is_empty() {
                        continue;
                    }
                    let truncated = if content.len() > MAX_FILE_CHARS {
                        warn!("{filename} exceeds {MAX_FILE_CHARS} chars, truncating");
                        content[..MAX_FILE_CHARS].to_string()
                    } else {
                        content
                    };
                    if total_chars + truncated.len() > MAX_TOTAL_CHARS {
                        warn!(
                            "Total workspace content exceeds {MAX_TOTAL_CHARS} chars, skipping {filename}"
                        );
                        continue;
                    }
                    total_chars += truncated.len();
                    debug!(
                        "Loaded workspace file: {filename} ({} chars)",
                        truncated.len()
                    );
                    sections.push(WorkspaceSection {
                        label: label.to_string(),
                        content: truncated,
                    });
                }
                Err(_) => {
                    debug!("Workspace file not found: {filename}");
                }
            }
        }

        // Check for bootstrap file
        let bootstrap_path = workspace_dir.join("BOOTSTRAP.md");
        let bootstrap = std::fs::read_to_string(&bootstrap_path)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if bootstrap.is_some() {
            debug!("Bootstrap file found — onboarding flow active");
        }

        Workspace {
            sections,
            bootstrap,
            path: workspace_dir.to_path_buf(),
        }
    }

    /// Assemble all loaded workspace sections into a single system prompt.
    /// Each section is wrapped with a header for clarity.
    pub fn to_system_prompt(&self) -> String {
        let mut parts = Vec::new();

        for section in &self.sections {
            parts.push(format!("--- {} ---\n{}", section.label, section.content));
        }

        if let Some(ref bootstrap) = self.bootstrap {
            parts.push(format!("--- Bootstrap (First Run) ---\n{bootstrap}"));
        }

        parts.join("\n\n")
    }

    /// Build system prompt by combining workspace content with a fallback.
    /// If workspace has content, it replaces the fallback entirely.
    /// If workspace is empty, the fallback is used.
    pub fn build_system_prompt(&self, fallback: &str) -> String {
        let workspace_prompt = self.to_system_prompt();
        if workspace_prompt.is_empty() {
            fallback.to_string()
        } else {
            workspace_prompt
        }
    }

    /// Delete the bootstrap file after onboarding is complete.
    pub fn delete_bootstrap(&self) -> std::io::Result<()> {
        let path = self.path.join("BOOTSTRAP.md");
        if path.exists() {
            std::fs::remove_file(&path)?;
            debug!("Deleted BOOTSTRAP.md after onboarding");
        }
        Ok(())
    }

    /// Returns true if workspace files were found and loaded.
    pub fn is_loaded(&self) -> bool {
        !self.sections.is_empty()
    }
}

/// Resolve the workspace directory path.
/// Priority: GCLAW_WORKSPACE env var > config value > ./workspace > ~/.config/gclaw/workspace
pub fn resolve_workspace_dir(config_path: Option<&str>) -> PathBuf {
    if let Ok(path) = std::env::var("GCLAW_WORKSPACE") {
        return PathBuf::from(path);
    }
    if let Some(path) = config_path {
        return PathBuf::from(path);
    }
    // Check ./workspace relative to cwd
    let local = PathBuf::from("workspace");
    if local.exists() {
        return local;
    }
    // Fall back to XDG config
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gclaw")
        .join("workspace")
}
