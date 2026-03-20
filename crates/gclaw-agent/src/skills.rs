use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Parsed SKILL.md frontmatter.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillFrontmatter {
    pub name: Option<String>,
    pub description: String,
    #[serde(default = "default_true")]
    #[serde(rename = "user-invocable")]
    pub user_invocable: bool,
    #[serde(default)]
    #[serde(rename = "disable-model-invocation")]
    pub disable_model_invocation: bool,
    #[serde(default)]
    #[serde(rename = "allowed-tools")]
    pub allowed_tools: Option<String>,
    #[serde(default)]
    #[serde(rename = "argument-hint")]
    pub argument_hint: Option<String>,
}

fn default_true() -> bool {
    true
}

/// A loaded skill.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub user_invocable: bool,
    pub disable_model_invocation: bool,
    pub allowed_tools: Option<String>,
    pub argument_hint: Option<String>,
    /// The markdown body (after frontmatter).
    pub body: String,
    /// Source path for debugging.
    pub source: PathBuf,
}

impl Skill {
    /// Parse a SKILL.md file. The `dir_name` is used as the default name.
    pub fn parse(content: &str, dir_name: &str, source: PathBuf) -> Option<Self> {
        let (frontmatter, body) = split_frontmatter(content)?;
        let fm: SkillFrontmatter = serde_yaml_minimal_parse(&frontmatter)?;

        Some(Skill {
            name: fm.name.unwrap_or_else(|| dir_name.to_string()),
            description: fm.description,
            user_invocable: fm.user_invocable,
            disable_model_invocation: fm.disable_model_invocation,
            allowed_tools: fm.allowed_tools,
            argument_hint: fm.argument_hint,
            body: body.to_string(),
            source,
        })
    }

    /// Render the skill body with argument substitution.
    pub fn render(&self, args: &str) -> String {
        let mut result = self.body.clone();
        result = result.replace("$ARGUMENTS", args);
        result = result.replace("$0", args);

        // Split args for positional substitution
        let parts: Vec<&str> = args.split_whitespace().collect();
        // Replace $N (1-indexed) — do it in reverse order so $10 is replaced before $1
        for i in (1..=9).rev() {
            let placeholder = format!("${i}");
            let value = parts.get(i - 1).copied().unwrap_or("");
            result = result.replace(&placeholder, value);
        }

        result
    }
}

/// Split YAML frontmatter from markdown body.
/// Expects `---\n...\n---\n` at the start.
fn split_frontmatter(content: &str) -> Option<(String, String)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }

    let after_first = &trimmed[3..];
    let after_first = after_first.strip_prefix('\n').unwrap_or(after_first);

    let end = after_first.find("\n---")?;
    let frontmatter = after_first[..end].to_string();
    let body = after_first[end + 4..].trim_start_matches('\n').to_string();

    Some((frontmatter, body))
}

/// Minimal YAML-like parser for skill frontmatter.
/// Handles simple key: value pairs and quoted strings.
fn serde_yaml_minimal_parse(yaml: &str) -> Option<SkillFrontmatter> {
    let mut map: HashMap<String, serde_json::Value> = HashMap::new();

    for line in yaml.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (key, value) = line.split_once(':')?;
        let key = key.trim().to_string();
        let value = value.trim();

        // Parse value
        let json_value = if value == "true" {
            serde_json::Value::Bool(true)
        } else if value == "false" {
            serde_json::Value::Bool(false)
        } else if value.starts_with('"') && value.ends_with('"') {
            serde_json::Value::String(value[1..value.len() - 1].to_string())
        } else if value.starts_with('[') && value.ends_with(']') {
            // Simple array like [message]
            serde_json::Value::String(value.to_string())
        } else {
            serde_json::Value::String(value.to_string())
        };

        map.insert(key, json_value);
    }

    let json = serde_json::to_value(&map).ok()?;
    serde_json::from_value(json).ok()
}

/// Registry that discovers and manages skills.
pub struct SkillRegistry {
    skills: HashMap<String, Skill>,
}

impl SkillRegistry {
    /// Discover skills from the three standard locations.
    /// Later locations override earlier ones.
    pub fn discover(workspace_dir: &Path) -> Self {
        let mut skills = HashMap::new();

        // 1. User-level: ~/.gclaw/skills/*/SKILL.md
        if let Some(home) = dirs::home_dir() {
            let user_dir = home.join(".gclaw").join("skills");
            Self::load_from_dir(&user_dir, &mut skills);
        }

        // 2. Project-level: .gclaw/skills/*/SKILL.md (relative to CWD)
        let project_dir = PathBuf::from(".gclaw").join("skills");
        Self::load_from_dir(&project_dir, &mut skills);

        // 3. Workspace-level: {workspace_dir}/skills/*/SKILL.md
        let workspace_skills = workspace_dir.join("skills");
        Self::load_from_dir(&workspace_skills, &mut skills);

        if !skills.is_empty() {
            info!(count = skills.len(), "Skills discovered");
        }

        Self { skills }
    }

    fn load_from_dir(dir: &Path, skills: &mut HashMap<String, Skill>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return, // Directory doesn't exist, that's fine
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let skill_file = path.join("SKILL.md");
            if !skill_file.exists() {
                continue;
            }

            let dir_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            match std::fs::read_to_string(&skill_file) {
                Ok(content) => {
                    if let Some(skill) = Skill::parse(&content, dir_name, skill_file.clone()) {
                        debug!(name = %skill.name, source = %skill_file.display(), "Loaded skill");
                        skills.insert(skill.name.clone(), skill);
                    } else {
                        warn!(path = %skill_file.display(), "Failed to parse SKILL.md");
                    }
                }
                Err(e) => {
                    warn!(path = %skill_file.display(), error = %e, "Failed to read SKILL.md");
                }
            }
        }
    }

    /// Get a skill by name.
    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }

    /// List all user-invocable skill names.
    pub fn user_invocable_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self
            .skills
            .values()
            .filter(|s| s.user_invocable)
            .map(|s| s.name.clone())
            .collect();
        names.sort();
        names
    }

    /// List all skills with their descriptions (for /skills display).
    pub fn list_display(&self) -> Vec<(String, String, Option<String>)> {
        let mut list: Vec<_> = self
            .skills
            .values()
            .filter(|s| s.user_invocable)
            .map(|s| {
                (
                    s.name.clone(),
                    s.description.clone(),
                    s.argument_hint.clone(),
                )
            })
            .collect();
        list.sort_by(|a, b| a.0.cmp(&b.0));
        list
    }

    /// Check if a name matches a registered skill.
    pub fn has(&self, name: &str) -> bool {
        self.skills.contains_key(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skill_basic() {
        let content = r#"---
name: commit
description: Create a well-structured git commit
user-invocable: true
---

Create a git commit. Analyze staged changes.
$ARGUMENTS
"#;
        let skill = Skill::parse(content, "commit", PathBuf::from("test")).unwrap();
        assert_eq!(skill.name, "commit");
        assert_eq!(skill.description, "Create a well-structured git commit");
        assert!(skill.user_invocable);
        assert!(!skill.disable_model_invocation);
        assert!(skill.body.contains("$ARGUMENTS"));
    }

    #[test]
    fn parse_skill_defaults() {
        let content = r#"---
description: A test skill
---

Do something.
"#;
        let skill = Skill::parse(content, "my-skill", PathBuf::from("test")).unwrap();
        assert_eq!(skill.name, "my-skill"); // defaults to dir name
        assert!(skill.user_invocable); // defaults to true
        assert!(!skill.disable_model_invocation); // defaults to false
        assert!(skill.allowed_tools.is_none());
    }

    #[test]
    fn parse_skill_all_fields() {
        let content = r#"---
name: custom
description: Custom skill
user-invocable: false
disable-model-invocation: true
allowed-tools: Bash(git *)
argument-hint: [message]
---

Body here.
"#;
        let skill = Skill::parse(content, "dir", PathBuf::from("test")).unwrap();
        assert_eq!(skill.name, "custom");
        assert!(!skill.user_invocable);
        assert!(skill.disable_model_invocation);
        assert_eq!(skill.allowed_tools.as_deref(), Some("Bash(git *)"));
        assert_eq!(skill.argument_hint.as_deref(), Some("[message]"));
    }

    #[test]
    fn parse_skill_no_frontmatter() {
        let content = "Just some markdown without frontmatter.";
        assert!(Skill::parse(content, "test", PathBuf::from("test")).is_none());
    }

    #[test]
    fn parse_skill_malformed_yaml() {
        let content = r#"---
this is not: valid: yaml: at: all
---

Body.
"#;
        // Should fail gracefully
        assert!(Skill::parse(content, "test", PathBuf::from("test")).is_none());
    }

    #[test]
    fn render_arguments() {
        let content = r#"---
description: Test
---

Do this: $ARGUMENTS
"#;
        let skill = Skill::parse(content, "test", PathBuf::from("test")).unwrap();
        let rendered = skill.render("fix the bug");
        assert_eq!(rendered, "Do this: fix the bug\n");
    }

    #[test]
    fn render_positional_args() {
        let content = r#"---
description: Test
---

First: $1, Second: $2, All: $0
"#;
        let skill = Skill::parse(content, "test", PathBuf::from("test")).unwrap();
        let rendered = skill.render("hello world");
        assert!(rendered.contains("First: hello"));
        assert!(rendered.contains("Second: world"));
        assert!(rendered.contains("All: hello world"));
    }

    #[test]
    fn render_missing_positional() {
        let content = r#"---
description: Test
---

$1 and $2
"#;
        let skill = Skill::parse(content, "test", PathBuf::from("test")).unwrap();
        let rendered = skill.render("only-one");
        assert!(rendered.contains("only-one and "));
    }

    #[test]
    fn split_frontmatter_basic() {
        let (fm, body) = split_frontmatter("---\nfoo: bar\n---\nBody text").unwrap();
        assert_eq!(fm, "foo: bar");
        assert_eq!(body, "Body text");
    }

    #[test]
    fn split_frontmatter_no_separator() {
        assert!(split_frontmatter("No frontmatter here").is_none());
    }

    #[test]
    fn split_frontmatter_with_leading_whitespace() {
        let (fm, _) = split_frontmatter("  ---\nkey: val\n---\nbody").unwrap();
        assert_eq!(fm, "key: val");
    }

    #[test]
    fn discovery_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let registry = SkillRegistry::discover(dir.path());
        assert!(registry.user_invocable_names().is_empty());
    }

    #[test]
    fn discovery_with_skill() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("skills").join("greet");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\ndescription: Say hello\n---\nHello $ARGUMENTS!\n",
        )
        .unwrap();

        let registry = SkillRegistry::discover(dir.path());
        assert!(registry.has("greet"));
        let names = registry.user_invocable_names();
        assert_eq!(names, vec!["greet"]);
    }

    #[test]
    fn discovery_override_precedence() {
        // Workspace skills override project skills
        let dir = tempfile::tempdir().unwrap();
        let ws_skill_dir = dir.path().join("skills").join("test");
        std::fs::create_dir_all(&ws_skill_dir).unwrap();
        std::fs::write(
            ws_skill_dir.join("SKILL.md"),
            "---\ndescription: Workspace version\n---\nWorkspace body\n",
        )
        .unwrap();

        let registry = SkillRegistry::discover(dir.path());
        let skill = registry.get("test").unwrap();
        assert_eq!(skill.description, "Workspace version");
    }

    #[test]
    fn list_display() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("skills").join("hello");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\ndescription: Say hello\nargument-hint: [name]\n---\nHi $1!\n",
        )
        .unwrap();

        let registry = SkillRegistry::discover(dir.path());
        let list = registry.list_display();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, "hello");
        assert_eq!(list[0].1, "Say hello");
        assert_eq!(list[0].2.as_deref(), Some("[name]"));
    }
}
