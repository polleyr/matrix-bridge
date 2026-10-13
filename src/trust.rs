use crate::config::TrustMode;
use matrix_sdk::Client;
use tracing::{debug, warn};

/// Apply the configured device trust policy.
/// - Tofu/All: auto-verify all unverified devices for all known users.
/// - Explicit: do nothing (user must verify manually).
pub async fn apply_trust_policy(client: &Client, mode: &TrustMode) {
    match mode {
        TrustMode::Explicit => {
            debug!("trust mode: explicit — skipping auto-verification");
        }
        TrustMode::Tofu | TrustMode::All => {
            debug!("trust mode: {:?} — auto-verifying devices", mode);
            let user_id = client.user_id().expect("client must be logged in");

            // Verify devices of all tracked users
            let encryption = client.encryption();
            for room in client.rooms() {
                for member in room
                    .members_no_sync(matrix_sdk::RoomMemberships::ACTIVE)
                    .await
                    .unwrap_or_default()
                {
                    if member.user_id() == user_id {
                        continue;
                    }
                    match encryption.get_user_devices(member.user_id()).await {
                        Ok(devices) => {
                            for device in devices.devices() {
                                if !device.is_verified() {
                                    if let Err(e) = device.verify().await {
                                        warn!(
                                            "failed to verify device {} for {}: {}",
                                            device.device_id(),
                                            member.user_id(),
                                            e
                                        );
                                    } else {
                                        debug!(
                                            "verified device {} for {}",
                                            device.device_id(),
                                            member.user_id()
                                        );
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            warn!("failed to get devices for {}: {}", member.user_id(), e);
                        }
                    }
                }
            }
        }
    }
}
