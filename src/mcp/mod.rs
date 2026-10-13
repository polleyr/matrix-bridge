pub mod notifications;
pub mod tools;

use crate::client::MatrixBridgeClient;
use crate::config::Config;
use crate::error::Result;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

/// Run the MCP server over stdin/stdout.
/// Starts the MCP server IMMEDIATELY so the client gets an initialize response,
/// then syncs in the background. Tools wait for sync to complete on first use.
pub async fn run_server() -> Result<()> {
    let config = Config::load()?;
    let mention_pattern = config.mention_pattern();
    let client = MatrixBridgeClient::restore(&config).await?;
    let client = Arc::new(RwLock::new(client));

    // Spawn sync in background — don't block MCP startup
    let sync_client = Arc::clone(&client);
    tokio::spawn(async move {
        info!("starting background sync...");
        let mut c = sync_client.write().await;
        if let Err(e) = c.start_sync().await {
            tracing::error!("sync failed: {}", e);
        }
        drop(c);
        info!("background sync started");
    });

    // Register mention detection handler
    let notify_client = Arc::clone(&client);
    tokio::spawn(async move {
        // Wait a moment for sync to initialize
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        notifications::register_mention_handler(&notify_client, mention_pattern).await;
    });

    info!("MCP server starting on stdio");
    tools::serve(client).await?;

    Ok(())
}
