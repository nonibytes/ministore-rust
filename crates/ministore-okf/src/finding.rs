use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// Stable, versioned OKF diagnostic identifiers.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum FindingCode {
    OKF100,
    OKF101,
    OKF102,
    OKF103,
    OKF104,
    OKF105,
    OKF106,
    OKF107,
    OKF108,
    OKF200,
    OKF201,
    OKF202,
    OKF203,
    OKF204,
    OKF205,
    OKF206,
    OKF207,
    OKF300,
    OKF301,
    OKF302,
    OKF303,
    OKF304,
    OKF310,
    OKF311,
    OKF312,
    OKF313,
    OKF314,
    OKF315,
    OKF320,
    OKF321,
    OKF330,
    OKF340,
    OKF341,
    OKF350,
    OKF351,
    OKF352,
    OKF353,
    OKF354,
    OKF360,
    OKF400,
    OKF401,
    OKF402,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub code: FindingCode,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec_section: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct Parsed<T> {
    pub value: T,
    pub findings: Vec<Finding>,
}
