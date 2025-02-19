use thiserror::Error;
use tokio::task::JoinError;

/// An error type representing all the possible errors that can occur in the IPC library.
///
/// This type is used as the error type for the `IpcResult` type alias, which is used to
/// represent the result of operations in the IPC library.
///
/// The `IpcError` type has the following variants:
///
/// * `Io`: an IO error that occurred when reading from or writing to the Unix socket.
/// * `Serialization`: a serialization error that occurred when serializing or deserializing
///   a message.
/// * `HandlerNotFound`: an error that occurred when the client called an event handler that
///   was not registered on the server.
/// * `ChannelClosed`: an error that occurred when the client tried to send a message on a
///   channel that had already been closed.
/// * `JoinError`: an error that occurred when waiting for a task to complete.
#[derive(Error, Debug)]
pub enum IpcError
{
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] bincode::Error),
    #[error("Event handler not found: {0}")]
    HandlerNotFound(String),
    #[error("Channel closed")]
    ChannelClosed,
    #[error("Task join error: {0}")]
    JoinError(#[from] JoinError),
    #[error("Other: {0}")]
    Other(String),
}

/// A specialized `Result` type for IPC operations.
///
/// This type is used throughout the IPC library to represent the outcome of an operation
/// that may produce an `IpcError`.
///
/// # Type Parameters
///
/// * `T`: The type of the successful result.
pub type IpcResult<T> = std::result::Result<T, IpcError>;
