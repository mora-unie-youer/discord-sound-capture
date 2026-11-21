use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Field {
    #[serde(rename = "application.name")]
    ApplicationName,
    #[serde(rename = "application.process.binary")]
    ProcessBinary,
    #[serde(rename = "media.name")]
    MediaName,
    #[serde(rename = "node.name")]
    NodeName,
}

impl Field {
    pub const ALL: [Field; 4] = [
        Field::ApplicationName,
        Field::ProcessBinary,
        Field::MediaName,
        Field::NodeName,
    ];

    pub fn key(&self) -> &'static str {
        match self {
            Field::ApplicationName => "application.name",
            Field::ProcessBinary => "application.process.binary",
            Field::MediaName => "media.name",
            Field::NodeName => "node.name",
        }
    }
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Rule {
    Binary {
        binary: String,
        label: Option<String>,
    },
    Regex {
        field: Field,
        pattern: String,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StreamInfo {
    pub index: u32,
    pub app_name: Option<String>,
    pub binary: Option<String>,
    pub media_name: Option<String>,
    pub node_name: Option<String>,
}

impl StreamInfo {
    pub fn field(&self, f: &Field) -> Option<&str> {
        match f {
            Field::ApplicationName => self.app_name.as_deref(),
            Field::ProcessBinary => self.binary.as_deref(),
            Field::MediaName => self.media_name.as_deref(),
            Field::NodeName => self.node_name.as_deref(),
        }
    }
}

pub struct RuleSet {
    rules: Vec<Rule>,
    compiled: Vec<Option<regex::Regex>>,
}

impl RuleSet {
    pub fn new(rules: Vec<Rule>) -> Self {
        let compiled = rules
            .iter()
            .map(|r| match r {
                Rule::Regex { pattern, .. } => regex::Regex::new(pattern).ok(),
                Rule::Binary { .. } => None,
            })
            .collect();
        Self { rules, compiled }
    }

    pub fn allows(&self, s: &StreamInfo) -> bool {
        self.rules
            .iter()
            .zip(&self.compiled)
            .any(|(rule, re)| match rule {
                Rule::Binary { binary, .. } => s.binary.as_deref() == Some(binary.as_str()),
                Rule::Regex { field, .. } => match (re, s.field(field)) {
                    (Some(re), Some(v)) => re.is_match(v),
                    _ => false,
                },
            })
    }
}

pub fn validate_pattern(p: &str) -> Result<(), String> {
    regex::Regex::new(p).map(|_| ()).map_err(|e| e.to_string())
}

/// true = capture must be muted.
pub fn desired_mute(
    target: Option<u32>,
    streams: &HashMap<u32, StreamInfo>,
    rules: &RuleSet,
    shutting_down: bool,
) -> bool {
    if shutting_down {
        return false;
    }
    match target.and_then(|t| streams.get(&t)) {
        Some(s) => !rules.allows(s),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> StreamInfo {
        StreamInfo {
            index: 7,
            app_name: Some("Factorio".into()),
            binary: Some("factorio".into()),
            media_name: Some("Playback".into()),
            node_name: Some("alsa_playback.factorio".into()),
        }
    }
    fn re(field: Field, p: &str) -> Rule {
        Rule::Regex { field, pattern: p.into() }
    }

    #[test]
    fn binary_rule() {
        let rs = RuleSet::new(vec![Rule::Binary { binary: "factorio".into(), label: None }]);
        assert!(rs.allows(&s()));
        let mut x = s();
        x.binary = None;
        assert!(!rs.allows(&x));
    }

    #[test]
    fn regex_each_field() {
        for (f, p) in [
            (Field::ApplicationName, "^Fact"),
            (Field::ProcessBinary, "^factorio$"),
            (Field::MediaName, "Play"),
            (Field::NodeName, "alsa_.*torio"),
        ] {
            assert!(RuleSet::new(vec![re(f.clone(), p)]).allows(&s()), "{f}");
            assert!(!RuleSet::new(vec![re(f, "^zzz$")]).allows(&s()));
        }
    }

    #[test]
    fn missing_field_no_match() {
        let mut x = s();
        x.media_name = None;
        assert!(!RuleSet::new(vec![re(Field::MediaName, "")]).allows(&x));
    }

    #[test]
    fn invalid_regex() {
        assert!(!RuleSet::new(vec![re(Field::ApplicationName, "(")]).allows(&s()));
        assert!(validate_pattern("(").is_err());
        assert!(validate_pattern("a+").is_ok());
    }

    #[test]
    fn mute_policy() {
        let mut m = HashMap::new();
        m.insert(7, s());
        let allow = RuleSet::new(vec![re(Field::ApplicationName, "Factorio")]);
        let deny = RuleSet::new(vec![]);
        assert!(desired_mute(Some(99), &m, &allow, false));
        assert!(desired_mute(None, &m, &allow, false));
        assert!(!desired_mute(Some(7), &m, &allow, false));
        assert!(desired_mute(Some(7), &m, &deny, false));
        assert!(!desired_mute(Some(99), &m, &deny, true));
        assert!(!desired_mute(Some(7), &m, &deny, true));
    }
}
