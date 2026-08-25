mod a2a_server;
mod config;
mod lifeos_process;
mod ndjson;
mod sse;

use std::sync::Arc;
use tokio::sync::Mutex;

use a2a_server::{create_router, AppState};
use config::Config;
use lifeos_process::LifeOsProcess;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    // Parse CLI args
    let args: Vec<String> = std::env::args().collect();
    let config_path = args
        .iter()
        .position(|a| a == "--config" || a == "-c")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .unwrap_or("agents.toml");

    let agent_name = args
        .iter()
        .position(|a| a == "--agent" || a == "-a")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str());

    // Load config
    let config = Config::load(config_path).unwrap_or_else(|e| {
        eprintln!("❌ Failed to load config: {}", e);
        std::process::exit(1);
    });

    // Select agent
    let agent_name = match agent_name {
        Some(name) => {
            if !config.agents.contains_key(name) {
                eprintln!("❌ Agent '{}' not found in config.", name);
                eprintln!("Available agents:");
                for (name, _) in config.list_agents() {
                    eprintln!("  - {}", name);
                }
                std::process::exit(1);
            }
            name.to_string()
        }
        None => {
            // If only one agent defined, use it. Otherwise, list and exit.
            let agents = config.list_agents();
            if agents.len() == 1 {
                agents[0].0.clone()
            } else {
                eprintln!("❌ No agent specified. Use --agent <name>");
                eprintln!("Available agents:");
                for (name, def) in &agents {
                    let desc = def.description.as_deref().unwrap_or("(no description)");
                    eprintln!("  - {} : {}", name, desc);
                }
                std::process::exit(1);
            }
        }
    };

    let agent_def = config.get_agent(&agent_name).unwrap().clone();

    tracing::info!(
        "Starting {} v{} with agent '{}'",
        config.agent.name,
        config.agent.version,
        agent_name
    );
    tracing::info!(
        "Command: {} {}",
        agent_def.command,
        agent_def.args.join(" ")
    );

    // Create the process manager for the selected agent
    let process = Arc::new(LifeOsProcess::new(agent_def.clone()));

    // Create shared state
    let state = Arc::new(Mutex::new(AppState {
        config: config.clone(),
        process,
        agent_name: agent_name.clone(),
        agent_def: agent_def.clone(),
    }));

    // Create router
    let app = create_router(state);

    // Bind TCP listener
    let addr = config.socket_addr();
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Listening on http://{}", addr);
    tracing::info!(
        "Agent Card: http://{}/.well-known/agent-card.json",
        addr
    );

    // Start server
    axum::serve(listener, app).await?;

    Ok(())
}
