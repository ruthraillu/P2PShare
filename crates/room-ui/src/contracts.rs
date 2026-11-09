//! Input DTOs. Platform adapters translate the room engine snapshot into these values.
#[derive(Clone, Debug)]
pub struct MemberState {
    pub id: String,
    pub name: String,
    pub sharing: bool,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug)]
pub struct RoomState {
    pub local_id: String,
    pub members: Vec<MemberState>,
    pub in_room: bool,
    pub busy: bool,
    pub invite: Option<String>,
    pub saved_invite: Option<String>,
    pub status: String,
}
