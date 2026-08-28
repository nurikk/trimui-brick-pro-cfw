use crate::SyncError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadResponse {
    pub status: u16,
    pub etag: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PutCondition {
    IfMatch(String),
    IfNoneMatchStar,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PutResponse {
    Created,
    Replaced,
    PreconditionFailed,
    Other(u16),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WebDavOutcome {
    Ready(PutCondition),
    Stored,
    Conflict(String),
}

pub struct WebDavAdapter;

impl WebDavAdapter {
    pub fn replacement(head: &HeadResponse) -> WebDavOutcome {
        match strong_etag(head) {
            Some(etag) => WebDavOutcome::Ready(PutCondition::IfMatch(etag)),
            None => WebDavOutcome::Conflict("strong validator is missing or weak".into()),
        }
    }

    pub fn create_only(head: &HeadResponse) -> WebDavOutcome {
        if head.status == 404 && head.etag.is_none() {
            WebDavOutcome::Ready(PutCondition::IfNoneMatchStar)
        } else {
            WebDavOutcome::Conflict("create-only target already exists or is untrustworthy".into())
        }
    }

    pub fn finish(condition: &PutCondition, response: PutResponse) -> WebDavOutcome {
        if matches!(condition, PutCondition::IfMatch(etag) if !is_strong_etag(etag)) {
            return WebDavOutcome::Conflict("replacement requires a strong ETag".into());
        }
        match response {
            PutResponse::Created | PutResponse::Replaced => WebDavOutcome::Stored,
            PutResponse::PreconditionFailed => {
                let _ = condition;
                WebDavOutcome::Conflict("WebDAV precondition failed; no unconditional retry".into())
            }
            PutResponse::Other(status) => {
                WebDavOutcome::Conflict(format!("WebDAV status {status}"))
            }
        }
    }

    pub fn require_strong_validator(etag: Option<&str>) -> Result<String, SyncError> {
        etag.filter(|value| is_strong_etag(value))
            .map(str::to_owned)
            .ok_or_else(|| SyncError::message("strong WebDAV ETag is required"))
    }
}

fn is_strong_etag(etag: &str) -> bool {
    etag.len() <= 255
        && etag.starts_with('"')
        && etag.ends_with('"')
        && !etag.starts_with("W/")
        && !etag.contains(['\n', '\r'])
}

fn strong_etag(head: &HeadResponse) -> Option<String> {
    (head.status == 200)
        .then_some(head.etag.as_deref())
        .flatten()
        .filter(|etag| is_strong_etag(etag))
        .map(str::to_owned)
}
