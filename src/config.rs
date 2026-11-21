use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::rules::Rule;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub rules: Vec<Rule>,
}

pub enum LoadOutcome {
    Loaded(Config),
    Missing,
    Broken { moved_to: PathBuf, error: String },
}

pub fn config_home() -> PathBuf {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"),
    }
}

pub fn config_path() -> PathBuf {
    config_home().join("discord-sound-capture").join("config.toml")
}

pub fn load() -> LoadOutcome {
    let path = config_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LoadOutcome::Missing,
        Err(e) => return broken(path, e.to_string()),
    };
    match toml::from_str::<Config>(&text) {
        Ok(c) => LoadOutcome::Loaded(c),
        Err(e) => broken(path, e.to_string()),
    }
}

fn broken(path: PathBuf, error: String) -> LoadOutcome {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let moved_to = path.with_file_name(format!("config.toml.broken-{secs}"));
    if let Err(e) = std::fs::rename(&path, &moved_to) {
        eprintln!("cannot move broken config: {e}");
    }
    LoadOutcome::Broken { moved_to, error }
}

pub fn save(cfg: &Config) -> std::io::Result<()> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = toml::to_string_pretty(cfg).map_err(std::io::Error::other)?;
    let tmp = path.with_file_name("config.toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Field;

    #[test]
    fn parses_example_and_roundtrips() {
        let src = r#"
[[rules]]
kind = "binary"
binary = "chrome"
label = "Google Chrome"

[[rules]]
kind = "regex"
field = "application.name"
pattern = "(?i)factorio"

[[rules]]
kind = "regex"
field = "node.name"
pattern = "x"
enabled = false
"#;
        let c: Config = toml::from_str(src).unwrap();
        assert_eq!(c.rules.len(), 3);
        assert_eq!(
            c.rules[1],
            Rule::Regex { field: Field::ApplicationName, pattern: "(?i)factorio".into(), enabled: true }
        );
        assert_eq!(
            c.rules[2],
            Rule::Regex { field: Field::NodeName, pattern: "x".into(), enabled: false }
        );
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }
}
