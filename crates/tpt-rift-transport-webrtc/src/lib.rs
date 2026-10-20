#![forbid(unsafe_code)]
//! WebRTC peer-to-peer transport for tpt-rift.
//!
//! One data channel carries one encoded delta per message. Connecting takes
//! two session descriptions, an offer and an answer, and the library does not
//! send them. The caller carries them over any signaling path, such as the
//! sync server's WebSocket, or a QR code between two phones:
//!
//! 1. The offering side calls [`create_offer`], and sends `offer.sdp()`.
//! 2. The answering side calls [`accept`] with that string, and sends back the
//!    answer it gets.
//! 3. The offering side calls [`Offer::complete`] with the answer.
//!
//! Both sides then call [`WebRtcTransport::wait_open`]. The session uses host
//! candidates only, with no STUN or TURN servers. That suits peers on one LAN.
//! Peers behind NAT need ICE servers, which this module does not configure.
//!
//! Limit: a delta must fit in one data-channel message. Keep the number of
//! events per round small, because the sync engine sends one delta per round.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::Notify;
use tpt_rift::{Delta, RiftError, Transport};
use webrtc::api::APIBuilder;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_credential_type::RTCIceCredentialType;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;

const CHANNEL_LABEL: &str = "tpt-rift";

fn transport_err(e: impl std::fmt::Display) -> RiftError {
    RiftError::Transport(e.to_string())
}

/// State shared by the callbacks and the transport.
#[derive(Default)]
struct Shared {
    inbox: Mutex<VecDeque<Delta>>,
    channel: Mutex<Option<Arc<RTCDataChannel>>>,
    opened: AtomicBool,
    closed: AtomicBool,
    opened_notify: Notify,
}

impl Shared {
    fn inbox(&self) -> Result<std::sync::MutexGuard<'_, VecDeque<Delta>>, RiftError> {
        self.inbox
            .lock()
            .map_err(|_| RiftError::Transport("inbox poisoned".into()))
    }
}

/// Wires the channel's message, open, and close callbacks to `shared`, and
/// stores the channel so `send` can reach it.
async fn attach(dc: &Arc<RTCDataChannel>, shared: &Arc<Shared>) {
    let inbox_shared = shared.clone();
    dc.on_message(Box::new(move |msg: DataChannelMessage| {
        let shared = inbox_shared.clone();
        Box::pin(async move {
            // A message that does not decode is dropped. It never reaches the store.
            if let Ok(delta) = Delta::decode(&msg.data) {
                if let Ok(mut queue) = shared.inbox.lock() {
                    queue.push_back(delta);
                }
            }
        })
    }));

    let open_shared = shared.clone();
    dc.on_open(Box::new(move || {
        let shared = open_shared.clone();
        Box::pin(async move {
            shared.opened.store(true, Ordering::SeqCst);
            shared.opened_notify.notify_waiters();
        })
    }));

    let close_shared = shared.clone();
    dc.on_close(Box::new(move || {
        let shared = close_shared.clone();
        Box::pin(async move {
            shared.closed.store(true, Ordering::SeqCst);
        })
    }));

    if let Ok(mut slot) = shared.channel.lock() {
        *slot = Some(dc.clone());
    }
    // The channel may already be open when we attach. Then on_open will not
    // fire, so check the state now.
    if dc.ready_state() == webrtc::data_channel::data_channel_state::RTCDataChannelState::Open {
        shared.opened.store(true, Ordering::SeqCst);
        shared.opened_notify.notify_waiters();
    }
}

/// ICE servers for NAT traversal. The default is empty: host candidates only.
#[derive(Debug, Clone, Default)]
pub struct IceConfig {
    /// STUN server URLs, such as `stun:stun.example.org:3478`. These let a peer
    /// learn its public address.
    pub stun_urls: Vec<String>,
    /// TURN relays, for peers behind symmetric NAT.
    pub turn: Vec<TurnServer>,
}

/// A TURN relay with its long-term credentials.
#[derive(Debug, Clone)]
pub struct TurnServer {
    /// A `turn:` or `turns:` URL.
    pub url: String,
    /// The username for the relay.
    pub username: String,
    /// The password for the relay.
    pub credential: String,
}

impl IceConfig {
    fn to_rtc(&self) -> RTCConfiguration {
        let mut servers = Vec::new();
        if !self.stun_urls.is_empty() {
            servers.push(RTCIceServer {
                urls: self.stun_urls.clone(),
                ..Default::default()
            });
        }
        for turn in &self.turn {
            servers.push(RTCIceServer {
                urls: vec![turn.url.clone()],
                username: turn.username.clone(),
                credential: turn.credential.clone(),
                credential_type: RTCIceCredentialType::Password,
            });
        }
        RTCConfiguration {
            ice_servers: servers,
            ..Default::default()
        }
    }
}

async fn new_connection(ice: &IceConfig) -> Result<Arc<RTCPeerConnection>, RiftError> {
    let api = APIBuilder::new().build();
    let pc = api
        .new_peer_connection(ice.to_rtc())
        .await
        .map_err(transport_err)?;
    Ok(Arc::new(pc))
}

/// The offering side, between creating the offer and receiving the answer.
pub struct Offer {
    pc: Arc<RTCPeerConnection>,
    shared: Arc<Shared>,
    sdp: String,
}

impl Offer {
    /// The session description to send to the answering side.
    pub fn sdp(&self) -> &str {
        &self.sdp
    }

    /// Applies the answer and returns the transport. Call
    /// [`WebRtcTransport::wait_open`] before sending.
    pub async fn complete(self, answer_sdp: String) -> Result<WebRtcTransport, RiftError> {
        let answer = RTCSessionDescription::answer(answer_sdp).map_err(transport_err)?;
        self.pc
            .set_remote_description(answer)
            .await
            .map_err(transport_err)?;
        Ok(WebRtcTransport {
            pc: self.pc,
            shared: self.shared,
        })
    }
}

/// Creates the offering side of a connection.
pub async fn create_offer() -> Result<Offer, RiftError> {
    create_offer_with(&IceConfig::default()).await
}

/// Creates the offering side, using the given ICE servers.
pub async fn create_offer_with(ice: &IceConfig) -> Result<Offer, RiftError> {
    let pc = new_connection(ice).await?;
    let shared = Arc::new(Shared::default());
    let dc = pc
        .create_data_channel(CHANNEL_LABEL, None)
        .await
        .map_err(transport_err)?;
    attach(&dc, &shared).await;

    let offer = pc.create_offer(None).await.map_err(transport_err)?;
    // Create the gathering promise before setting the description, so that a
    // fast gathering cannot finish unobserved.
    let mut gathered = pc.gathering_complete_promise().await;
    pc.set_local_description(offer)
        .await
        .map_err(transport_err)?;
    let _ = gathered.recv().await;
    let local = pc
        .local_description()
        .await
        .ok_or_else(|| RiftError::Transport("no local description".into()))?;
    Ok(Offer {
        pc,
        shared,
        sdp: local.sdp,
    })
}

/// Accepts an offer and returns the transport and the answer to send back.
pub async fn accept(offer_sdp: String) -> Result<(WebRtcTransport, String), RiftError> {
    accept_with(&IceConfig::default(), offer_sdp).await
}

/// Accepts an offer, using the given ICE servers.
pub async fn accept_with(
    ice: &IceConfig,
    offer_sdp: String,
) -> Result<(WebRtcTransport, String), RiftError> {
    let pc = new_connection(ice).await?;
    let shared = Arc::new(Shared::default());

    let handler_shared = shared.clone();
    pc.on_data_channel(Box::new(move |dc: Arc<RTCDataChannel>| {
        let shared = handler_shared.clone();
        Box::pin(async move {
            attach(&dc, &shared).await;
        })
    }));

    let offer = RTCSessionDescription::offer(offer_sdp).map_err(transport_err)?;
    pc.set_remote_description(offer)
        .await
        .map_err(transport_err)?;
    let answer = pc.create_answer(None).await.map_err(transport_err)?;
    let mut gathered = pc.gathering_complete_promise().await;
    pc.set_local_description(answer)
        .await
        .map_err(transport_err)?;
    let _ = gathered.recv().await;
    let local = pc
        .local_description()
        .await
        .ok_or_else(|| RiftError::Transport("no local description".into()))?;

    Ok((WebRtcTransport { pc, shared }, local.sdp))
}

/// An open or opening data channel to one peer.
pub struct WebRtcTransport {
    pc: Arc<RTCPeerConnection>,
    shared: Arc<Shared>,
}

impl WebRtcTransport {
    /// Waits until the channel is open, or fails after `timeout`.
    pub async fn wait_open(&self, timeout: Duration) -> Result<(), RiftError> {
        tokio::time::timeout(timeout, async {
            loop {
                let notified = self.shared.opened_notify.notified();
                if self.shared.opened.load(Ordering::SeqCst) {
                    return Ok(());
                }
                if self.shared.closed.load(Ordering::SeqCst) {
                    return Err(RiftError::Transport("channel closed before opening".into()));
                }
                notified.await;
            }
        })
        .await
        .map_err(|_| RiftError::Transport("channel did not open in time".into()))?
    }

    fn channel(&self) -> Result<Arc<RTCDataChannel>, RiftError> {
        self.shared
            .channel
            .lock()
            .map_err(|_| RiftError::Transport("channel slot poisoned".into()))?
            .clone()
            .ok_or_else(|| RiftError::Transport("no data channel yet".into()))
    }
}

#[async_trait]
impl Transport for WebRtcTransport {
    async fn disconnect(&self) -> Result<(), RiftError> {
        self.pc.close().await.map_err(transport_err)
    }

    async fn send(&self, delta: Delta) -> Result<(), RiftError> {
        if !self.shared.opened.load(Ordering::SeqCst) {
            return Err(RiftError::Transport("channel is not open".into()));
        }
        let channel = self.channel()?;
        let bytes = Bytes::from(delta.encode());
        channel.send(&bytes).await.map_err(transport_err)?;
        Ok(())
    }

    async fn receive(&self) -> Result<Option<Delta>, RiftError> {
        if let Some(delta) = self.shared.inbox()?.pop_front() {
            return Ok(Some(delta));
        }
        if self.shared.closed.load(Ordering::SeqCst) {
            return Err(RiftError::Transport("channel closed".into()));
        }
        Ok(None)
    }
}
