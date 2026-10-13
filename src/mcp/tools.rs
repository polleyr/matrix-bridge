use crate::client::MatrixBridgeClient;
use rmcp::{
    handler::server::router::{tool::ToolRouter, Router},
    handler::server::wrapper::{Json, Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_router, ServerHandler, ServiceExt,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct BridgeServer {
    client: Arc<RwLock<MatrixBridgeClient>>,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

// --- Tool parameter/output types ---

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SendMessageParams {
    /// The Matrix room ID (e.g. !abc123:matrix.org)
    pub room_id: String,
    /// The message text to send
    pub message: String,
    /// Optional user ID to @mention (e.g. @user:matrix.org)
    #[serde(default)]
    pub mention: Option<String>,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct SendMessageOutput {
    pub event_id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SendAndWaitParams {
    /// The Matrix room ID
    pub room_id: String,
    /// The message text to send
    pub message: String,
    /// Optional user ID to @mention
    #[serde(default)]
    pub mention: Option<String>,
    /// Timeout in seconds (default: 30)
    #[serde(default = "default_timeout")]
    #[schemars(schema_with = "unsigned_integer_schema")]
    pub timeout: u64,
}

fn default_timeout() -> u64 {
    30
}

fn unsigned_integer_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    let mut schema = <u64 as schemars::JsonSchema>::json_schema(generator);
    schema.remove("format");
    schema
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct SendAndWaitOutput {
    pub reply: Option<MessageOutput>,
    pub timed_out: bool,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct MessageOutput {
    pub sender: String,
    pub body: String,
    pub timestamp: String,
    pub event_id: String,
    pub decryption_failed: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReadMessagesParams {
    /// The Matrix room ID
    pub room_id: String,
    /// Number of messages to read (default: 20)
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_limit() -> u32 {
    20
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct ReadMessagesOutput {
    pub messages: Vec<MessageOutput>,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct ListRoomsOutput {
    pub rooms: Vec<RoomOutput>,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct RoomOutput {
    pub room_id: String,
    pub name: Option<String>,
    pub encrypted: bool,
    #[schemars(schema_with = "unsigned_integer_schema")]
    pub member_count: u64,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct JoinRoomParams {
    /// Room ID or alias to join
    pub room_id: String,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct JoinRoomOutput {
    pub room_id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReadRecentMessagesParams {
    /// Cutoff time as an RFC 3339 timestamp (e.g. "2026-09-29T09:00:00+02:00")
    /// or a plain date (e.g. "2026-09-29", interpreted as local midnight).
    /// Defaults to 24 hours ago. ALL messages newer than this are returned.
    pub since: Option<String>,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct RecentRoomOutput {
    pub room_id: String,
    pub room_name: Option<String>,
    pub encrypted: bool,
    /// Pagination for this room aborted early; older messages from the
    /// requested window may be missing.
    pub partial: bool,
    pub messages: Vec<MessageOutput>,
}

#[derive(Serialize, schemars::JsonSchema)]
pub struct ReadRecentMessagesOutput {
    /// The cutoff that was applied, as an RFC 3339 timestamp.
    pub since: String,
    /// Rooms with matching messages, most recently active first.
    pub rooms: Vec<RecentRoomOutput>,
}

// --- Tool implementations ---

#[tool_router]
impl BridgeServer {
    #[tool(
        name = "send_message",
        description = "Send a message to a Matrix room. Automatically encrypted if the room has E2EE enabled."
    )]
    async fn send_message(
        &self,
        Parameters(params): Parameters<SendMessageParams>,
    ) -> std::result::Result<Json<SendMessageOutput>, String> {
        let client = self.client.read().await;
        let event_id = client
            .send_message(&params.room_id, &params.message, params.mention.as_deref())
            .await
            .map_err(|e| e.to_string())?;
        Ok(Json(SendMessageOutput { event_id }))
    }

    #[tool(
        name = "send_and_wait",
        description = "Send a message and wait for a reply from another user. Returns the first reply or times out."
    )]
    async fn send_and_wait(
        &self,
        Parameters(params): Parameters<SendAndWaitParams>,
    ) -> std::result::Result<Json<SendAndWaitOutput>, String> {
        let client = self.client.read().await;
        let timeout = params.timeout.clamp(1, 300);
        let result = client
            .send_and_wait(
                &params.room_id,
                &params.message,
                params.mention.as_deref(),
                timeout,
            )
            .await
            .map_err(|e| e.to_string())?;

        let timed_out = result.is_none();
        Ok(Json(SendAndWaitOutput {
            reply: result.map(|msg| MessageOutput {
                sender: msg.sender,
                body: msg.body,
                timestamp: msg.timestamp,
                event_id: msg.event_id,
                decryption_failed: msg.decryption_failed,
            }),
            timed_out,
        }))
    }

    #[tool(
        name = "read_messages",
        description = "Read recent messages from a Matrix room, decrypting E2EE messages automatically."
    )]
    async fn read_messages(
        &self,
        Parameters(params): Parameters<ReadMessagesParams>,
    ) -> std::result::Result<Json<ReadMessagesOutput>, String> {
        let client = self.client.read().await;
        let limit = params.limit.clamp(1, 100);
        let messages = client
            .read_messages(&params.room_id, limit)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|m| MessageOutput {
                sender: m.sender,
                body: m.body,
                timestamp: m.timestamp,
                event_id: m.event_id,
                decryption_failed: m.decryption_failed,
            })
            .collect();
        Ok(Json(ReadMessagesOutput { messages }))
    }

    #[tool(
        name = "list_rooms",
        description = "List all Matrix rooms the bridge has joined."
    )]
    async fn list_rooms(&self) -> Json<ListRoomsOutput> {
        let client = self.client.read().await;
        let rooms = client
            .get_rooms()
            .await
            .into_iter()
            .map(|r| RoomOutput {
                room_id: r.room_id,
                name: r.name,
                encrypted: r.encrypted,
                member_count: r.member_count,
            })
            .collect();
        Json(ListRoomsOutput { rooms })
    }

    #[tool(
        name = "join_room",
        description = "Join a Matrix room by room ID or alias."
    )]
    async fn join_room(
        &self,
        Parameters(params): Parameters<JoinRoomParams>,
    ) -> std::result::Result<Json<JoinRoomOutput>, String> {
        let client = self.client.read().await;
        let room_id = client
            .join_room(&params.room_id)
            .await
            .map_err(|e| e.to_string())?;
        Ok(Json(JoinRoomOutput { room_id }))
    }

    #[tool(
        name = "read_recent_messages",
        description = "Read ALL messages from all joined rooms since a cutoff time (default: last \
            24 hours) — nothing is trimmed and there is no page cap. Each room's history is \
            paginated backwards until the cutoff is reached, decrypting E2EE messages automatically. \
            Returns rooms with matching messages, most recently active first, with each room's \
            messages in chronological order. Rooms flagged `partial` hit repeated fetch failures \
            and may be missing older messages."
    )]
    async fn read_recent_messages(
        &self,
        Parameters(params): Parameters<ReadRecentMessagesParams>,
    ) -> std::result::Result<Json<ReadRecentMessagesOutput>, String> {
        let client = self.client.read().await;

        let since_ms = match params.since.as_deref() {
            Some(s) => crate::client::parse_since_cutoff(s).ok_or_else(|| {
                format!(
                    "invalid since value '{s}': expected an RFC 3339 timestamp \
                     (e.g. 2026-09-29T09:00:00+02:00) or a date (e.g. 2026-09-29)"
                )
            })?,
            None => crate::client::default_since_ms(),
        };

        let rooms = client.read_recent_messages(since_ms).await;

        Ok(Json(ReadRecentMessagesOutput {
            since: crate::client::format_timestamp(since_ms),
            rooms: rooms
                .into_iter()
                .map(|r| RecentRoomOutput {
                    room_id: r.room_id,
                    room_name: r.room_name,
                    encrypted: r.encrypted,
                    partial: r.partial,
                    messages: r
                        .messages
                        .into_iter()
                        .map(|m| MessageOutput {
                            sender: m.sender,
                            body: m.body,
                            timestamp: m.timestamp,
                            event_id: m.event_id,
                            decryption_failed: m.decryption_failed,
                        })
                        .collect(),
                })
                .collect(),
        }))
    }
}

impl ServerHandler for BridgeServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .build(),
        )
        .with_instructions("E2EE Matrix bridge. Send and read encrypted messages in Matrix rooms. Messages are automatically encrypted/decrypted.")
    }
}

impl BridgeServer {
    pub fn new(client: Arc<RwLock<MatrixBridgeClient>>) -> Router<Self> {
        let server = Self {
            client,
            tool_router: ToolRouter::new(),
        };
        let mut router = Router::new(server);
        router.tool_router = Self::tool_router();
        router
    }
}

/// Start the MCP server on stdio.
pub async fn serve(client: Arc<RwLock<MatrixBridgeClient>>) -> crate::error::Result<()> {
    use rmcp::transport::io::stdio;

    let router = BridgeServer::new(client);
    let transport = stdio();

    router
        .serve(transport)
        .await
        .map_err(|e| crate::error::BridgeError::Other(format!("MCP server error: {}", e)))?
        .waiting()
        .await
        .map_err(|e| crate::error::BridgeError::Other(format!("MCP server error: {}", e)))?;

    Ok(())
}
