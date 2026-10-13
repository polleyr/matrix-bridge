use crate::client::MatrixBridgeClient;
use matrix_sdk::{ruma::events::room::message::OriginalSyncRoomMessageEvent, Room};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Register a Matrix event handler that detects mentions and could emit
/// MCP channel notifications in the future.
///
/// Currently logs mention detections. Full MCP notification support requires
/// access to the rmcp session/transport to send JSON-RPC notifications,
/// which is not yet exposed by the rmcp SDK for server-initiated messages.
pub async fn register_mention_handler(
    client: &Arc<RwLock<MatrixBridgeClient>>,
    mention_pattern: String,
) {
    let inner = {
        let c = client.read().await;
        c.inner().clone()
    };

    let own_user_id = inner.user_id().map(|u| u.to_string()).unwrap_or_default();
    let pattern = mention_pattern.to_lowercase();

    inner.add_event_handler(move |event: OriginalSyncRoomMessageEvent, room: Room| {
        let pattern = pattern.clone();
        let own_user_id = own_user_id.clone();
        async move {
            // Skip own messages
            if event.sender.as_str() == own_user_id {
                return;
            }

            let body = match &event.content.msgtype {
                matrix_sdk::ruma::events::room::message::MessageType::Text(text) => &text.body,
                _ => return,
            };

            // Check if the message mentions the configured pattern
            if body.to_lowercase().contains(&pattern) {
                let sender = event.sender.as_str();
                let room_id = room.room_id().to_string();
                info!(
                    "[matrix-bridge] mention detected: room={}, sender={}, preview=\"{}\"",
                    room_id,
                    sender,
                    &body[..body.len().min(60)]
                );

                // TODO: When rmcp exposes server-to-client notification API,
                // send a channel notification here:
                // session.send_notification("notifications/message", {
                //   content: body,
                //   chat_id: room_id,
                //   user: sender,
                //   ...
                // })
            }
        }
    });

    debug!("mention handler registered (pattern={})", mention_pattern);
}
