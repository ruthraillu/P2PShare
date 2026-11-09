//! Per-stream relay distribution trees. Implementation starts in Phase 8.
#[derive(Clone, Copy, Debug)]
pub struct RelayLimits {
    pub max_children: usize,
    pub max_depth: usize,
}
impl Default for RelayLimits {
    fn default() -> Self {
        Self {
            max_children: 2,
            max_depth: 3,
        }
    }
}
