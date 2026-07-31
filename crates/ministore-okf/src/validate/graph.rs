use std::path::Path;

use pulldown_cmark::{Event, Parser, Tag};
use rusqlite::{params, OptionalExtension};

use crate::staging::ValidationStage;
use crate::{Document, Finding, FindingCode, Result, Severity};

pub(crate) struct LinkCandidate {
    pub destination: String,
    pub line: usize,
    pub column: usize,
}

pub(crate) fn extract_link_candidates(document: &Document) -> Vec<LinkCandidate> {
    let Some(body) = document.body() else {
        return Vec::new();
    };
    let Ok(text) = std::str::from_utf8(body) else {
        return Vec::new();
    };
    let base = document.body_offset().unwrap_or(0);
    Parser::new(text)
        .into_offset_iter()
        .filter_map(|(event, range)| {
            let Event::Start(Tag::Link { dest_url, .. }) = event else {
                return None;
            };
            let (line, column) = byte_position(document.raw(), base + range.start);
            Some(LinkCandidate {
                destination: dest_url.into_string(),
                line,
                column,
            })
        })
        .collect()
}

pub(crate) fn resolve_staged_links(stage: &mut ValidationStage) -> Result<()> {
    let mut after = 0_i64;
    loop {
        let candidate: Option<(i64,String,String,usize,usize)> = stage.connection.query_row("SELECT id,source,destination,line,column FROM link_candidates WHERE id>?1 ORDER BY id LIMIT 1",[after],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let Some((id, source, destination, line, column)) = candidate else {
            break;
        };
        after = id;
        match normalize_link(&source, &destination) {
            LinkResult::External => {}
            LinkResult::Invalid(code) => {
                insert_finding(stage, &source, &destination, line, column, code)?
            }
            LinkResult::Local(target) => {
                let kind: Option<String> = stage
                    .connection
                    .query_row("SELECT kind FROM entries WHERE path=?1", [&target], |r| {
                        r.get(0)
                    })
                    .optional()?;
                match kind.as_deref() {
                    Some("concept") => {
                        stage.connection.execute(
                            "INSERT OR IGNORE INTO edges(source,target) VALUES (?1,?2)",
                            params![source, target],
                        )?;
                    }
                    None => insert_finding(
                        stage,
                        &source,
                        &destination,
                        line,
                        column,
                        FindingCode::OKF400,
                    )?,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_attested_resources(
    stage: &ValidationStage,
    document: &Document,
) -> Result<Vec<Finding>> {
    let Some(metadata) = document.metadata() else {
        return Ok(Vec::new());
    };
    if metadata.string("type") != Some("Attested Computation") {
        return Ok(Vec::new());
    }
    let mut refs: Vec<(&str, FindingCode, crate::Position)> = Vec::new();
    if let Some(node) = metadata
        .get("computation")
        .and_then(|n| metadata.resolve(n))
    {
        if let Some(value) = node.as_string().filter(|v| !v.is_empty()) {
            refs.push((value, FindingCode::OKF352, node.position))
        }
    }
    for (key, code) in [
        ("executor", FindingCode::OKF353),
        ("attester", FindingCode::OKF354),
    ] {
        if let Some(container) = metadata.get(key).and_then(|n| metadata.resolve(n)) {
            if let Some(node) = metadata.mapping_get(container, "resource") {
                if let Some(value) = node.as_string().filter(|v| !v.is_empty()) {
                    refs.push((value, code, node.position))
                }
            }
        }
    }
    let mut findings = Vec::new();
    for (value, code, position) in refs {
        let missing = match normalize_link(document.path(), value) {
            LinkResult::External => false,
            LinkResult::Invalid(_) => true,
            LinkResult::Local(target) => stage
                .connection
                .query_row("SELECT 1 FROM entries WHERE path=?1", [target], |r| {
                    r.get::<_, i64>(0)
                })
                .optional()?
                .is_none(),
        };
        if missing {
            findings.push(Finding {
                severity: Severity::Warning,
                code,
                path: document.path().to_owned(),
                line: Some(position.line),
                column: Some(position.column),
                spec_section: Some("10.3".into()),
                message: "local computation resource does not exist".into(),
            })
        }
    }
    Ok(findings)
}

enum LinkResult {
    External,
    Local(String),
    Invalid(FindingCode),
}
fn normalize_link(source: &str, destination: &str) -> LinkResult {
    let without_fragment = destination.split('#').next().unwrap_or("");
    if without_fragment.is_empty() {
        return LinkResult::External;
    }
    let raw = without_fragment.split('?').next().unwrap_or("");
    if raw.starts_with("//") || has_scheme(raw) {
        return LinkResult::External;
    }
    let mut decoded_segments = Vec::new();
    for segment in raw.split('/') {
        match percent_decode(segment) {
            Ok(s) => decoded_segments.push(s),
            Err(()) => return LinkResult::Invalid(FindingCode::OKF402),
        }
    }
    let decoded = decoded_segments.join("/");
    let combined = if decoded.starts_with('/') {
        decoded.trim_start_matches('/').to_owned()
    } else {
        let parent = Path::new(source)
            .parent()
            .and_then(Path::to_str)
            .unwrap_or("");
        if parent.is_empty() {
            decoded
        } else {
            format!("{parent}/{decoded}")
        }
    };
    let mut stack = Vec::new();
    for segment in combined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if stack.pop().is_none() {
                    return LinkResult::Invalid(FindingCode::OKF401);
                }
            }
            s => stack.push(s),
        }
    }
    LinkResult::Local(stack.join("/"))
}
fn has_scheme(value: &str) -> bool {
    let Some((head, _)) = value.split_once(':') else {
        return false;
    };
    !head.is_empty()
        && head.chars().enumerate().all(|(i, c)| {
            if i == 0 {
                c.is_ascii_alphabetic()
            } else {
                c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')
            }
        })
}
fn percent_decode(segment: &str) -> std::result::Result<String, ()> {
    let bytes = segment.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(());
            };
            let h = hex(bytes[i + 1])?;
            let l = hex(bytes[i + 2])?;
            let value = h * 16 + l;
            if matches!(value, b'/' | b'\\' | 0) {
                return Err(());
            };
            out.push(value);
            i += 3
        } else {
            out.push(bytes[i]);
            i += 1
        }
    }
    String::from_utf8(out).map_err(|_| ())
}
fn hex(value: u8) -> std::result::Result<u8, ()> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(()),
    }
}
fn byte_position(raw: &[u8], offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    for c in String::from_utf8_lossy(&raw[..offset]).chars() {
        if c == '\n' {
            line += 1;
            column = 1
        } else {
            column += 1
        }
    }
    (line, column)
}
fn insert_finding(
    stage: &mut ValidationStage,
    source: &str,
    destination: &str,
    line: usize,
    column: usize,
    code: FindingCode,
) -> Result<()> {
    let message = match code {
        FindingCode::OKF401 => "local link escapes the bundle root",
        FindingCode::OKF402 => "local link contains unsafe percent encoding",
        _ => "local link target does not exist",
    };
    let tx = stage.transaction()?;
    ValidationStage::insert_finding(
        &tx,
        &Finding {
            severity: Severity::Warning,
            code,
            path: source.to_owned(),
            line: Some(line),
            column: Some(column),
            spec_section: Some("4.1".into()),
            message: format!("{message}: {destination}"),
        },
    )?;
    tx.commit()?;
    Ok(())
}
