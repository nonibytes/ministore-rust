use serde::Serialize;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ValidateOptions {
    pub target_version: Option<String>,
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
