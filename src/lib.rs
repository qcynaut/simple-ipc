//! Simple Ipc
//!
//! Simple Ipc is a library for sending and receiving messages over a Unix socket.

use std::sync::Arc;

use error::IpcResult;
use serde::{
    Deserialize,
    Serialize,
};

pub mod client;
pub mod error;
pub mod server;

/// A message sent between the client and server.
///
/// This struct represents a single message sent between the client and server.  It
/// contains the name of the event and the payload of the event.
///
/// The `event` field is the name of the event that the message is being sent to.
/// The `payload` field is the payload of the event.  It is a vector of bytes that
/// is serialized and deserialized using bincode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Message
{
    event: String,
    payload: Vec<u8>,
}

/// A handler for an event on the server.
///
/// This type represents a single handler for an event on the server.  It is
/// a closure that takes a single argument, the payload of the event, and
/// returns a result containing the response to the event.  The handler
/// closure is wrapped in an `Arc` so that it can be safely shared between
/// threads.
///
/// The handler closure must implement `Send` and `Sync` so that it can be
/// safely moved between threads and accessed concurrently by multiple
/// threads.
pub(crate) type EventHandler = Arc<dyn Fn(Vec<u8>) -> IpcResult<Vec<u8>> + Send + Sync>;

#[cfg(test)]
mod tests
{
    use super::*;
    use crate::{
        client::Client,
        server::Server,
    };

    #[tokio::test]
    async fn test_ipc()
    {
        let server = Server::new("/tmp/test.sock").await.unwrap();

        server
            .on("status", |msg: String| -> IpcResult<String> {
                Ok(format!("Received: {}", msg))
            })
            .await;

        #[derive(Serialize, Deserialize)]
        struct Ping
        {
            message: String,
        }

        #[derive(Serialize, Deserialize)]
        struct Pong
        {
            response: String,
        }

        server
            .on("ping", |msg: Ping| -> IpcResult<Pong> {
                Ok(Pong {
                    response: format!("Received: {}", msg.message),
                })
            })
            .await;

        let client = Client::new("/tmp/test.sock").await.unwrap();
        let response: String = client.call("status", "Hello").await.unwrap();
        assert_eq!(response, "Received: Hello");

        let response: Pong = client
            .call(
                "ping",
                Ping {
                    message: "Hello".to_owned(),
                },
            )
            .await
            .unwrap();
        assert_eq!(response.response, "Received: Hello");

        server.shutdown();

        let res = client.call::<_, String>("ping", "Hello").await;
        println!("{:?}", res);
        assert!(res.is_err());
    }
}
