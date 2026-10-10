//! Project-local configuration: `.lazydelta/config.json`.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EditorConfig {
    pub command: String,
    /// Argument template; each arg may contain `{file}` and `{line}`.
    pub args: Vec<String>,
}

impl EditorConfig {
    /// Build the argument list for a concrete file/line.
    pub fn args_for(&self, file: &Path, line: usize) -> Vec<String> {
        let file = file.display().to_string();
        self.args
            .iter()
            .map(|a| a.replace("{file}", &file).replace("{line}", &line.to_string()))
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub agents: Vec<AgentConfig>,
    #[serde(default)]
    pub editor: Option<EditorConfig>,
    /// Alternative to `editor`: a named preset ("zed", "nvim", "code").
    #[serde(default)]
    pub editor_preset: Option<String>,
    /// Per-prompt timeout for the agent runner, in seconds.
    #[serde(default = "default_prompt_timeout_secs")]
    pub prompt_timeout_secs: u64,
}

fn default_prompt_timeout_secs() -> u64 {
    600
}

impl Default for Config {
    fn default() -> Self {
        Config {
            agents: Vec::new(),
            editor: None,
            editor_preset: None,
            prompt_timeout_secs: default_prompt_timeout_secs(),
        }
    }
}

impl Config {
    pub fn config_path(project_root: &Path) -> PathBuf {
        project_root.join(".lazydelta").join("config.json")
    }

    /// Load `.lazydelta/config.json`; missing file → defaults.
    pub fn load(project_root: &Path) -> std::io::Result<Config> {
        let path = Self::config_path(project_root);
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{}: {e}", path.display()),
                )
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e),
        }
    }

    /// The effective editor config: explicit `editor` wins, then
    /// `editor_preset`, else `None` (jump action must surface an error).
    pub fn effective_editor(&self) -> Option<EditorConfig> {
        if let Some(editor) = &self.editor {
            return Some(editor.clone());
        }
        self.editor_preset.as_deref().and_then(editor_preset)
    }
}

/// Known editor presets (task 5.1).
pub fn editor_preset(name: &str) -> Option<EditorConfig> {
    let cfg = match name {
        "zed" => EditorConfig {
            command: "zed".into(),
            args: vec!["{file}:{line}".into()],
        },
        "nvim" => EditorConfig {
            command: "nvim".into(),
            args: vec!["+{line}".into(), "{file}".into()],
        },
        "code" => EditorConfig {
            command: "code".into(),
            args: vec!["-g".into(), "{file}:{line}".into()],
        },
        _ => return None,
    };
    Some(cfg)
}

/// Launch an editor detached. The caller supplies absolute paths.
pub fn launch_editor(cfg: &EditorConfig, file: &Path, line: usize) -> std::io::Result<()> {
    Command::new(&cfg.command)
        .args(cfg.args_for(file, line))
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_substitute_file_and_line() {
        let nvim = editor_preset("nvim").unwrap();
        assert_eq!(
            nvim.args_for(Path::new("/tmp/a.rs"), 42),
            vec!["+42".to_string(), "/tmp/a.rs".to_string()]
        );
        let zed = editor_preset("zed").unwrap();
        assert_eq!(zed.args_for(Path::new("/tmp/a.rs"), 7), vec!["/tmp/a.rs:7".to_string()]);
    }

    #[test]
    fn effective_editor_prefers_explicit_then_preset() {
        let mut cfg = Config::default();
        assert!(cfg.effective_editor().is_none());
        cfg.editor_preset = Some("nvim".into());
        assert_eq!(cfg.effective_editor().unwrap().command, "nvim");
        cfg.editor = Some(editor_preset("code").unwrap());
        assert_eq!(cfg.effective_editor().unwrap().command, "code");
        assert!(editor_preset("emacs").is_none());
    }

    #[test]
    fn load_missing_config_gives_defaults() {
        let root = std::env::temp_dir().join(format!("ld-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let cfg = Config::load(&root).unwrap();
        assert_eq!(cfg, Config::default());
        let _ = std::fs::remove_dir_all(&root);
    }
}
