use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// Stable, versioned OKF diagnostic identifiers.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

impl Severity {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

impl FindingCode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::OKF100 => "OKF100",
            Self::OKF101 => "OKF101",
            Self::OKF102 => "OKF102",
            Self::OKF103 => "OKF103",
            Self::OKF104 => "OKF104",
            Self::OKF105 => "OKF105",
            Self::OKF106 => "OKF106",
            Self::OKF107 => "OKF107",
            Self::OKF108 => "OKF108",
            Self::OKF200 => "OKF200",
            Self::OKF201 => "OKF201",
            Self::OKF202 => "OKF202",
            Self::OKF203 => "OKF203",
            Self::OKF204 => "OKF204",
            Self::OKF205 => "OKF205",
            Self::OKF206 => "OKF206",
            Self::OKF207 => "OKF207",
            Self::OKF300 => "OKF300",
            Self::OKF301 => "OKF301",
            Self::OKF302 => "OKF302",
            Self::OKF303 => "OKF303",
            Self::OKF304 => "OKF304",
            Self::OKF310 => "OKF310",
            Self::OKF311 => "OKF311",
            Self::OKF312 => "OKF312",
            Self::OKF313 => "OKF313",
            Self::OKF314 => "OKF314",
            Self::OKF315 => "OKF315",
            Self::OKF320 => "OKF320",
            Self::OKF321 => "OKF321",
            Self::OKF330 => "OKF330",
            Self::OKF340 => "OKF340",
            Self::OKF341 => "OKF341",
            Self::OKF350 => "OKF350",
            Self::OKF351 => "OKF351",
            Self::OKF352 => "OKF352",
            Self::OKF353 => "OKF353",
            Self::OKF354 => "OKF354",
            Self::OKF360 => "OKF360",
            Self::OKF400 => "OKF400",
            Self::OKF401 => "OKF401",
            Self::OKF402 => "OKF402",
        }
    }
}
