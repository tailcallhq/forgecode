use forge_domain::{Conversation, ConversationId};
use serde::{Deserialize, Serialize};
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    UpsertConversation { conversation: Conversation },
    UpsertConversationRef { conversation: Conversation },
    UpdateParentId {
        conversation_id: ConversationId,
        new_parent_id: Option<ConversationId>,
    },
    DeleteConversation { conversation_id: ConversationId },
    OptimizeFts,
    RefreshFts,
    CheckpointWal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    Ack,
    Error { message: String },
}

/// Async length-prefixed frame writer: writes u32 length prefix + serialized data
pub async fn write_frame<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    value: &T,
) -> io::Result<()> {
    let serialized = bincode::serialize(value)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("bincode error: {e}")))?;
    let len = serialized.len() as u32;
    writer.write_all(&len.to_le_bytes()).await?;
    writer.write_all(&serialized).await?;
    Ok(())
}

/// Async length-prefixed frame reader: reads u32 length prefix + deserializes data
pub async fn read_frame<R: AsyncRead + Unpin, T: for<'de> Deserialize<'de>>(
    reader: &mut R,
) -> io::Result<T> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes).await?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    bincode::deserialize(&buf)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("bincode error: {e}")))
}
