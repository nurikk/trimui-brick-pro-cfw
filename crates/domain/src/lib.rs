use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    #[serde(rename = "catalogVersion")]
    pub catalog_version: String,
    pub entries: Vec<CatalogEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub id: String,
    pub title: String,
    pub system: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Route {
    Library,
    Systems,
    Games,
    Catalog,
    Session,
}

impl Route {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Systems => "systems",
            Self::Games => "games",
            Self::Catalog => "catalog",
            Self::Session => "session",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct LaunchRequest {
    pub selection: CatalogEntry,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    Started,
    Completed,
    Failed,
    Aborted,
}
