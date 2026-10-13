#[cfg(feature = "cli")]
use clap::{Parser, Subcommand};

#[cfg(feature = "cli")]
#[derive(Parser)]
#[command(
    name = "matrix-bridge",
    about = "E2EE Matrix bridge — CLI and MCP server"
)]
pub struct Cli {
    /// Output in JSON format
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[cfg(feature = "cli")]
#[derive(Subcommand)]
pub enum Commands {
    /// Interactive setup: login, create device, save config
    Setup,

    /// Restore historical room keys from the account's server-side backup
    Recover,

    /// Send a message to a room
    Send {
        /// Message text
        message: String,

        /// Room ID (overrides default_room)
        #[arg(short, long)]
        room: Option<String>,

        /// User ID to @mention
        #[arg(short, long)]
        mention: Option<String>,

        /// Suppress default mention
        #[arg(long)]
        no_mention: bool,
    },

    /// Read recent messages from a room
    Read {
        /// Room ID (overrides default_room)
        #[arg(short, long)]
        room: Option<String>,

        /// Number of messages (1-100)
        #[arg(short, long, default_value = "10", value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },

    /// Read all messages from all rooms since a cutoff time
    Recent {
        /// Cutoff: RFC 3339 timestamp or YYYY-MM-DD date (default: last 24 hours)
        #[arg(long)]
        since: Option<String>,
    },

    /// List joined rooms
    Rooms,

    /// Send a message and wait for a reply
    SendWait {
        /// Message text
        message: String,

        /// Room ID (overrides default_room)
        #[arg(short, long)]
        room: Option<String>,

        /// User ID to @mention
        #[arg(short, long)]
        mention: Option<String>,

        /// Suppress default mention
        #[arg(long)]
        no_mention: bool,

        /// Timeout in seconds (1-300)
        #[arg(short, long, default_value = "30", value_parser = clap::value_parser!(u64).range(1..=300))]
        timeout: u64,
    },

    /// View or set config values
    Config {
        /// Config key to view or set
        key: Option<String>,

        /// Value to set
        value: Option<String>,
    },

    /// Start the MCP server (stdin/stdout)
    #[cfg(feature = "mcp")]
    McpServer,
}
