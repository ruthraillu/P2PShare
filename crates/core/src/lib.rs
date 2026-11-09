//! Application state independent of OS, codecs, and transport implementations.
use p2pshare_media::{CaptureSource, CaptureState, FrameMetadata};
use p2pshare_protocol::{PeerId, RoomId, StreamId};
#[derive(Clone, Debug)]
pub enum Command {
    RefreshSources,
    StartCapture(CaptureSource),
    StopCapture,
    CreateRoom { name: String },
    JoinRoom { invite: String },
}
#[derive(Clone, Debug)]
pub enum Event {
    Sources(Vec<CaptureSource>),
    CaptureChanged(CaptureState),
    Frame(FrameMetadata),
    RoomCreated(RoomId),
    PeerJoined(PeerId),
    StreamPublished(StreamId),
    Error(String),
}
#[derive(Debug)]
pub struct AppState {
    pub sources: Vec<CaptureSource>,
    pub capture: CaptureState,
    pub last_frame: Option<FrameMetadata>,
}
impl Default for AppState {
    fn default() -> Self {
        Self {
            sources: vec![],
            capture: CaptureState::Stopped,
            last_frame: None,
        }
    }
}
impl AppState {
    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Sources(sources) => self.sources = sources,
            Event::CaptureChanged(state) => {
                self.capture = state;
                if state == CaptureState::Stopped {
                    self.last_frame = None;
                }
            }
            Event::Frame(frame) => self.last_frame = Some(frame),
            _ => (),
        }
    }
}
