use std::fs;
use std::path::Path;

use crate::document::parse_document;
use crate::finding::{Finding, FindingCode, Severity};
use crate::model::{ValidateOptions, ValidationSummary};
use crate::staging::ValidationStage;
use crate::{OkfError, Result};

mod reserved;

const DEFAULT_TARGET_VERSION: &str = "0.2";

pub fn validate_bundle<F>(
    root: &Path,
    options: &ValidateOptions,
    emit: F,
) -> Result<ValidationSummary>
where
    F: FnMut(Finding) -> Result<()>,
{
    let metadata = fs::symlink_metadata(root)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(OkfError::InvalidBundle(format!(
            "bundle root is not a directory: {}",
            root.display()
        )));
    }
    let root = std::path::absolute(root)?;
    let root_string = root
        .to_str()
        .ok_or_else(|| OkfError::InvalidBundle("bundle root is not valid UTF-8".into()))?;
    let target_version = options
        .target_version
        .as_deref()
        .unwrap_or(DEFAULT_TARGET_VERSION);

    let mut stage = ValidationStage::create()?;
    {
        let transaction = stage.transaction()?;
        enumerate_directory(&transaction, &root, &root)?;
        transaction.commit()?;
    }
    validate_staged_concepts(&mut stage, &root)?;
    validate_staged_reserved_files(&mut stage, &root)?;
    let summary = stage.summary(root_string, target_version)?;
    stage.emit_findings(emit)?;
    Ok(summary)
}

fn enumerate_directory(
    transaction: &rusqlite::Transaction<'_>,
    root: &Path,
    directory: &Path,
) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let relative = relative_path(root, &path)?;
        let file_type = entry.file_type()?;
        let kind = classify_entry(&entry.file_name(), &file_type)?;
        ValidationStage::insert_entry(transaction, &relative, kind)?;

        if file_type.is_dir() {
            enumerate_directory(transaction, root, &path)?;
        } else if kind == "ignored" {
            ValidationStage::insert_finding(
                transaction,
                &Finding {
                    severity: Severity::Warning,
                    code: FindingCode::OKF205,
                    path: relative,
                    line: None,
                    column: None,
                    spec_section: Some("4.1".into()),
                    message: "symlink or special file is ignored".into(),
                },
            )?;
        }
    }
    Ok(())
}

fn relative_path(root: &Path, path: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| OkfError::InvalidBundle("bundle entry escaped its root".into()))?;
    let relative = relative
        .to_str()
        .ok_or_else(|| OkfError::InvalidBundle("bundle path is not valid UTF-8".into()))?;
    Ok(relative.replace(std::path::MAIN_SEPARATOR, "/"))
}

fn classify_entry(name: &std::ffi::OsStr, file_type: &fs::FileType) -> Result<&'static str> {
    if file_type.is_dir() {
        return Ok("ignored");
    }
    if !file_type.is_file() {
        return Ok("ignored");
    }
    let name = name
        .to_str()
        .ok_or_else(|| OkfError::InvalidBundle("bundle path is not valid UTF-8".into()))?;
    Ok(match name {
        "index.md" => "index",
        "log.md" => "log",
        _ if name.ends_with(".md") => "concept",
        _ => "ancillary",
    })
}

fn validate_staged_concepts(stage: &mut ValidationStage, root: &Path) -> Result<()> {
    let mut after = String::new();
    loop {
        let Some(relative) = stage.next_entry_path("concept", &after)? else {
            return Ok(());
        };
        after.clone_from(&relative);
        let raw = fs::read(root.join(&relative))?;
        let mut parsed = parse_document(&relative, &raw)?;
        parsed.findings.extend(validate_concept_base(&parsed.value));

        let transaction = stage.transaction()?;
        for finding in &parsed.findings {
            ValidationStage::insert_finding(&transaction, finding)?;
        }
        transaction.commit()?;
    }
}

fn validate_staged_reserved_files(stage: &mut ValidationStage, root: &Path) -> Result<()> {
    for kind in ["index", "log"] {
        let mut after = String::new();
        loop {
            let Some(relative) = stage.next_entry_path(kind, &after)? else {
                break;
            };
            after.clone_from(&relative);
            let raw = fs::read(root.join(&relative))?;
            let findings = if kind == "index" {
                reserved::validate_index(&relative, &raw)?
            } else {
                reserved::validate_log(&relative, &raw)
            };
            let transaction = stage.transaction()?;
            for finding in &findings {
                ValidationStage::insert_finding(&transaction, finding)?;
            }
            transaction.commit()?;
        }
    }
    Ok(())
}

fn validate_concept_base(document: &crate::Document) -> Vec<Finding> {
    let Some(metadata) = document.metadata() else {
        return Vec::new();
    };
    if metadata
        .string("type")
        .is_some_and(|value| !value.trim().is_empty())
    {
        return Vec::new();
    }

    let position = metadata
        .get("type")
        .and_then(|node| metadata.resolve(node))
        .map(|node| node.position);
    vec![Finding {
        severity: Severity::Error,
        code: FindingCode::OKF105,
        path: document.path().to_owned(),
        line: position.map(|position| position.line),
        column: position.map(|position| position.column),
        spec_section: Some("4.1".into()),
        message: "type must be a non-empty string".into(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(source: &str) -> crate::Document {
        parse_document("concept.md", source.as_bytes())
            .unwrap()
            .value
    }

    #[test]
    fn type_must_be_a_non_empty_string() {
        for (source, has_position) in [
            ("---\ntitle: missing\n---\n", false),
            ("---\ntype: 42\n---\n", true),
            ("---\ntype: '   '\n---\n", true),
        ] {
            let findings = validate_concept_base(&parsed(source));
            assert_eq!(findings.len(), 1);
            assert_eq!(findings[0].code, FindingCode::OKF105);
            assert_eq!(findings[0].line.is_some(), has_position);
            assert_eq!(findings[0].column.is_some(), has_position);
        }
        assert!(validate_concept_base(&parsed("---\ntype: Reference\n---\n")).is_empty());
        assert!(
            validate_concept_base(&parsed("---\nkind: &kind Reference\ntype: *kind\n---\n"))
                .is_empty()
        );
    }
}
