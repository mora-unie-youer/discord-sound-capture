use std::path::PathBuf;

pub const WP_RULE_PATH_HINT: &str = "~/.config/wireplumber/wireplumber.conf.d/50-discord-capture.conf";
pub const WP_RULE_TEXT: &str = r#"stream.rules = [
  {
    matches = [ { application.name = "discord_capture" } ]
    actions = { update-props = { state.restore-props = false } }
  }
]"#;

pub fn wp_rule_present() -> bool {
    let dirs: [PathBuf; 3] = [
        crate::config::config_home().join("wireplumber/wireplumber.conf.d"),
        "/etc/wireplumber/wireplumber.conf.d".into(),
        "/usr/share/wireplumber/wireplumber.conf.d".into(),
    ];
    dirs.iter().any(|d| {
        let Ok(rd) = std::fs::read_dir(d) else { return false };
        rd.flatten().any(|e| {
            e.path().extension().is_some_and(|x| x == "conf")
                && std::fs::read_to_string(e.path())
                    .is_ok_and(|t| t.contains("discord_capture") && t.contains("state.restore-props"))
        })
    })
}
