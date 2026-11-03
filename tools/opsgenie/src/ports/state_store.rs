use anyhow::Result;

use crate::domain::MirrorState;

/// The mirror's memory between ticks and restarts. A missing store is an
/// empty state; an unreadable one is an error, never a fresh start that
/// would open every alert again.
pub trait StateStore: Send + Sync {
    fn load(&self) -> Result<MirrorState>;
    fn save(&self, state: &MirrorState) -> Result<()>;
}
