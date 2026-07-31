use std::fs;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use tempfile::{Builder, TempDir};

use crate::finding::Finding;
use crate::model::ValidationSummary;
use crate::Result;

const STAGE_DDL: &str = r#"
CREATE TABLE entries (
    path TEXT PRIMARY KEY COLLATE BINARY,
    kind TEXT NOT NULL
);
CREATE TABLE findings (
    id INTEGER PRIMARY KEY,
    severity TEXT NOT NULL,
    code TEXT NOT NULL,
    path TEXT NOT NULL COLLATE BINARY,
    line INTEGER,
    column INTEGER,
    spec_section TEXT,
    finding_json BLOB NOT NULL
);
CREATE INDEX findings_order ON findings(
    path COLLATE BINARY,
    COALESCE(line, 0),
    COALESCE(column, 0),
    CASE severity WHEN 'error' THEN 0 ELSE 1 END,
    code,
    id
);
"#;

pub(crate) struct ValidationStage {
    connection: Connection,
    _directory: TempDir,
    #[cfg(test)]
    path: PathBuf,
}

impl ValidationStage {
    pub(crate) fn create() -> Result<Self> {
        Self::create_in(None)
    }

    fn create_in(parent: Option<&Path>) -> Result<Self> {
        let mut builder = Builder::new();
        builder.prefix("ministore-okf-");
        let directory = match parent {
            Some(parent) => builder.tempdir_in(parent)?,
            None => builder.tempdir()?,
        };
        let path = directory.path().join("stage.sqlite3");
        let connection = Connection::open(&path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }

        connection.execute_batch(STAGE_DDL)?;
        Ok(Self {
            connection,
            _directory: directory,
            #[cfg(test)]
            path,
        })
    }

    pub(crate) fn transaction(&mut self) -> Result<Transaction<'_>> {
        Ok(self.connection.transaction()?)
    }

    pub(crate) fn insert_entry(
        transaction: &Transaction<'_>,
        path: &str,
        kind: &str,
    ) -> Result<()> {
        transaction.execute(
            "INSERT INTO entries(path, kind) VALUES (?1, ?2)",
            params![path, kind],
        )?;
        Ok(())
    }

    pub(crate) fn insert_finding(transaction: &Transaction<'_>, finding: &Finding) -> Result<()> {
        let encoded = serde_json::to_vec(finding)?;
        transaction.execute(
            r#"INSERT INTO findings(
                severity, code, path, line, column, spec_section, finding_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![
                finding.severity.as_str(),
                finding.code.as_str(),
                &finding.path,
                finding.line,
                finding.column,
                finding.spec_section.as_deref(),
                encoded,
            ],
        )?;
        Ok(())
    }

    pub(crate) fn next_entry_path(&self, kind: &str, after: &str) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                r#"SELECT path FROM entries
                   WHERE kind = ?1 AND path > ?2 COLLATE BINARY
                   ORDER BY path COLLATE BINARY LIMIT 1"#,
                params![kind, after],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub(crate) fn summary(&self, bundle: &str, target_version: &str) -> Result<ValidationSummary> {
        let concepts = self.connection.query_row(
            "SELECT COUNT(*) FROM entries WHERE kind = 'concept'",
            [],
            |row| row.get(0),
        )?;
        let (errors, warnings) = self.connection.query_row(
            r#"SELECT
                COALESCE(SUM(CASE WHEN severity = 'error' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN severity = 'warning' THEN 1 ELSE 0 END), 0)
               FROM findings"#,
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok(ValidationSummary {
            target_version: target_version.to_owned(),
            declared_version: None,
            bundle: bundle.to_owned(),
            concepts,
            errors,
            warnings,
        })
    }

    pub(crate) fn emit_findings<F>(&self, mut emit: F) -> Result<()>
    where
        F: FnMut(Finding) -> Result<()>,
    {
        let mut statement = self.connection.prepare(
            r#"SELECT finding_json FROM findings
               ORDER BY
                 path COLLATE BINARY,
                 COALESCE(line, 0),
                 COALESCE(column, 0),
                 CASE severity WHEN 'error' THEN 0 ELSE 1 END,
                 code,
                 id"#,
        )?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let encoded: Vec<u8> = row.get(0)?;
            emit(serde_json::from_slice(&encoded)?)?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn directory_path(&self) -> &Path {
        self._directory.path()
    }

    #[cfg(test)]
    fn database_path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_is_private_and_removed() {
        let parent = tempfile::tempdir().unwrap();
        let (directory, database) = {
            let stage = ValidationStage::create_in(Some(parent.path())).unwrap();
            let directory = stage.directory_path().to_path_buf();
            let database = stage.database_path().to_path_buf();

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                    0o700
                );
                assert_eq!(
                    fs::metadata(&database).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
            (directory, database)
        };
        assert!(!directory.exists());
        assert!(!database.exists());
    }
}
