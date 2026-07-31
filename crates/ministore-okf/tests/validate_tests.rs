use std::fs;
use std::path::{Path, PathBuf};

use ministore_okf::{
    validate_bundle, Finding, FindingCode, OkfError, Severity, ValidateOptions, ValidationSummary,
};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

#[derive(Deserialize, Serialize)]
struct NormalizedValidation {
    fixture: String,
    concepts: usize,
    errors: usize,
    warnings: usize,
    findings: Vec<NormalizedFinding>,
}

#[derive(Deserialize, Serialize)]
struct NormalizedFinding {
    severity: Severity,
    code: FindingCode,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    column: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    spec_section: Option<String>,
}

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("testdata/okf/v0.2")
        .join(relative)
}

#[test]
fn matches_base_validation_golden() {
    let fixture_root = fixture("");
    let golden = fs::read_to_string(fixture_root.join("expected/base-validation.jsonl")).unwrap();
    for line in golden.lines() {
        let expected: NormalizedValidation = serde_json::from_str(line).unwrap();
        let mut findings = Vec::new();
        let summary = validate_bundle(
            &fixture_root.join(&expected.fixture),
            &ValidateOptions::default(),
            |finding| {
                findings.push(finding);
                Ok(())
            },
        )
        .unwrap();
        let actual = NormalizedValidation {
            fixture: expected.fixture,
            concepts: summary.concepts,
            errors: summary.errors,
            warnings: summary.warnings,
            findings: normalize_findings(findings),
        };
        assert_eq!(serde_json::to_string(&actual).unwrap(), line);
    }
}

fn normalize_findings(findings: Vec<Finding>) -> Vec<NormalizedFinding> {
    findings
        .into_iter()
        .map(|finding| {
            let (line, column) = if finding.code == FindingCode::OKF103 {
                (None, None)
            } else {
                (finding.line, finding.column)
            };
            NormalizedFinding {
                severity: finding.severity,
                code: finding.code,
                path: finding.path,
                line,
                column,
                spec_section: finding.spec_section,
            }
        })
        .collect()
}

#[test]
fn validates_base_conformance_fixtures() {
    struct Case<'a> {
        name: &'a str,
        concepts: usize,
        errors: usize,
        warnings: usize,
        codes: &'a [FindingCode],
    }
    let cases = [
        Case {
            name: "valid/minimal",
            concepts: 1,
            errors: 0,
            warnings: 0,
            codes: &[],
        },
        Case {
            name: "invalid/missing-frontmatter",
            concepts: 1,
            errors: 1,
            warnings: 0,
            codes: &[FindingCode::OKF101],
        },
        Case {
            name: "invalid/missing-type",
            concepts: 1,
            errors: 1,
            warnings: 0,
            codes: &[FindingCode::OKF105],
        },
        Case {
            name: "invalid/invalid-yaml",
            concepts: 1,
            errors: 1,
            warnings: 0,
            codes: &[FindingCode::OKF103],
        },
        Case {
            name: "invalid/malformed-index",
            concepts: 1,
            errors: 2,
            warnings: 0,
            codes: &[FindingCode::OKF201, FindingCode::OKF202],
        },
        Case {
            name: "invalid/malformed-log",
            concepts: 0,
            errors: 1,
            warnings: 0,
            codes: &[FindingCode::OKF203],
        },
    ];

    for case in cases {
        let mut findings = Vec::new();
        let summary = validate_bundle(
            &fixture(case.name),
            &ValidateOptions::default(),
            |finding| {
                findings.push(finding);
                Ok(())
            },
        )
        .unwrap();
        assert_summary(&summary, case.concepts, case.errors, case.warnings);
        assert_codes(&findings, case.codes);
    }
}

#[test]
fn emits_findings_in_bytewise_path_order() {
    let bundle = TempDir::new().unwrap();
    write_concept(bundle.path(), "a/z.md", "---\ntitle: Missing\n---\n");
    write_concept(bundle.path(), "a-.md", "no frontmatter\n");
    write_concept(bundle.path(), "b.md", "---\ntype: 42\n---\n");

    let mut paths = Vec::new();
    let summary = validate_bundle(
        bundle.path(),
        &ValidateOptions {
            target_version: Some("0.2".into()),
        },
        |finding| {
            paths.push(finding.path);
            Ok(())
        },
    )
    .unwrap();

    assert_eq!(paths, ["a-.md", "a/z.md", "b.md"]);
    assert_summary(&summary, 3, 3, 0);
}

#[test]
fn propagates_finding_sink_errors() {
    let error = validate_bundle(
        &fixture("invalid/missing-type"),
        &ValidateOptions::default(),
        |_| Err(OkfError::InvalidBundle("stop".into())),
    )
    .unwrap_err();
    assert!(matches!(error, OkfError::InvalidBundle(message) if message == "stop"));
}

#[cfg(unix)]
#[test]
fn does_not_follow_symlinks() {
    use std::os::unix::fs::symlink;

    let bundle = TempDir::new().unwrap();
    let external = TempDir::new().unwrap();
    let external_concept = external.path().join("outside.md");
    fs::write(
        &external_concept,
        "---\ntitle: would fail if followed\n---\n",
    )
    .unwrap();
    symlink(&external_concept, bundle.path().join("linked.md")).unwrap();

    let mut findings = Vec::new();
    let summary = validate_bundle(bundle.path(), &ValidateOptions::default(), |finding| {
        findings.push(finding);
        Ok(())
    })
    .unwrap();
    assert_summary(&summary, 0, 0, 1);
    assert_codes(&findings, &[FindingCode::OKF205]);
    assert_eq!(findings[0].path, "linked.md");
}

fn write_concept(root: &Path, relative: &str, source: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
}

fn assert_summary(summary: &ValidationSummary, concepts: usize, errors: usize, warnings: usize) {
    assert_eq!(summary.target_version, "0.2");
    assert_eq!(summary.declared_version, None);
    assert_eq!(summary.concepts, concepts);
    assert_eq!(summary.errors, errors);
    assert_eq!(summary.warnings, warnings);
    assert_eq!(summary.ok(), errors == 0);
}

fn assert_codes(findings: &[Finding], expected: &[FindingCode]) {
    let actual: Vec<_> = findings.iter().map(|finding| finding.code).collect();
    assert_eq!(actual, expected);
}
