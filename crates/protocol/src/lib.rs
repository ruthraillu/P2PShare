//! Versioned, platform-independent room protocol. No transport library types.
use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub const PROTOCOL_VERSION: u16 = 1;

/// Public-key identity, never an IP address. Key generation belongs to the control backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PeerId(pub [u8; 32]);
macro_rules! random_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub Uuid);
        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}
random_id!(RoomId);
random_id!(StreamId);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoCodec {
    H264,
    VP9,
    AV1,
    H265,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioCodec {
    Opus,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quality {
    Thumbnail,
    Medium,
    High,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Capabilities {
    pub video_encode: Vec<VideoCodec>,
    pub video_decode: Vec<VideoCodec>,
    pub audio: Vec<AudioCodec>,
    pub relay_allowed: bool,
}
impl Default for Capabilities {
    fn default() -> Self {
        // Advertise no codecs until a real backend has passed runtime probing.
        Self {
            video_encode: vec![],
            video_decode: vec![],
            audio: vec![],
            relay_allowed: true,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope {
    pub version: u16,
    pub sequence: u64,
    pub message: ControlMessage,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[non_exhaustive]
pub enum ControlMessage {
    Hello {
        peer: PeerId,
        name: String,
        capabilities: Capabilities,
    },
    PeerJoined {
        peer: PeerId,
        name: String,
    },
    PeerLeft {
        peer: PeerId,
    },
    PublishStream {
        stream: StreamId,
        codec: VideoCodec,
        width: u32,
        height: u32,
    },
    UnpublishStream {
        stream: StreamId,
    },
    Subscribe {
        stream: StreamId,
        quality: Quality,
    },
    Unsubscribe {
        stream: StreamId,
    },
    RtcOffer {
        peer: PeerId,
        sdp: String,
    },
    RtcAnswer {
        peer: PeerId,
        sdp: String,
    },
    RequestKeyframe {
        stream: StreamId,
    },
    Ping {
        nonce: u64,
    },
    Pong {
        nonce: u64,
    },
    #[serde(other)]
    Unknown,
}
