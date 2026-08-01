//! Ministore library: single-file search index on SQLite.

pub mod error;
pub mod schema;
pub mod index;
pub mod item;
pub mod query;
pub mod cursor;
pub mod discover;
pub mod stats;
pub mod batch;
pub mod db;
pub mod constants;
pub mod output;

// Re-exports
pub use error::{MinistoreError, Result};
pub use schema::{Schema, FieldSpec, FieldType};
pub use item::{ItemDoc, ItemMeta, ItemView};
pub use index::{
    Index, IndexOptions, RankMode, CursorMode, 
    OutputFieldSelector, SearchResultPage,
};
pub use db::search::SearchOptions;
pub use batch::{Batch, BatchWriter};
pub use output::{format_search_results, SearchOutputFormat, SearchOutputOptions};
