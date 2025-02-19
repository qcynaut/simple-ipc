use std::{
    collections::HashMap,
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
    net::{
        UnixListener,
        UnixStream,
    },
    sync::{
        broadcast,
        mpsc,
        RwLock,
    },
    task::JoinHandle,
};

use crate::{
    error::{
        IpcError,
        IpcResult,
    },
    EventHandler,
    Message,
};

/// Handles a single connection to the server.
///
/// This function takes a UnixStream and a shared map of event handlers and
/// processes incoming messages from the client.  It spawns two tasks: one to
/// handle incoming messages and another to send outgoing messages.  It
/// returns an `IpcResult` indicating success or failure of the
/// connection.
async fn handle_connection(
    stream: UnixStream,
    handlers: Arc<RwLock<HashMap<String, EventHandler>>>,
    shutdown_tx: broadcast::Sender<()>,
) -> IpcResult<()>
{
    let (mut reader, writer) = stream.into_split();
    let (tx, mut rx) = mpsc::channel(32);

    let mut shutdown_rx = shutdown_tx.subscribe();
    let write_task: JoinHandle<IpcResult<()>> = tokio::spawn(async move {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                Ok(())
            }
            res = async {
                let mut writer = writer;
                while let Some(response) = rx.recv().await {
                    let msg_bytes = bincode::serialize(&response)?;
                    let msg_len = msg_bytes.len();
                    writer.write_all(&msg_len.to_le_bytes()).await?;
                    writer.write_all(&msg_bytes).await?;
                    writer.flush().await?;
                }
                Ok(())
            } => res,
        }
    });

    let mut shutdown_rx = shutdown_tx.subscribe();
    let read_task: JoinHandle<IpcResult<()>> = tokio::spawn(async move {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                Ok(())
            }
            res = async {
                loop {
                    let mut msg_len_buf = [0u8; 8];
                    reader.read_exact(&mut msg_len_buf).await?;
                    let msg_len = u64::from_le_bytes(msg_len_buf);
                    let mut msg_buf = vec![0u8; msg_len as usize];
                    reader.read_exact(&mut msg_buf).await?;
                    let msg: Message = bincode::deserialize(&msg_buf)?;
                    let handlers = handlers.read().await;

                    if let Some(handler) = handlers.get(&msg.event) {
                        match handler(msg.payload) {
                            Ok(response) => {
                                let response = Message {
                                    event: msg.event,
                                    payload: response,
                                };
                                tx.send(response)
                                    .await
                                    .map_err(|_| IpcError::ChannelClosed)?
                            }
                            Err(e) => tracing::error!("Handler error: {}", e),
                        }
                    } else {
                        tracing::error!("Handler not found for event: {}", msg.event);
                    }
                }
            } => res,
        }
    });

    let (write_result, read_result) = tokio::try_join!(write_task, read_task)?;
    write_result?;
    read_result?;
    Ok(())
}

/// A server for handling IPC communication.
///
/// This struct listens for incoming connections on a Unix socket and dispatches
/// messages to the appropriate event handlers. It manages a set of event handlers
/// that can be dynamically registered and invoked by clients.
///
/// The server is cloneable, allowing it to be safely shared between tasks.
#[derive(Clone)]
pub struct Server
{
    handlers: Arc<RwLock<HashMap<String, EventHandler>>>,
    shutdown: broadcast::Sender<()>,
}

impl std::fmt::Debug for Server
{
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result
    {
        f.debug_struct("Server")
            .field("handlers", &self.handlers.blocking_read().keys())
            .field("_shutdown", &self.shutdown)
            .finish()
    }
}

impl Server
{
    /// Creates a new `Server` by binding to the specified Unix socket path.
    ///
    /// # Arguments
    ///
    /// * `path` - A type that can be converted into a `PathBuf`, representing the Unix socket path to bind to.
    ///
    /// # Returns
    ///
    /// Returns an `IpcResult` containing the new `Server` instance or an error if the binding fails.
    ///
    /// This function initializes the server by binding to the provided Unix socket path. It sets up
    /// a listener for incoming connections and spawns a background task to handle these connections.
    /// If a connection is accepted, it spawns a new task to handle the connection using the `handle_connection` function.
    ///
    /// It also initializes a set of event handlers and a shutdown channel. The server can be safely cloned
    /// and shared between tasks.
    pub async fn new(path: impl Into<PathBuf>) -> IpcResult<Self>
    {
        let path = path.into();
        if path.exists() {
            std::fs::remove_file(&path)?
        }

        let listener = UnixListener::bind(&path)?;
        let handlers = Arc::new(RwLock::new(HashMap::new()));
        let (shutdown_tx, _) = broadcast::channel(1);

        let handlers_clone = handlers.clone();
        let mut shutdown_rx = shutdown_tx.subscribe();
        let shutdown_tx_clone = shutdown_tx.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    accept_result = listener.accept() => {
                        match accept_result {
                            Ok((stream, _)) => {
                                let handlers = handlers_clone.clone();
                                let shutdown_tx = shutdown_tx_clone.clone();

                                tokio::spawn(async move {
                                    if let Err(e) = handle_connection(stream, handlers, shutdown_tx).await {
                                        tracing::error!("Error handling connection: {}", e);
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::error!("Accept error: {}", e);
                                break;
                            }
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        break;
                    }
                }
            }
        });

        Ok(Server {
            handlers,
            shutdown: shutdown_tx,
        })
    }

    /// Registers an event handler for the given event.
    ///
    /// The handler is called with the deserialized payload and the result of the handler is serialized and sent back to the client.
    ///
    /// The handler is shared between all clients, so it must be `Send` and `Sync`.
    ///
    /// # Type Parameters
    ///
    /// * `F`: The type of the handler function.
    /// * `T`: The type of the input to the handler.
    /// * `R`: The type of the output of the handler.
    pub async fn on<F, T, R>(
        &self,
        event: &str,
        handler: F,
    ) where
        F: Fn(T) -> IpcResult<R> + Send + Sync + 'static,
        T: DeserializeOwned + Send + 'static,
        R: Serialize + Send + 'static,
    {
        let handler = Arc::new(move |payload: Vec<u8>| -> IpcResult<Vec<u8>> {
            let input: T = bincode::deserialize(&payload)?;
            let result = handler(input)?;
            Ok(bincode::serialize(&result)?)
        });

        self.handlers
            .write()
            .await
            .insert(event.to_string(), handler);
    }

    /// Shuts down the server and closes all connections.
    pub fn shutdown(self)
    {
        if let Err(e) = self.shutdown.send(()) {
            tracing::error!("Error sending shutdown signal: {}", e);
        }
    }
}
