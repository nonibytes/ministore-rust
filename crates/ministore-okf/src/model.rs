use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ValidateOptions {
    pub target_version: Option<String>,
}

pub type Projection = Map<String, Value>;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SyncOptions {
    pub target_version: Option<String>,
    pub strict: bool,
    pub dry_run: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SyncReport {
    pub ok: bool,
    pub bundle: String,
    pub projection_version: usize,
    pub concepts: usize,
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub deleted: usize,
    pub duration_ms: u128,
    pub validation: ValidationSummary,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ValidationSummary {
    pub target_version: String,
    pub declared_version: Option<String>,
    pub bundle: String,
    pub concepts: usize,
    pub errors: usize,
    pub warnings: usize,
}

impl ValidationSummary {
    pub fn ok(&self) -> bool {
        self.errors == 0
    }
}
