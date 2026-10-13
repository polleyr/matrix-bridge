use crate::config::{Config, Credentials};
use crate::error::{BridgeError, Result};
use crate::trust::apply_trust_policy;
use matrix_sdk::{
    config::SyncSettings,
    deserialized_responses::{TimelineEvent, TimelineEventKind},
    encryption::{BackupDownloadStrategy, EncryptionSettings},
    room::MessagesOptions,
    ruma::{
        events::{
            room::message::RoomMessageEventContent, AnySyncMessageLikeEvent, AnySyncTimelineEvent,
        },
        OwnedRoomId,
    },
    Client, Room,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    task::JoinHandle,
};
use tracing::{debug, info, warn};

/// A message returned from read operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub sender: String,
    pub body: String,
    pub timestamp: String,
    pub event_id: String,
    pub decryption_failed: bool,
}

/// A room returned from list operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomInfo {
    pub room_id: String,
    pub name: Option<String>,
    pub encrypted: bool,
    pub member_count: u64,
}

/// All messages of one room newer than a cutoff (result of `read_recent_messages`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomMessages {
    pub room_id: String,
    pub room_name: Option<String>,
    pub encrypted: bool,
    /// Pagination aborted early (repeated fetch failures); older messages
    /// from the requested window may be missing.
    #[serde(default)]
    pub partial: bool,
    pub messages: Vec<Message>,
}

/// Core Matrix E2EE client wrapper.
pub struct MatrixBridgeClient {
    client: Client,
    config: Config,
    has_synced: bool,
    sync_task: Option<JoinHandle<()>>,
}

impl MatrixBridgeClient {
    /// Return the login flow names advertised by the homeserver.
    pub async fn supported_login_types(config: &Config) -> Result<Vec<String>> {
        let client = Self::build_login_client(config).await?;
        let response = client
            .matrix_auth()
            .get_login_types()
            .await
            .map_err(|e| BridgeError::Matrix(e.to_string()))?;

        Ok(response
            .flows
            .iter()
            .map(|flow| flow.login_type().to_owned())
            .collect())
    }

    /// Build a client and login with password. Creates a new device and crypto store.
    pub async fn login_with_password(config: &Config, password: &str) -> Result<Self> {
        let client = Self::build_login_client(config).await?;

        info!("logging in as {}...", config.user_id);
        let login = client
            .matrix_auth()
            .login_username(&config.user_id, password)
            .initial_device_display_name(&config.device_name)
            .await
            .map_err(|e| BridgeError::LoginFailed(e.to_string()))?;

        Self::finish_login(
            config,
            client,
            login.access_token,
            login.user_id.to_string(),
            login.device_id.to_string(),
        )
        .await
    }

    /// Build a client and login through the homeserver's SSO flow.
    pub async fn login_with_sso(config: &Config) -> Result<Self> {
        let client = Self::build_login_client(config).await?;

        info!("starting SSO login for {}...", config.user_id);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let redirect_url = format!("http://{}/", listener.local_addr()?);
        let sso_url = client
            .matrix_auth()
            .get_sso_login_url(&redirect_url, None)
            .await
            .map_err(|e| BridgeError::LoginFailed(e.to_string()))?;

        println!("Opening the homeserver's SSO login in your browser...");
        if let Err(error) = open_browser(&sso_url) {
            eprintln!("Could not open a browser ({error}). Open this URL manually:\n{sso_url}");
        }

        let login_token = wait_for_sso_callback(listener).await?;
        let login = client
            .matrix_auth()
            .login_token(&login_token)
            .initial_device_display_name(&config.device_name)
            .await
            .map_err(|e| BridgeError::LoginFailed(e.to_string()))?;

        Self::finish_login(
            config,
            client,
            login.access_token,
            login.user_id.to_string(),
            login.device_id.to_string(),
        )
        .await
    }

    async fn build_login_client(config: &Config) -> Result<Client> {
        config.ensure_store_dir()?;
        Client::builder()
            .homeserver_url(&config.homeserver)
            .with_encryption_settings(EncryptionSettings {
                backup_download_strategy: BackupDownloadStrategy::AfterDecryptionFailure,
                ..Default::default()
            })
            .sqlite_store(&config.store_path, None)
            .build()
            .await
            .map_err(|e| BridgeError::Matrix(e.to_string()))
    }

    async fn finish_login(
        config: &Config,
        client: Client,
        access_token: String,
        user_id: String,
        device_id: String,
    ) -> Result<Self> {
        info!("logged in, device_id={device_id}");

        Credentials {
            access_token,
            user_id,
            device_id,
        }
        .save(config)?;

        info!("running initial sync...");
        client
            .sync_once(SyncSettings::default())
            .await
            .map_err(|e| BridgeError::SyncFailed(e.to_string()))?;

        apply_trust_policy(&client, &config.trust_mode).await;

        Ok(Self {
            client,
            config: config.clone(),
            has_synced: true,
            sync_task: None,
        })
    }

    /// Restore a previously saved session.
    pub async fn restore(config: &Config) -> Result<Self> {
        config.ensure_store_dir()?;
        let creds = Credentials::load(config)?;

        let client = Client::builder()
            .homeserver_url(&config.homeserver)
            .with_encryption_settings(EncryptionSettings {
                backup_download_strategy: BackupDownloadStrategy::AfterDecryptionFailure,
                ..Default::default()
            })
            .sqlite_store(&config.store_path, None)
            .build()
            .await
            .map_err(|e| BridgeError::Matrix(e.to_string()))?;

        let session =
            matrix_sdk::authentication::matrix::MatrixSession {
                meta: matrix_sdk::SessionMeta {
                    user_id: creds.user_id.as_str().try_into().map_err(
                        |e: matrix_sdk::IdParseError| BridgeError::Config(e.to_string()),
                    )?,
                    device_id: creds.device_id.as_str().into(),
                },
                tokens: matrix_sdk::SessionTokens {
                    access_token: creds.access_token,
                    refresh_token: None,
                },
            };

        client
            .restore_session(session)
            .await
            .map_err(|e| BridgeError::Matrix(e.to_string()))?;

        debug!("session restored for {}", config.user_id);

        Ok(Self {
            client,
            config: config.clone(),
            has_synced: false,
            sync_task: None,
        })
    }

    /// Restore the account's room-key backup from its recovery key/passphrase.
    /// Returns whether the backup key was accepted and the backup is enabled.
    pub async fn recover_key_backup(&self, recovery_key: &str) -> Result<bool> {
        self.client
            .encryption()
            .recovery()
            .recover(recovery_key)
            .await
            .map_err(|e| BridgeError::Matrix(e.to_string()))?;

        Ok(self.client.encryption().backups().are_enabled().await)
    }

    /// Check whether the homeserver currently has a room-key backup.
    pub async fn has_key_backup(&self) -> Result<bool> {
        self.client
            .encryption()
            .backups()
            .fetch_exists_on_server()
            .await
            .map_err(|e| BridgeError::Matrix(e.to_string()))
    }

    /// Run a single sync cycle. First call does full state.
    pub async fn sync_once(&mut self) -> Result<()> {
        let settings = if self.has_synced {
            SyncSettings::default().timeout(Duration::from_secs(10))
        } else {
            SyncSettings::default()
        };

        self.client
            .sync_once(settings)
            .await
            .map_err(|e| BridgeError::SyncFailed(e.to_string()))?;

        if !self.has_synced {
            apply_trust_policy(&self.client, &self.config.trust_mode).await;
            self.has_synced = true;
        }

        Ok(())
    }

    /// Start a background sync loop (for MCP server).
    pub async fn start_sync(&mut self) -> Result<()> {
        self.sync_once().await?;

        let client = self.client.clone();
        let trust_mode = self.config.trust_mode.clone();

        let handle = tokio::spawn(async move {
            loop {
                let settings = SyncSettings::default().timeout(Duration::from_secs(30));
                match client.sync_once(settings).await {
                    Ok(_) => {
                        apply_trust_policy(&client, &trust_mode).await;
                    }
                    Err(e) => {
                        warn!("sync error: {}, retrying in 5s", e);
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        });

        self.sync_task = Some(handle);
        Ok(())
    }

    /// Send a message to a room, optionally with an @mention.
    pub async fn send_message(
        &self,
        room_id: &str,
        body: &str,
        mention: Option<&str>,
    ) -> Result<String> {
        let room = self.get_room(room_id)?;

        let content = if let Some(mention_user) = mention {
            let display = mention_user
                .strip_prefix('@')
                .and_then(|s| s.split(':').next())
                .unwrap_or(mention_user);
            let html = format!(
                "<a href=\"https://matrix.to/#/{}\">{}</a> {}",
                mention_user, display, body
            );
            let plain = format!("{} {}", display, body);
            RoomMessageEventContent::text_html(plain, html)
        } else {
            RoomMessageEventContent::text_plain(body)
        };

        let response = room
            .send(content)
            .await
            .map_err(|e| BridgeError::SendFailed(e.to_string()))?;

        Ok(response.event_id.to_string())
    }

    /// Read recent messages from a room.
    /// Uses room.messages() which auto-decrypts E2EE events via the SDK's olm machine.
    pub async fn read_messages(&self, room_id: &str, limit: u32) -> Result<Vec<Message>> {
        let room = self.get_room(room_id)?;

        let mut options = MessagesOptions::backward();
        options.limit = limit.into();

        let response = room
            .messages(options)
            .await
            .map_err(|e| BridgeError::Matrix(e.to_string()))?;

        let mut messages = Vec::new();

        for timeline_event in &response.chunk {
            if let Some((_, message)) = extract_message(timeline_event) {
                messages.push(message);
            }
        }

        messages.reverse(); // chronological order
        Ok(messages)
    }

    /// Read all messages newer than `since_ms` (epoch millis) from every
    /// joined room — nothing is trimmed. Rooms are fetched concurrently and
    /// each room's history is paginated backwards until the cutoff is reached.
    /// Rooms without matching messages are omitted (unless a fetch failed
    /// part-way, in which case they are included and flagged `partial`); the
    /// result is sorted most-recently-active first.
    pub async fn read_recent_messages(&self, since_ms: i64) -> Vec<RoomMessages> {
        const CONCURRENCY: usize = 8;

        let semaphore = Arc::new(tokio::sync::Semaphore::new(CONCURRENCY));
        let mut set: tokio::task::JoinSet<(i64, RoomMessages)> = tokio::task::JoinSet::new();

        for room in self.client.rooms() {
            let room = room.clone();
            let semaphore = Arc::clone(&semaphore);
            set.spawn(async move {
                let _permit = semaphore.acquire_owned().await.expect("semaphore closed");
                fetch_room_messages_since(&room, since_ms).await
            });
        }

        let mut rooms: Vec<(i64, RoomMessages)> = Vec::new();
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((newest_ms, room_messages)) => {
                    if !room_messages.messages.is_empty() || room_messages.partial {
                        rooms.push((newest_ms, room_messages));
                    }
                }
                Err(e) => warn!("room history fetch task failed: {e}"),
            }
        }

        rooms.sort_by_key(|(newest_ms, _)| std::cmp::Reverse(*newest_ms));
        rooms.into_iter().map(|(_, r)| r).collect()
    }

    /// List joined rooms.
    pub async fn get_rooms(&self) -> Vec<RoomInfo> {
        let mut rooms = Vec::new();
        for room in self.client.rooms() {
            rooms.push(RoomInfo {
                room_id: room.room_id().to_string(),
                name: room.name(),
                encrypted: room
                    .latest_encryption_state()
                    .await
                    .map(|s| s.is_encrypted())
                    .unwrap_or(false),
                member_count: room.joined_members_count(),
            });
        }
        rooms
    }

    /// Join a room by ID or alias.
    pub async fn join_room(&self, room_id_or_alias: &str) -> Result<String> {
        let room = if room_id_or_alias.starts_with('!') {
            let id: OwnedRoomId = room_id_or_alias
                .try_into()
                .map_err(|_| BridgeError::RoomNotFound(room_id_or_alias.to_string()))?;
            self.client
                .join_room_by_id(&id)
                .await
                .map_err(|e| BridgeError::Matrix(e.to_string()))?
        } else {
            let alias = matrix_sdk::ruma::RoomOrAliasId::parse(room_id_or_alias)
                .map_err(|_| BridgeError::RoomNotFound(room_id_or_alias.to_string()))?;
            self.client
                .join_room_by_id_or_alias(&alias, &[])
                .await
                .map_err(|e| BridgeError::Matrix(e.to_string()))?
        };

        Ok(room.room_id().to_string())
    }

    /// Send a message and wait for a reply from another user.
    /// Polls with increasing window to handle chatty rooms.
    pub async fn send_and_wait(
        &self,
        room_id: &str,
        body: &str,
        mention: Option<&str>,
        timeout_secs: u64,
    ) -> Result<Option<Message>> {
        let event_id = self.send_message(room_id, body, mention).await?;
        let own_user = self
            .client
            .user_id()
            .map(|u| u.to_string())
            .unwrap_or_default();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        let mut poll_window: u32 = 20;

        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_secs(2)).await;

            let msgs = self.read_messages(room_id, poll_window.min(100)).await?;
            let mut found_ours = false;
            for msg in &msgs {
                if msg.event_id == event_id {
                    found_ours = true;
                    continue;
                }
                if found_ours && msg.sender != own_user && !msg.decryption_failed {
                    return Ok(Some(msg.clone()));
                }
            }

            // Widen the window if our message is scrolling out
            if !found_ours && poll_window < 100 {
                poll_window = (poll_window + 20).min(100);
            }
        }

        Ok(None)
    }

    /// Get the inner matrix-sdk Client (for MCP event handlers).
    pub fn inner(&self) -> &Client {
        &self.client
    }

    /// Get the config.
    pub fn config(&self) -> &Config {
        &self.config
    }

    fn get_room(&self, room_id: &str) -> Result<Room> {
        let id: OwnedRoomId = room_id
            .try_into()
            .map_err(|_| BridgeError::RoomNotFound(room_id.to_string()))?;
        self.client
            .get_room(&id)
            .ok_or_else(|| BridgeError::RoomNotFound(room_id.to_string()))
    }
}

/// Extract a chat message and its origin-server timestamp (epoch millis) from
/// a timeline event. Returns None for non-message events and redactions.
fn extract_message(timeline_event: &TimelineEvent) -> Option<(i64, Message)> {
    let decryption_failed = matches!(
        &timeline_event.kind,
        TimelineEventKind::UnableToDecrypt { .. }
    );

    // .raw() returns the decrypted event for Decrypted, or the raw event for PlainText/UTD
    match timeline_event.kind.raw().deserialize() {
        Ok(AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(msg))) => {
            let ts = msg.origin_server_ts();
            let millis = i64::from(ts.0);

            let original = msg.as_original()?;

            Some((
                millis,
                Message {
                    sender: msg.sender().to_string(),
                    body: original.content.body().to_string(),
                    timestamp: format_timestamp(millis),
                    event_id: msg.event_id().to_string(),
                    decryption_failed: false,
                },
            ))
        }
        Ok(AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomEncrypted(enc))) => {
            let ts = enc.origin_server_ts();
            let millis = i64::from(ts.0);

            Some((
                millis,
                Message {
                    sender: enc.sender().to_string(),
                    body: "[encrypted message — unable to decrypt]".to_string(),
                    timestamp: format_timestamp(millis),
                    event_id: enc.event_id().to_string(),
                    decryption_failed: true,
                },
            ))
        }
        Err(_) if decryption_failed => Some((
            0,
            Message {
                sender: "unknown".to_string(),
                body: "[encrypted message — unable to decrypt]".to_string(),
                timestamp: String::new(),
                event_id: String::new(),
                decryption_failed: true,
            },
        )),
        Err(e) => {
            debug!("failed to deserialize event: {}", e);
            None
        }
        _ => None, // skip non-message events
    }
}

/// Paginate a single room's history backwards, collecting ALL messages newer
/// than `since_ms` — no trimming, no page cap. Pagination continues until an
/// event older than the cutoff is hit or the start of the room history is
/// reached; repeated fetch failures abort it (the room is then flagged
/// `partial`). Returns the newest matching timestamp alongside the room's
/// messages for sorting.
async fn fetch_room_messages_since(room: &Room, since_ms: i64) -> (i64, RoomMessages) {
    const PAGE_SIZE: u32 = 100;

    let room_name = room.name();
    let encrypted = room
        .latest_encryption_state()
        .await
        .map(|state| state.is_encrypted())
        .unwrap_or(false);

    let mut messages: Vec<Message> = Vec::new();
    let mut newest_ms: i64 = 0;
    let mut from: Option<String> = None;
    let mut partial = false;

    'pagination: loop {
        let Some(response) = fetch_messages_page(room, from.as_deref(), PAGE_SIZE).await else {
            partial = true;
            break;
        };

        if response.chunk.is_empty() {
            break;
        }

        // chunk arrives newest → oldest.
        for timeline_event in &response.chunk {
            let Some((ts_ms, message)) = extract_message(timeline_event) else {
                continue;
            };
            if ts_ms == 0 {
                continue; // undated placeholder; skip instead of treating as cutoff
            }
            if ts_ms < since_ms {
                // Everything after this event is older than the cutoff.
                break 'pagination;
            }
            if messages.is_empty() {
                newest_ms = ts_ms;
            }
            messages.push(message);
        }

        match response.end {
            Some(token) => {
                if from.as_deref() == Some(token.as_str()) {
                    // No progress; avoid refetching the same page forever.
                    break;
                }
                from = Some(token);
            }
            None => break, // start of history reached
        }
    }

    messages.reverse(); // restore chronological order

    (
        newest_ms,
        RoomMessages {
            room_id: room.room_id().to_string(),
            room_name,
            encrypted,
            partial,
            messages,
        },
    )
}

/// Fetch one page of a room's message history, retrying transient failures a
/// few times before giving up. Only room-message and encrypted events are
/// requested, so pagination never wades through state-event noise (joins,
/// leaves, power levels) in large rooms.
async fn fetch_messages_page(
    room: &Room,
    from: Option<&str>,
    limit: u32,
) -> Option<matrix_sdk::room::Messages> {
    const MAX_RETRIES: u32 = 3;

    for attempt in 1..=MAX_RETRIES {
        let mut options = MessagesOptions::backward();
        if let Some(token) = from {
            options.from = Some(token.to_owned());
        }
        options.limit = limit.into();
        options.filter.types = Some(vec![
            "m.room.message".to_owned(),
            "m.room.encrypted".to_owned(),
        ]);

        match room.messages(options).await {
            Ok(response) => return Some(response),
            Err(e) => {
                warn!("messages request failed (attempt {attempt}): {e}");
                if attempt < MAX_RETRIES {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }

    None
}

/// Parse a cutoff timestamp: either RFC 3339 (e.g. "2026-09-29T09:00:00+02:00")
/// or a plain date (e.g. "2026-09-29", interpreted as local midnight).
/// Returns epoch millis.
pub fn parse_since_cutoff(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()?
        .and_hms_opt(0, 0, 0)?
        .and_local_timezone(chrono::Local)
        .single()
        .map(|dt| dt.timestamp_millis())
}

/// The default cutoff when none is given: 24 hours ago (epoch millis).
pub fn default_since_ms() -> i64 {
    chrono::Utc::now().timestamp_millis() - 24 * 60 * 60 * 1000
}

/// Format epoch millis as an RFC 3339 timestamp in the local timezone.
pub fn format_timestamp(millis: i64) -> String {
    chrono::DateTime::from_timestamp_millis(millis)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        })
        .unwrap_or_default()
}

fn open_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(url);
        command
    };

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("rundll32");
        command.args(["url.dll,FileProtocolHandler", url]);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };

    #[cfg(not(any(unix, target_os = "windows")))]
    let mut command = {
        let _ = url;
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "automatic browser opening is not supported on this platform",
        ));
    };

    command.spawn().map(|_| ())
}

async fn wait_for_sso_callback(listener: TcpListener) -> Result<String> {
    loop {
        let (stream, _) = listener.accept().await?;
        let mut stream = BufReader::new(stream);
        let mut request_line = String::new();
        stream.read_line(&mut request_line).await?;

        let mut header = String::new();
        loop {
            header.clear();
            if stream.read_line(&mut header).await? == 0 || header == "\r\n" || header == "\n" {
                break;
            }
        }

        let mut request_parts = request_line.split_whitespace();
        let request_target = (request_parts.next() == Some("GET"))
            .then(|| request_parts.next())
            .flatten();
        let query =
            request_target.and_then(|target| target.split_once('?').map(|(_, query)| query));
        let params = query
            .and_then(|query| serde_html_form::from_str::<HashMap<String, String>>(query).ok());
        let login_token = params
            .as_ref()
            .and_then(|params| params.get("loginToken"))
            .cloned();

        if let Some(token) = login_token.filter(|token| !token.is_empty()) {
            respond_to_sso_callback(
                &mut stream,
                "200 OK",
                "SSO login complete. You can close this tab.",
            )
            .await?;
            return Ok(token);
        }

        if let Some(error) = params.and_then(|params| params.get("error").cloned()) {
            respond_to_sso_callback(
                &mut stream,
                "400 Bad Request",
                "SSO login failed. You can close this tab.",
            )
            .await?;
            return Err(BridgeError::LoginFailed(format!(
                "SSO provider returned {error}"
            )));
        }

        respond_to_sso_callback(
            &mut stream,
            "400 Bad Request",
            "Waiting for a valid Matrix SSO callback.",
        )
        .await?;
    }
}

async fn respond_to_sso_callback(
    stream: &mut BufReader<tokio::net::TcpStream>,
    status: &str,
    body: &str,
) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.get_mut().write_all(response.as_bytes()).await
}

impl Drop for MatrixBridgeClient {
    fn drop(&mut self) {
        if let Some(handle) = self.sync_task.take() {
            handle.abort();
        }
    }
}
