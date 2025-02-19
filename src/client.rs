use std::{
    path::PathBuf,
    sync::Arc,
};

use serde::{
    de::DeserializeOwned,
    Serialize,
};
use tokio::{
    io::{
        AsyncReadExt,
        AsyncWriteExt,
    },
    net::UnixStream,
    sync::RwLock,
};

use crate::{
    error::IpcResult,
    Message,
};

/// A client for IPC communication with a server.
///
/// This struct provides a convenient interface for calling event handlers on a server over a Unix socket.
/// It allows you to send arbitrary payloads to event handlers and receive the response.
///
/// The client is cloneable and can be safely shared between threads.
#[derive(Debug, Clone)]
pub struct Client
{
    stream: Arc<RwLock<UnixStream>>,
}

impl Client
{
    /// Creates a new `Client` by establishing a connection to the specified Unix socket path.
    ///
    /// # Arguments
    ///
    /// * `path` - A type that can be converted into a `PathBuf`, representing the Unix socket path to connect to.
    ///
    /// # Returns
    ///
    /// Returns an `IpcResult` containing the new `Client` instance or an error if the connection fails.
    pub async fn new(path: impl Into<PathBuf>) -> IpcResult<Self>
    {
        let stream = UnixStream::connect(path.into()).await?;
        Ok(Client {
            stream: Arc::new(RwLock::new(stream)),
        })
    }

    /// Calls an event handler on the server with the specified payload.
    ///
    /// # Arguments
    ///
    /// * `event` - The name of the event to call.
    /// * `payload` - The payload to pass to the event handler.
    ///
    /// # Returns
    ///
    /// Returns an `IpcResult` containing the result of the event handler or an error if the call fails.
    pub async fn call<T, R>(
        &self,
        event: &str,
        payload: T,
    ) -> IpcResult<R>
    where
        T: Serialize,
        R: DeserializeOwned,
    {
        let mut stream = self.stream.write().await;

        let msg = Message {
            event: event.to_string(),
            payload: bincode::serialize(&payload)?,
        };

        let msg_bytes = bincode::serialize(&msg)?;
        let msg_len = msg_bytes.len();
        stream.write_all(&msg_len.to_le_bytes()).await?;
        stream.write_all(&msg_bytes).await?;

        let mut msg_len_buf = [0u8; 8];
        stream.read_exact(&mut msg_len_buf).await?;
        let msg_len = u64::from_le_bytes(msg_len_buf);
        let mut msg_bytes = vec![0u8; msg_len as usize];
        stream.read_exact(&mut msg_bytes).await?;

        let msg: Message = bincode::deserialize(&msg_bytes)?;
        Ok(bincode::deserialize(&msg.payload)?)
    }

    /// Closes the connection to the server.
    pub async fn close(self)
    {
        let mut stream = self.stream.write().await;
        if let Err(e) = stream.shutdown().await {
            tracing::error!("Error closing connection: {}", e);
        }
    }
}
