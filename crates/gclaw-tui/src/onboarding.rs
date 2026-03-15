use std::path::{Path, PathBuf};

/// The onboarding wizard steps.
#[derive(Debug, Clone, PartialEq)]
pub enum OnboardingStep {
    Welcome,
    Name,
    Pronouns,
    Timezone,
    Projects,
    CommStyle,
    Shell,
    Editor,
    Confirm,
    Writing,
    Done,
}

impl OnboardingStep {
    pub fn index(&self) -> usize {
        match self {
            Self::Welcome => 0,
            Self::Name => 1,
            Self::Pronouns => 2,
            Self::Timezone => 3,
            Self::Projects => 4,
            Self::CommStyle => 5,
            Self::Shell => 6,
            Self::Editor => 7,
            Self::Confirm => 8,
            Self::Writing => 9,
            Self::Done => 10,
        }
    }

    pub fn total() -> usize {
        9 // Welcome through Confirm (visible steps)
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Welcome => Self::Name,
            Self::Name => Self::Pronouns,
            Self::Pronouns => Self::Timezone,
            Self::Timezone => Self::Projects,
            Self::Projects => Self::CommStyle,
            Self::CommStyle => Self::Shell,
            Self::Shell => Self::Editor,
            Self::Editor => Self::Confirm,
            Self::Confirm => Self::Writing,
            Self::Writing => Self::Done,
            Self::Done => Self::Done,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            Self::Welcome => Self::Welcome,
            Self::Name => Self::Welcome,
            Self::Pronouns => Self::Name,
            Self::Timezone => Self::Pronouns,
            Self::Projects => Self::Timezone,
            Self::CommStyle => Self::Projects,
            Self::Shell => Self::CommStyle,
            Self::Editor => Self::Shell,
            Self::Confirm => Self::Editor,
            Self::Writing => Self::Confirm,
            Self::Done => Self::Done,
        }
    }

    pub fn prompt(&self) -> &str {
        match self {
            Self::Welcome => "",
            Self::Name => "What should I call you?",
            Self::Pronouns => "What are your pronouns?",
            Self::Timezone => "What timezone are you in?",
            Self::Projects => "What are you working on right now?",
            Self::CommStyle => "How should I communicate?",
            Self::Shell => "What shell do you use?",
            Self::Editor => "Preferred editor?",
            Self::Confirm => "",
            Self::Writing => "",
            Self::Done => "",
        }
    }

    pub fn hint(&self) -> &str {
        match self {
            Self::Name => "e.g., Brian, Dr. Smith, Captain",
            Self::Pronouns => "e.g., he/him, she/her, they/them (Tab to skip)",
            Self::Timezone => "e.g., US/Eastern, Europe/London, Asia/Tokyo",
            Self::Projects => "Brief description, one line (Tab to skip)",
            Self::CommStyle => "terse / casual / detailed / formal (Tab for default: casual)",
            Self::Shell => "e.g., zsh, bash, fish (Tab to auto-detect)",
            Self::Editor => "e.g., vim, neovim, vscode, emacs (Tab to skip)",
            _ => "",
        }
    }

    pub fn is_input_step(&self) -> bool {
        matches!(
            self,
            Self::Name
                | Self::Pronouns
                | Self::Timezone
                | Self::Projects
                | Self::CommStyle
                | Self::Shell
                | Self::Editor
        )
    }
}

/// All the data collected during onboarding.
#[derive(Debug, Clone, Default)]
pub struct OnboardingData {
    pub name: String,
    pub pronouns: String,
    pub timezone: String,
    pub projects: String,
    pub comm_style: String,
    pub shell: String,
    pub editor: String,
}

impl OnboardingData {
    /// Store the current input into the appropriate field for the given step.
    pub fn set_field(&mut self, step: &OnboardingStep, value: String) {
        match step {
            OnboardingStep::Name => self.name = value,
            OnboardingStep::Pronouns => self.pronouns = value,
            OnboardingStep::Timezone => self.timezone = value,
            OnboardingStep::Projects => self.projects = value,
            OnboardingStep::CommStyle => self.comm_style = value,
            OnboardingStep::Shell => self.shell = value,
            OnboardingStep::Editor => self.editor = value,
            _ => {}
        }
    }

    /// Get the current value for a step (used when navigating back).
    pub fn get_field(&self, step: &OnboardingStep) -> &str {
        match step {
            OnboardingStep::Name => &self.name,
            OnboardingStep::Pronouns => &self.pronouns,
            OnboardingStep::Timezone => &self.timezone,
            OnboardingStep::Projects => &self.projects,
            OnboardingStep::CommStyle => &self.comm_style,
            OnboardingStep::Shell => &self.shell,
            OnboardingStep::Editor => &self.editor,
            _ => "",
        }
    }

    pub fn generate_user_md(&self) -> String {
        let mut lines = vec!["# USER".to_string(), String::new()];

        if !self.name.is_empty() {
            lines.push(format!("- **Name:** {}", self.name));
        }
        if !self.pronouns.is_empty() {
            lines.push(format!("- **Pronouns:** {}", self.pronouns));
        }
        if !self.timezone.is_empty() {
            lines.push(format!("- **Timezone:** {}", self.timezone));
        }

        lines.push(String::new());
        lines.push("## Current Projects".to_string());
        lines.push(String::new());
        if !self.projects.is_empty() {
            lines.push(format!("- {}", self.projects));
        } else {
            lines.push("-".to_string());
        }

        lines.push(String::new());
        lines.push("## Preferences".to_string());
        lines.push(String::new());
        let style = if self.comm_style.is_empty() {
            "casual"
        } else {
            &self.comm_style
        };
        lines.push(format!("- **Communication:** {style}"));

        lines.push(String::new());
        lines.join("\n")
    }

    pub fn generate_tools_md(&self) -> String {
        let mut lines = vec![
            "# TOOLS".to_string(),
            String::new(),
            "## Local Environment".to_string(),
            String::new(),
        ];

        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        lines.push(format!("- **OS:** {os} ({arch})"));

        if !self.shell.is_empty() {
            lines.push(format!("- **Shell:** {}", self.shell));
        } else if let Ok(shell) = std::env::var("SHELL") {
            lines.push(format!("- **Shell:** {shell}"));
        }

        if !self.editor.is_empty() {
            lines.push(format!("- **Default editor:** {}", self.editor));
        } else if let Ok(editor) = std::env::var("EDITOR") {
            lines.push(format!("- **Default editor:** {editor}"));
        }

        lines.push(String::new());
        lines.join("\n")
    }

    /// Write USER.md and TOOLS.md, delete BOOTSTRAP.md.
    pub fn write_to_workspace(&self, workspace_dir: &Path) -> std::io::Result<()> {
        std::fs::write(workspace_dir.join("USER.md"), self.generate_user_md())?;
        std::fs::write(workspace_dir.join("TOOLS.md"), self.generate_tools_md())?;

        let bootstrap = workspace_dir.join("BOOTSTRAP.md");
        if bootstrap.exists() {
            std::fs::remove_file(bootstrap)?;
        }
        Ok(())
    }
}

/// Onboarding state, held by the App when in onboarding mode.
pub struct OnboardingState {
    pub step: OnboardingStep,
    pub data: OnboardingData,
    pub input: String,
    pub cursor_position: usize,
    pub workspace_dir: PathBuf,
    pub tick: u64,
}

impl OnboardingState {
    pub fn new(workspace_dir: PathBuf) -> Self {
        // Auto-detect shell for default
        let shell_default = std::env::var("SHELL").unwrap_or_default();
        let data = OnboardingData {
            shell: shell_default.rsplit('/').next().unwrap_or("").to_string(),
            ..Default::default()
        };

        Self {
            step: OnboardingStep::Welcome,
            data,
            input: String::new(),
            cursor_position: 0,
            workspace_dir,
            tick: 0,
        }
    }

    pub fn advance(&mut self) {
        // Save current input
        if self.step.is_input_step() {
            self.data.set_field(&self.step, self.input.clone());
        }
        self.step = self.step.next();
        // Load existing data for new step (for going back)
        self.input = self.data.get_field(&self.step).to_string();
        self.cursor_position = self.input.len();
    }

    pub fn go_back(&mut self) {
        if self.step.is_input_step() {
            self.data.set_field(&self.step, self.input.clone());
        }
        self.step = self.step.prev();
        self.input = self.data.get_field(&self.step).to_string();
        self.cursor_position = self.input.len();
    }

    pub fn skip_field(&mut self) {
        if self.step.is_input_step() {
            // Use auto-detected default for shell, "casual" for comm style
            match self.step {
                OnboardingStep::Shell => {
                    // keep the auto-detected value
                }
                OnboardingStep::CommStyle => {
                    self.input = "casual".to_string();
                    self.data.set_field(&self.step, self.input.clone());
                }
                _ => {
                    self.input.clear();
                }
            }
            self.advance();
        }
    }

    pub fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor_position, c);
        self.cursor_position += c.len_utf8();
    }

    pub fn delete_char(&mut self) {
        if self.cursor_position > 0 {
            let prev = self.input[..self.cursor_position]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor_position -= prev;
            self.input.remove(self.cursor_position);
        }
    }

    pub fn move_cursor_left(&mut self) {
        if self.cursor_position > 0 {
            let prev = self.input[..self.cursor_position]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor_position -= prev;
        }
    }

    pub fn move_cursor_right(&mut self) {
        if self.cursor_position < self.input.len() {
            let next = self.input[self.cursor_position..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor_position += next;
        }
    }

    /// Finalize: write files and mark done.
    pub fn finalize(&mut self) -> std::io::Result<()> {
        self.data.write_to_workspace(&self.workspace_dir)?;
        self.step = OnboardingStep::Done;
        Ok(())
    }

    pub fn tick(&mut self) {
        self.tick += 1;
    }
}
