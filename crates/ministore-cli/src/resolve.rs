use std::path::PathBuf;
use ministore::Result;

pub fn resolve_index_path(index: &str) -> Result<PathBuf> {
    // 1. If it contains separator or ends with .db, use as is
    if index.contains(std::path::MAIN_SEPARATOR) || index.ends_with(".db") {
        return Ok(PathBuf::from(index));
    }

    // 2. Otherwise assume it's a bare name in current directory -> ./<name>.db
    Ok(std::env::current_dir()?.join(format!("{}.db", index)))
}
