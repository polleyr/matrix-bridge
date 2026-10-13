use crate::client::{Message, RoomInfo, RoomMessages};

fn message_line(m: &Message) -> String {
    if m.decryption_failed {
        format!("[{}] [encrypted — unable to decrypt]", m.timestamp)
    } else {
        let sender = m
            .sender
            .strip_prefix('@')
            .and_then(|s| s.split(':').next())
            .unwrap_or(&m.sender);
        format!("[{}] {}: {}", m.timestamp, sender, m.body)
    }
}

pub fn format_messages(messages: &[Message], json: bool) -> String {
    if json {
        serde_json::to_string_pretty(messages).unwrap_or_default()
    } else {
        messages
            .iter()
            .map(message_line)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub fn format_recent_rooms(rooms: &[RoomMessages], json: bool) -> String {
    if json {
        serde_json::to_string_pretty(rooms).unwrap_or_default()
    } else {
        rooms
            .iter()
            .map(|r| {
                let mut header = format!(
                    "── {} ({})",
                    r.room_name.as_deref().unwrap_or("(unnamed)"),
                    r.room_id
                );
                if r.partial {
                    header.push_str(" [partial]");
                }
                let lines: Vec<String> = r.messages.iter().map(message_line).collect();
                std::iter::once(header)
                    .chain(lines)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

pub fn format_rooms(rooms: &[RoomInfo], json: bool) -> String {
    if json {
        serde_json::to_string_pretty(rooms).unwrap_or_default()
    } else {
        rooms
            .iter()
            .map(|r| {
                let name = r.name.as_deref().unwrap_or("(unnamed)");
                let enc = if r.encrypted { "🔒" } else { "  " };
                format!("{} {} {} ({})", enc, r.room_id, name, r.member_count)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
