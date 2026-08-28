use std::path::Path;

use crate::{Candidate, Exchange, StagedCandidate, SyncError};

pub struct SyncthingAdapter {
    exchange: Exchange,
}

impl SyncthingAdapter {
    pub fn new(exchange: Exchange) -> Self {
        Self { exchange }
    }

    pub fn exchange(&self) -> &Exchange {
        &self.exchange
    }

    pub fn ingest(
        &self,
        file_name: &str,
        candidate: Candidate,
        payload: &[u8],
    ) -> Result<StagedCandidate, SyncError> {
        if file_name.is_empty() || file_name.contains('/') || file_name.contains('\\') {
            return Err(SyncError::message("Syncthing file name is invalid"));
        }
        self.exchange
            .stage_remote(candidate, payload, is_conflict_copy(file_name))
    }

    pub fn reject_live_folder(&self, path: &Path) -> Result<(), SyncError> {
        if path == self.exchange.root().join("live") || path.join("live").exists() {
            return Err(SyncError::message(
                "Syncthing adapter only accepts its dedicated exchange",
            ));
        }
        Ok(())
    }
}

pub fn is_conflict_copy(file_name: &str) -> bool {
    file_name
        .split_once(".sync-conflict-")
        .is_some_and(|(prefix, suffix)| !prefix.is_empty() && !suffix.is_empty())
}
