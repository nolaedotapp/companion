use crate::companion::{AgentSnapshot, Command};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::io::{self, BufRead, Read, Write};

pub const VERSION: u8 = 1;
const MAX_MESSAGE: u64 = 16_384;

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum ClientMessage {
    Command(Command),
    Restart,
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum ServerMessage {
    Hello {
        protocol: u8,
        version: String,
        snapshot: AgentSnapshot,
    },
    Snapshot(AgentSnapshot),
    Activate,
    Shutdown,
    Error(String),
}

pub fn read_message<T: DeserializeOwned>(reader: &mut impl BufRead) -> io::Result<Option<T>> {
    let mut bytes = Vec::new();
    let length = reader.take(MAX_MESSAGE + 1).read_until(b'\n', &mut bytes)?;
    if length == 0 {
        return Ok(None);
    }
    if length as u64 > MAX_MESSAGE || bytes.last() != Some(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid UI message size",
        ));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid UI message"))
}

pub fn write_message(writer: &mut impl Write, message: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(message)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Could not encode UI message"))?;
    if bytes.len() as u64 >= MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "UI message is too large",
        ));
    }
    writer.write_all(&bytes)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn framing_preserves_messages_and_rejects_oversized_or_incomplete_input() {
        let mut bytes = Vec::new();
        write_message(
            &mut bytes,
            &ClientMessage::Command(Command::Pair("a".repeat(24))),
        )
        .unwrap();
        write_message(&mut bytes, &ClientMessage::Restart).unwrap();
        let mut reader = Cursor::new(bytes);
        assert!(
            matches!(read_message(&mut reader).unwrap(), Some(ClientMessage::Command(Command::Pair(code))) if code.len() == 24)
        );
        assert!(matches!(
            read_message(&mut reader).unwrap(),
            Some(ClientMessage::Restart)
        ));
        assert!(
            read_message::<ClientMessage>(&mut reader)
                .unwrap()
                .is_none()
        );
        assert!(
            read_message::<ClientMessage>(&mut Cursor::new(vec![b'x'; MAX_MESSAGE as usize + 1]))
                .is_err()
        );
        assert!(
            read_message::<ClientMessage>(&mut Cursor::new(b"{\"type\":\"restart\"}")).is_err()
        );
        assert!(
            read_message::<ClientMessage>(&mut Cursor::new(
                b"{\"type\":\"restart\",\"token\":\"unexpected\"}\n"
            ))
            .is_err()
        );
    }
}
