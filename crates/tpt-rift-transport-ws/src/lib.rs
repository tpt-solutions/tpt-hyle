#![forbid(unsafe_code)]
//! WebSocket transport for tpt-rift.
//!
//! One deltas-per-message framing: each binary message carries exactly one
//! encoded [`tpt_rift::Delta`]. A malformed message closes the connection.
//! Nothing from it reaches the store.
//!
//! **No authentication or TLS yet.** The auth scheme and access rules are
//! open decisions (see TODO.md, Phase 3). Use this for trusted links or
//! local tests only.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tpt_rift::{Delta, RiftError, Transport};

type Inbox = Arc<Mutex<VecDeque<Delta>>>;

/// One end of a WebSocket link. Built by [`connect`] (client) or [`accept`]
/// (server), or from any stream with [`WebSocketTransport::new`].
pub struct WebSocketTransport<S> {
    sink: tokio::sync::Mutex<SplitSink<WebSocketStream<S>, Message>>,
    inbox: Inbox,
    closed: Arc<AtomicBool>,
    reader: tokio::task::JoinHandle<()>,
}

impl<S> WebSocketTransport<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    /// Wraps an established WebSocket stream and starts reading from it.
    pub fn new(ws: WebSocketStream<S>) -> Self {
        let (sink, mut stream) = ws.split();
        let inbox: Inbox = Arc::default();
        let closed = Arc::new(AtomicBool::new(false));

        let q = inbox.clone();
        let flag = closed.clone();
        let reader = tokio::spawn(async move {
            while let Some(frame) = stream.next().await {
                match frame {
                    Ok(Message::Binary(bytes)) => match Delta::decode(&bytes) {
                        Ok(delta) => {
                            if let Ok(mut queue) = q.lock() {
                                queue.push_back(delta);
                            } else {
                                break;
                            }
                        }
                        // A message that breaks the wire rules ends the link.
                        Err(_) => break,
                    },
                    Ok(Message::Close(_)) | Err(_) => break,
                    // Ping, pong and text frames carry no deltas.
                    Ok(_) => {}
                }
            }
            flag.store(true, Ordering::SeqCst);
        });

        Self {
            sink: tokio::sync::Mutex::new(sink),
            inbox,
            closed,
            reader,
        }
    }
}

impl<S> Drop for WebSocketTransport<S> {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

fn transport_err(e: impl std::fmt::Display) -> RiftError {
    RiftError::Transport(e.to_string())
}

#[async_trait]
impl<S> Transport for WebSocketTransport<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    async fn disconnect(&self) -> Result<(), RiftError> {
        self.sink.lock().await.close().await.map_err(transport_err)
    }

    async fn send(&self, delta: Delta) -> Result<(), RiftError> {
        let bytes = delta.encode();
        self.sink
            .lock()
            .await
            .send(Message::Binary(bytes))
            .await
            .map_err(transport_err)
    }

    async fn receive(&self) -> Result<Option<Delta>, RiftError> {
        let next = self
            .inbox
            .lock()
            .map_err(|_| RiftError::Transport("inbox poisoned".into()))?
            .pop_front();
        match next {
            Some(delta) => Ok(Some(delta)),
            None if self.closed.load(Ordering::SeqCst) => {
                Err(RiftError::Transport("connection closed".into()))
            }
            None => Ok(None),
        }
    }
}

/// Opens a client connection to a `ws://` or `wss://` URL.
pub async fn connect(
    url: &str,
) -> Result<WebSocketTransport<MaybeTlsStream<TcpStream>>, RiftError> {
    let (ws, _response) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(transport_err)?;
    Ok(WebSocketTransport::new(ws))
}

/// Accepts the next TCP connection on `listener` and completes the WebSocket
/// handshake.
pub async fn accept(listener: &TcpListener) -> Result<WebSocketTransport<TcpStream>, RiftError> {
    let (stream, _addr) = listener.accept().await.map_err(transport_err)?;
    let ws = tokio_tungstenite::accept_async(stream)
        .await
        .map_err(transport_err)?;
    Ok(WebSocketTransport::new(ws))
}

/// Accepts a connection with a handshake callback and a message-size cap.
///
/// The callback sees the HTTP request, so it can check headers such as
/// `Authorization`. Returning an error rejects the upgrade, and the error
/// response is sent to the client.
pub async fn accept_with<S, F>(
    stream: S,
    max_message_bytes: usize,
    callback: F,
) -> Result<WebSocketTransport<S>, RiftError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    F: Unpin
        + FnOnce(
            &tokio_tungstenite::tungstenite::handshake::server::Request,
            tokio_tungstenite::tungstenite::handshake::server::Response,
        ) -> Result<
            tokio_tungstenite::tungstenite::handshake::server::Response,
            tokio_tungstenite::tungstenite::handshake::server::ErrorResponse,
        >,
{
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
        max_message_size: Some(max_message_bytes),
        ..Default::default()
    };
    let ws = tokio_tungstenite::accept_hdr_async_with_config(stream, callback, Some(config))
        .await
        .map_err(transport_err)?;
    Ok(WebSocketTransport::new(ws))
}

/// Opens a client connection that presents `token` as a bearer credential
/// in the `Authorization` header.
pub async fn connect_authorized(
    url: &str,
    token: &str,
) -> Result<WebSocketTransport<MaybeTlsStream<TcpStream>>, RiftError> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::HeaderValue;

    let mut request = url.into_client_request().map_err(transport_err)?;
    let value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(transport_err)?;
    request.headers_mut().insert("authorization", value);
    let (ws, _response) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(transport_err)?;
    Ok(WebSocketTransport::new(ws))
}
