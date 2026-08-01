use std::fs;

use ministore::{Index, IndexOptions};
use ministore_okf::{projection_schema, sync, walk_projections, SyncOptions, ValidateOptions};

#[test]
fn projections_build_graph_and_sync_idempotently() {
    let bundle = tempfile::tempdir().unwrap();
    fs::write(bundle.path().join("a.md"),"---\ntype: Note\ntags: [z, a, a]\nverified:\n  by: human:alice\n  at: 2026-01-02T03:04:05Z\n---\nHello [B](b.md).\n").unwrap();
    fs::write(bundle.path().join("b.md"), "---\ntype: Note\n---\nWorld\n").unwrap();
    let mut projections = Vec::new();
    let summary = walk_projections(bundle.path(), &ValidateOptions::default(), |p| {
        projections.push(p);
        Ok(())
    })
    .unwrap();
    assert_eq!(summary.errors, 0);
    assert_eq!(projections[0]["link_targets"], serde_json::json!(["/b"]));
    assert_eq!(projections[1]["backlinks"], serde_json::json!(["/a"]));
    assert_eq!(projections[0]["trust_tier"], "human-reviewed");
    let target = tempfile::tempdir().unwrap();
    let index = Index::create(
        target.path().join("index.db"),
        projection_schema(),
        IndexOptions::default(),
    )
    .unwrap();
    let first = sync(bundle.path(), &index, &SyncOptions::default()).unwrap();
    assert!(first.ok);
    assert_eq!(first.added, 2);
    let second = sync(bundle.path(), &index, &SyncOptions::default()).unwrap();
    assert_eq!(second.unchanged, 2);
    assert_eq!(second.updated, 0);
}

#[test]
fn minimal_projection_matches_golden() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/okf/v0.2");
    let mut got = String::new();
    walk_projections(
        &root.join("valid/minimal"),
        &ValidateOptions::default(),
        |p| {
            got.push_str(&serde_json::to_string(&p)?);
            got.push('\n');
            Ok(())
        },
    )
    .unwrap();
    let want = fs::read_to_string(root.join("expected/minimal-projection.jsonl")).unwrap();
    assert_eq!(got, want);
}

#[test]
fn dry_run_and_invalid_bundle_do_not_change_target() {
    let bundle = tempfile::tempdir().unwrap();
    let one = bundle.path().join("one.md");
    fs::write(&one, "---\ntype: Note\n---\nBody\n").unwrap();
    let target = tempfile::tempdir().unwrap();
    let index = Index::create(
        target.path().join("index.db"),
        projection_schema(),
        IndexOptions::default(),
    )
    .unwrap();
    sync(bundle.path(), &index, &SyncOptions::default()).unwrap();
    fs::write(bundle.path().join("two.md"), "---\ntype: Note\n---\nTwo\n").unwrap();
    let report = sync(
        bundle.path(),
        &index,
        &SyncOptions {
            dry_run: true,
            ..SyncOptions::default()
        },
    )
    .unwrap();
    assert_eq!(report.added, 1);
    assert!(index.get("/two").is_err());
    fs::write(&one, "not frontmatter\n").unwrap();
    let report = sync(bundle.path(), &index, &SyncOptions::default()).unwrap();
    assert!(!report.ok);
    assert!(index.get("/one").is_ok());
}

#[test]
fn projection_hash_canonicalizes_small_exponents() {
    let bundle = tempfile::tempdir().unwrap();
    fs::write(
        bundle.path().join("x.md"),
        "---\ntype: Note\nsources:\n - resource: x\n   usage_count: 0.0000001\n---\nx\n",
    )
    .unwrap();
    let mut hash = String::new();
    walk_projections(bundle.path(), &ValidateOptions::default(), |p| {
        hash = p["okf_projection_hash"].as_str().unwrap().to_owned();
        Ok(())
    })
    .unwrap();
    assert_eq!(
        hash,
        "c4da14b0d649d845d0fdaac4ff318506ba5bf52841a2f7556d3f27cf2f68a5d3"
    );
}
