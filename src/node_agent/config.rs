use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AgentFileConfig {
    pub agent: AgentConfig,
    #[serde(default)]
    pub process: Vec<ProcessConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_log_buffer")]
    pub log_buffer_lines: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProcessConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub working_dir: Option<String>,
    #[serde(default)]
    pub auto_restart: bool,
    #[serde(default = "default_restart_delay")]
    pub restart_delay_secs: u64,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

fn default_name() -> String {
    "node".to_string()
}

fn default_port() -> u16 {
    8090
}

fn default_log_buffer() -> usize {
    1000
}

fn default_restart_delay() -> u64 {
    3
}

impl AgentFileConfig {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let toml_str = r#"
[agent]
name = "test"

[[process]]
name = "echo"
command = "echo"
args = ["hello"]
"#;
        let config: AgentFileConfig = toml::from_str(toml_str).expect("parse minimal config");
        assert_eq!(config.agent.name, "test");
        assert_eq!(config.agent.port, 8090);
        assert_eq!(config.agent.log_buffer_lines, 1000);
        assert_eq!(config.process.len(), 1);
        assert_eq!(config.process[0].name, "echo");
        assert!(!config.process[0].auto_restart);
        assert_eq!(config.process[0].restart_delay_secs, 3);
    }

    #[test]
    fn parse_full_config() {
        let toml_str = r#"
[agent]
name = "reader-linux"
port = 9090
log_buffer_lines = 500

[[process]]
name = "reader-0"
command = "./target/release/reader"
args = ["--config", "config.toml", "--source-id", "0"]
auto_restart = true
restart_delay_secs = 10
working_dir = "/opt/daq"

[process.env]
RUST_LOG = "debug"
"#;
        let config: AgentFileConfig = toml::from_str(toml_str).expect("parse full config");
        assert_eq!(config.agent.port, 9090);
        assert_eq!(config.agent.log_buffer_lines, 500);
        assert_eq!(config.process[0].restart_delay_secs, 10);
        assert!(config.process[0].auto_restart);
        assert_eq!(
            config.process[0].env.get("RUST_LOG").unwrap(),
            "debug"
        );
        assert_eq!(
            config.process[0].working_dir,
            Some("/opt/daq".to_string())
        );
    }

    #[test]
    fn parse_no_processes() {
        let toml_str = r#"
[agent]
name = "empty"
"#;
        let config: AgentFileConfig = toml::from_str(toml_str).expect("parse no processes");
        assert_eq!(config.process.len(), 0);
    }
}
