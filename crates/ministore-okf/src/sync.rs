use std::path::Path;
use std::time::Instant;

use ministore::Index;
use rusqlite::{params, OptionalExtension};

use crate::model::{SyncOptions, SyncReport, ValidateOptions};
use crate::projection::{projection_schema, PROJECTION_VERSION};
use crate::validate::prepare_bundle;
use crate::Result;

pub fn sync(root: &Path, index: &Index, options: &SyncOptions) -> Result<SyncReport> {
    let started = Instant::now();
    let (stage, summary) = prepare_bundle(
        root,
        &ValidateOptions {
            target_version: options.target_version.clone(),
        },
    )?;
    let mut report = SyncReport {
        ok: false,
        bundle: summary.bundle.clone(),
        projection_version: PROJECTION_VERSION,
        concepts: summary.concepts,
        added: 0,
        updated: 0,
        unchanged: 0,
        deleted: 0,
        duration_ms: 0,
        validation: summary.clone(),
    };
    if summary.errors > 0 || (options.strict && summary.warnings > 0) {
        report.duration_ms = started.elapsed().as_millis();
        return Ok(report);
    }
    if index.schema() != &projection_schema() {
        return Err(crate::OkfError::InvalidBundle(
            "target index schema does not equal the OKF projection schema".into(),
        ));
    }
    index.scan_paths("", |path| {
        stage
            .connection
            .execute("INSERT INTO existing_paths(path) VALUES (?1)", [path])?;
        Ok(())
    })?;
    let mut after = String::new();
    loop {
        let row:Option<(String,Vec<u8>)>=stage.connection.query_row("SELECT path,raw FROM concepts WHERE path>?1 COLLATE BINARY ORDER BY path COLLATE BINARY LIMIT 1",[&after],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((source, raw)) = row else { break };
        after.clone_from(&source);
        let projection = stage.project(&source, &raw, &summary.target_version)?;
        let target = projection["path"].as_str().unwrap();
        let exists: Option<i64> = stage
            .connection
            .query_row(
                "SELECT 1 FROM existing_paths WHERE path=?1",
                [target],
                |r| r.get(0),
            )
            .optional()?;
        let kind = if exists.is_none() {
            "add"
        } else {
            let current = index.get(target)?;
            if current.doc.get("okf_projection_hash") == projection.get("okf_projection_hash") {
                "unchanged"
            } else {
                "update"
            }
        };
        stage.connection.execute(
            "INSERT INTO actions(path,kind) VALUES (?1,?2)",
            params![target, kind],
        )?;
    }
    stage.connection.execute("INSERT INTO actions(path,kind) SELECT path,'delete' FROM existing_paths WHERE NOT EXISTS(SELECT 1 FROM actions WHERE actions.path=existing_paths.path)",[])?;
    let counts:(usize,usize,usize,usize)=stage.connection.query_row("SELECT COALESCE(SUM(kind='add'),0),COALESCE(SUM(kind='update'),0),COALESCE(SUM(kind='unchanged'),0),COALESCE(SUM(kind='delete'),0) FROM actions",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    (
        report.added,
        report.updated,
        report.unchanged,
        report.deleted,
    ) = counts;
    report.ok = true;
    if !options.dry_run {
        index.write_batch(|writer|{let mut after=String::new();loop{let row:Option<(String,String)>=stage.connection.query_row("SELECT path,kind FROM actions WHERE kind!='unchanged' AND path>?1 COLLATE BINARY ORDER BY path COLLATE BINARY LIMIT 1",[&after],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(ministore::MinistoreError::from)?;let Some((target,kind))=row else{break};after.clone_from(&target);if kind=="delete"{writer.delete(&target)?;continue}let source=format!("{}.md",target.trim_start_matches('/'));let raw:Vec<u8>=stage.connection.query_row("SELECT raw FROM concepts WHERE path=?1",[&source],|r|r.get(0)).map_err(ministore::MinistoreError::from)?;let projection=stage.project(&source,&raw,&summary.target_version).map_err(|e|ministore::MinistoreError::Internal(e.to_string()))?;writer.put_json(serde_json::Value::Object(projection))?;}Ok(())})?;
    }
    report.duration_ms = started.elapsed().as_millis();
    Ok(report)
}
