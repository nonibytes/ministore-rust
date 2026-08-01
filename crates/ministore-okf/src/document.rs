use crate::finding::{Finding, FindingCode, Parsed, Severity};
use crate::yaml::{parse_metadata, Metadata};
use crate::Result;

const UTF8_BOM: &[u8] = &[0xef, 0xbb, 0xbf];

#[derive(Clone, Copy, Debug)]
struct ByteRange {
    start: usize,
    end: usize,
}

/// One complete concept document with parser-owned source bytes and exact byte
/// ranges for frontmatter and body.
#[derive(Clone, Debug)]
pub struct Document {
    path: String,
    raw: Vec<u8>,
    frontmatter: Option<ByteRange>,
    body: Option<ByteRange>,
    metadata: Option<Metadata>,
}

impl Document {
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Return the exact source bytes, including BOM and original line endings.
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// Return the exact bytes between the delimiter lines.
    pub fn frontmatter_yaml(&self) -> Option<&[u8]> {
        self.frontmatter
            .map(|range| &self.raw[range.start..range.end])
    }

    /// Return the exact bytes after the closing delimiter and its line ending.
    pub fn body(&self) -> Option<&[u8]> {
        self.body.map(|range| &self.raw[range.start..range.end])
    }

    pub(crate) fn body_offset(&self) -> Option<usize> {
        self.body.map(|range| range.start)
    }

    pub fn metadata(&self) -> Option<&Metadata> {
        self.metadata.as_ref()
    }
}

/// Split and parse one OKF concept without normalizing source bytes. Format
/// defects are findings; `Err` is reserved for operational failures.
pub fn parse_document(path: &str, raw: &[u8]) -> Result<Parsed<Document>> {
    let mut document = Document {
        path: path.to_owned(),
        raw: raw.to_vec(),
        frontmatter: None,
        body: None,
        metadata: None,
    };

    let text = match std::str::from_utf8(&document.raw) {
        Ok(text) => text,
        Err(error) => {
            let (line, column) = byte_position(&document.raw, error.valid_up_to());
            return Ok(Parsed {
                value: document,
                findings: vec![parse_finding(
                    Severity::Error,
                    FindingCode::OKF100,
                    path,
                    line,
                    column,
                    "concept is not valid UTF-8",
                )],
            });
        }
    };

    let mut findings = Vec::new();
    let content_start = if document.raw.starts_with(UTF8_BOM) {
        findings.push(parse_finding(
            Severity::Warning,
            FindingCode::OKF107,
            path,
            1,
            1,
            "document begins with a UTF-8 BOM",
        ));
        UTF8_BOM.len()
    } else {
        0
    };

    let (opening_content_end, opening_line_end) = physical_line(&document.raw, content_start);
    let opening = &text[content_start..opening_content_end];
    if opening.trim() != "---" {
        findings.push(parse_finding(
            Severity::Error,
            FindingCode::OKF101,
            path,
            1,
            1,
            "concept has no opening frontmatter delimiter",
        ));
        return Ok(Parsed {
            value: document,
            findings,
        });
    }
    if opening != "---" {
        findings.push(parse_finding(
            Severity::Warning,
            FindingCode::OKF108,
            path,
            1,
            1,
            "opening frontmatter delimiter contains surrounding whitespace",
        ));
    }

    let frontmatter_start = opening_line_end;
    let mut line_start = opening_line_end;
    while line_start < document.raw.len() {
        let (content_end, line_end) = physical_line(&document.raw, line_start);
        if &document.raw[line_start..content_end] == b"---" {
            document.frontmatter = Some(ByteRange {
                start: frontmatter_start,
                end: line_start,
            });
            document.body = Some(ByteRange {
                start: line_end,
                end: document.raw.len(),
            });
            let frontmatter = &text[frontmatter_start..line_start];
            match parse_metadata(path, frontmatter) {
                Ok((metadata, mut yaml_findings)) => {
                    document.metadata = Some(metadata);
                    findings.append(&mut yaml_findings);
                }
                Err(finding) => findings.push(finding),
            }
            return Ok(Parsed {
                value: document,
                findings,
            });
        }
        if line_end == document.raw.len() {
            break;
        }
        line_start = line_end;
    }

    findings.push(parse_finding(
        Severity::Error,
        FindingCode::OKF102,
        path,
        1,
        1,
        "concept has no closing frontmatter delimiter",
    ));
    Ok(Parsed {
        value: document,
        findings,
    })
}

fn physical_line(raw: &[u8], start: usize) -> (usize, usize) {
    let Some(relative_newline) = raw[start..].iter().position(|byte| *byte == b'\n') else {
        return (raw.len(), raw.len());
    };
    let newline = start + relative_newline;
    let content_end = if newline > start && raw[newline - 1] == b'\r' {
        newline - 1
    } else {
        newline
    };
    (content_end, newline + 1)
}

fn byte_position(raw: &[u8], byte_offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    for character in String::from_utf8_lossy(&raw[..byte_offset]).chars() {
        if character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

pub(crate) fn parse_finding(
    severity: Severity,
    code: FindingCode,
    path: &str,
    line: usize,
    column: usize,
    message: impl Into<String>,
) -> Finding {
    Finding {
        severity,
        code,
        path: path.to_owned(),
        line: Some(line),
        column: Some(column),
        spec_section: Some("4.1".to_owned()),
        message: message.into(),
    }
}
