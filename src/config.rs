use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub agent: AgentConfig,
    pub agents: HashMap<String, AgentDefinition>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub auth_token: Option<String>,
}

/// Metadata public de l'instance bridge (Agent Card)
#[derive(Debug, Deserialize, Clone)]
pub struct AgentConfig {
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default = "default_description")]
    pub description: String,
    #[serde(default = "default_version")]
    pub version: String,
}

/// Définition d'un agent sous-jacent disponible
#[derive(Debug, Deserialize, Clone)]
pub struct AgentDefinition {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub workdir: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    #[serde(default = "default_keep_alive")]
    pub keep_alive: bool,
    /// Description affichée dans l'Agent Card pour cet agent
    #[serde(default)]
    pub description: Option<String>,
    /// Session ID to resume (passed as --resume to Claude Code)
    #[serde(default)]
    pub resume: Option<String>,
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8742
}

fn default_name() -> String {
    "A2A Bridge".to_string()
}

fn default_description() -> String {
    "A2A bridge to AI coding agents".to_string()
}

fn default_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn default_timeout() -> u64 {
    600
}

fn default_keep_alive() -> bool {
    true
}

impl Config {
    pub fn load(path: &str) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file '{}': {}", path, e))?;

        let mut config: Config = toml::from_str(&content)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        // Expand environment variables
        for agent_def in config.agents.values_mut() {
            agent_def.command = Self::expand_vars(&agent_def.command);
            agent_def.args = agent_def
                .args
                .iter()
                .map(|a| Self::expand_vars(a))
                .collect();
            if let Some(ref wd) = agent_def.workdir {
                agent_def.workdir = Some(Self::expand_vars(wd));
            }
        }

        Ok(config)
    }

    /// Récupérer un agent par nom
    pub fn get_agent(&self, name: &str) -> Option<&AgentDefinition> {
        self.agents.get(name)
    }

    /// Lister les agents disponibles
    pub fn list_agents(&self) -> Vec<(&String, &AgentDefinition)> {
        self.agents.iter().collect()
    }

    fn expand_vars(s: &str) -> String {
        let mut result = s.to_string();
        if let Ok(home) = std::env::var("HOME") {
            result = result.replace("${HOME}", &home);
            result = result.replace("$HOME", &home);
        }
        if let Ok(user) = std::env::var("USER") {
            result = result.replace("${USER}", &user);
            result = result.replace("$USER", &user);
        }
        result
    }

    pub fn socket_addr(&self) -> String {
        format!("{}:{}", self.server.host, self.server.port)
    }

    /// Génère l'Agent Card pour l'agent sélectionné
    pub fn agent_card(&self, base_url: String, agent_name: &str, agent_def: &AgentDefinition) -> serde_json::Value {
        let display_name = if self.agent.name == default_name() {
            format!("{} ({})", agent_name, self.agent.name)
        } else {
            self.agent.name.clone()
        };

        let description = agent_def
            .description
            .clone()
            .unwrap_or_else(|| self.agent.description.clone());

        serde_json::json!({
            "name": display_name,
            "description": description,
            "url": base_url,
            "version": self.agent.version,
            "capabilities": {
                "streaming": true,
                "pushNotifications": false
            },
            "authentication": {
                "schemes": if self.server.auth_token.is_some() { vec!["bearer"] } else { vec![] }
            },
            "defaultInputModalities": ["text"],
            "defaultOutputModalities": ["text"],
            "skills": [
                {
                    "id": "chat",
                    "name": "Conversation",
                    "description": format!("Interact with {} via natural language", agent_name),
                    "tags": ["chat", "code", agent_name]
                }
            ]
        })
    }
}
