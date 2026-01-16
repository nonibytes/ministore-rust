pub mod sql;
pub mod ddl;
pub mod meta;
pub mod verify;
pub mod put;
pub mod delete;
pub mod search;
pub mod migrate;

use rusqlite::Connection;
use std::path::Path;
use crate::Result;

pub struct Db {
    pub conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Ok(Self { conn })
    }
}
