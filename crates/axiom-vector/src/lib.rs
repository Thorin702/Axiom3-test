//! Axiom3 vector store — LanceDB-backed embedding index.
//!
//! Implements [`axiom_core::store::VectorStore`], a SYNC trait, on top of
//! LanceDB's ASYNC API. The bridge (design decision B): [`LanceStore`] owns a
//! dedicated Tokio runtime and `block_on`s LanceDB calls. The shell must therefore
//! drive vector ops from a blocking context (`spawn_blocking`) — the same way it
//! drives the rusqlite-backed stores — so we never `block_on` inside an async task
//! (which would panic).
//!
//! # Confidentiality boundary (read before deploying)
//! This is the UNENCRYPTED, rebuildable cache tier: vectors are derived artifacts
//! re-creatable from the SQLCipher-encrypted durable store, so they live OUTSIDE
//! SQLCipher (LanceDB has no transparent at-rest encryption). Embeddings can be
//! *partially inverted* back toward their source text, so the on-disk dataset must
//! be treated as sensitive: confidentiality relies on **OS-level disk encryption**
//! (FileVault / BitLocker). On Unix we additionally chmod the dataset dir to `0o700`
//! as defence-in-depth, but that is not a substitute for disk encryption.
//!
//! # Scoping
//! One `LanceStore` == one project: isolation is by the on-disk *path* (each
//! project's `.workbench/` is separate), mirroring the per-project durable store /
//! keychain key. The single `vectors` table is therefore not shared across projects.
//!
//! # Distance metric
//! `search` uses LanceDB's default **L2 (Euclidean)** distance. Callers that want
//! cosine similarity must L2-normalise embeddings before `upsert`/`search` (on
//! unit vectors, L2 ordering equals cosine ordering).
//!
//! # Deferred (documented follow-ups, not defects in this increment)
//! - No scalar index on `id`, so `merge_insert` joins via a full-table scan
//!   (O(n) per upsert). Fine at cache-tier sizes; a scalar index + periodic
//!   `optimize()` is the scale fix.
//! - Single-row `upsert` writes one LanceDB fragment per call (write amplification);
//!   a batched `upsert_many` + compaction is the scale fix.
//! - `search` is brute-force (no ANN index): O(n) scan. An IVF/HNSW index is the
//!   scale fix once row counts grow.

use std::path::Path;
use std::sync::Arc;

use arrow_array::{Array, FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator, StringArray};
use arrow_schema::{ArrowError, DataType, Field, Schema, SchemaRef};
use axiom_core::store::{Result, StoreError, VectorStore};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::Table;

/// The single table this store manages.
const TABLE: &str = "vectors";

fn map_lance(e: lancedb::Error) -> StoreError {
    StoreError::Backend(e.to_string())
}

fn map_arrow(e: ArrowError) -> StoreError {
    StoreError::Backend(e.to_string())
}

/// Extract the fixed embedding dimension from a persisted table schema.
fn vector_dim(schema: &Schema) -> Result<i32> {
    let field = schema
        .field_with_name("vector")
        .map_err(|e| StoreError::Backend(format!("table missing `vector` column: {e}")))?;
    match field.data_type() {
        DataType::FixedSizeList(_, size) => Ok(*size),
        other => Err(StoreError::Backend(format!(
            "`vector` column has unexpected type {other:?} (expected FixedSizeList)"
        ))),
    }
}

/// Best-effort: keep the cache dataset directory non-world-readable. Never fails
/// `open()` — disk encryption, not file mode, is the real confidentiality control.
#[cfg(unix)]
fn harden_dir_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

/// A LanceDB-backed [`VectorStore`] for fixed-dimension embeddings.
///
/// Owns its own Tokio runtime; all trait methods are synchronous and bridge to
/// LanceDB's async API via `block_on`. Do not call these methods from within an
/// async runtime — wrap them in `spawn_blocking`.
pub struct LanceStore {
    rt: tokio::runtime::Runtime,
    table: Table,
    schema: SchemaRef,
    dim: i32,
}

impl LanceStore {
    /// Open (or create) the vector store at `path` for embeddings of dimension
    /// `dim`. On first open the `vectors` table is created with a fixed `dim`; on
    /// reopen the persisted dimension is authoritative and a mismatch is rejected.
    pub fn open(path: &Path, dim: usize) -> Result<Self> {
        // Validate before the lossy `as i32` cast: a 0 dim yields a meaningless
        // schema; a dim > i32::MAX would wrap negative.
        if dim == 0 || dim > i32::MAX as usize {
            return Err(StoreError::Backend(format!(
                "embedding dimension {dim} out of range [1, {}]",
                i32::MAX
            )));
        }
        let uri = path
            .to_str()
            .ok_or_else(|| StoreError::Backend("vector store path is not valid UTF-8".into()))?
            .to_string();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| StoreError::Backend(e.to_string()))?;

        let dim = dim as i32;
        let item_field = Arc::new(Field::new("item", DataType::Float32, true));
        let schema: SchemaRef = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("vector", DataType::FixedSizeList(item_field, dim), true),
        ]));

        let schema_for_create = schema.clone();
        let (table, persisted_schema) = rt
            .block_on(async {
                let conn = lancedb::connect(&uri).execute().await?;
                let names = conn.table_names().execute().await?;
                let table = if names.iter().any(|n| n == TABLE) {
                    conn.open_table(TABLE).execute().await?
                } else {
                    conn.create_empty_table(TABLE, schema_for_create).execute().await?
                };
                let schema = table.schema().await?;
                Ok::<_, lancedb::Error>((table, schema))
            })
            .map_err(map_lance)?;

        // Reject a reopen whose requested dim disagrees with what's on disk, up
        // front — otherwise it only surfaces as a confusing write-time error.
        let persisted_dim = vector_dim(&persisted_schema)?;
        if persisted_dim != dim {
            return Err(StoreError::Backend(format!(
                "existing vector table has dim {persisted_dim}, but {dim} was requested"
            )));
        }

        #[cfg(unix)]
        harden_dir_permissions(path);

        Ok(Self { rt, table, schema, dim })
    }

    /// Build a single-row Arrow batch `(id, vector)` matching `self.schema`.
    fn row_batch(&self, id: &str, embedding: &[f32]) -> Result<RecordBatch> {
        if embedding.len() != self.dim as usize {
            return Err(StoreError::Backend(format!(
                "embedding dim {} != table dim {}",
                embedding.len(),
                self.dim
            )));
        }
        // merge_insert bypasses LanceDB's own NaN guard (which only runs on the
        // `.add()` path), so non-finite values would silently corrupt distances.
        if embedding.iter().any(|v| !v.is_finite()) {
            return Err(StoreError::Backend(
                "embedding contains non-finite values (NaN or inf)".into(),
            ));
        }
        let item_field = Arc::new(Field::new("item", DataType::Float32, true));
        let values = Float32Array::from(embedding.to_vec());
        let list = FixedSizeListArray::try_new(item_field, self.dim, Arc::new(values), None)
            .map_err(map_arrow)?;
        let ids = StringArray::from(vec![id]);
        RecordBatch::try_new(self.schema.clone(), vec![Arc::new(ids), Arc::new(list)]).map_err(map_arrow)
    }
}

impl VectorStore for LanceStore {
    fn upsert(&self, id: &str, embedding: &[f32]) -> Result<()> {
        if id.is_empty() {
            return Err(StoreError::Backend("id must not be empty".into()));
        }
        let batch = self.row_batch(id, embedding)?;
        let schema = self.schema.clone();
        self.rt
            .block_on(async move {
                let batches = vec![Ok::<RecordBatch, ArrowError>(batch)];
                let reader = RecordBatchIterator::new(batches, schema);
                // Replace the row if `id` already exists, else insert it.
                let mut mib = self.table.merge_insert(&["id"]);
                mib.when_matched_update_all(None);
                mib.when_not_matched_insert_all();
                mib.execute(Box::new(reader)).await
            })
            .map_err(map_lance)?;
        Ok(())
    }

    fn search(&self, query: &[f32], k: usize) -> Result<Vec<(String, f32)>> {
        if query.len() != self.dim as usize {
            return Err(StoreError::Backend(format!(
                "query dim {} != table dim {}",
                query.len(),
                self.dim
            )));
        }
        if query.iter().any(|v| !v.is_finite()) {
            return Err(StoreError::Backend(
                "query contains non-finite values (NaN or inf)".into(),
            ));
        }
        self.rt.block_on(async move {
            let mut stream = self
                .table
                .query()
                .nearest_to(query)
                .map_err(map_lance)?
                .limit(k)
                .execute()
                .await
                .map_err(map_lance)?;

            let mut out = Vec::new();
            while let Some(batch) = stream
                .try_next()
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?
            {
                let ids = batch
                    .column_by_name("id")
                    .and_then(|a| a.as_any().downcast_ref::<StringArray>())
                    .ok_or_else(|| StoreError::Backend("result missing `id` column".into()))?;
                // Vector search projects a `_distance` Float32 column.
                let dists = batch
                    .column_by_name("_distance")
                    .and_then(|a| a.as_any().downcast_ref::<Float32Array>())
                    .ok_or_else(|| StoreError::Backend("result missing `_distance` column".into()))?;
                for i in 0..batch.num_rows() {
                    out.push((ids.value(i).to_string(), dists.value(i)));
                }
            }
            Ok(out)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn upsert_then_search_returns_nearest_first() {
        let dir = tempdir().unwrap();
        let store = LanceStore::open(dir.path(), 3).unwrap();
        store.upsert("a", &[1.0, 0.0, 0.0]).unwrap();
        store.upsert("b", &[0.0, 1.0, 0.0]).unwrap();
        store.upsert("c", &[0.0, 0.0, 1.0]).unwrap();

        let hits = store.search(&[0.9, 0.1, 0.0], 2).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].0, "a", "closest to (0.9,0.1,0) should be a");
    }

    #[test]
    fn upsert_replaces_existing_id_not_duplicates() {
        let dir = tempdir().unwrap();
        let store = LanceStore::open(dir.path(), 3).unwrap();
        store.upsert("x", &[1.0, 0.0, 0.0]).unwrap();
        store.upsert("x", &[0.0, 0.0, 1.0]).unwrap(); // overwrite same id

        let hits = store.search(&[0.0, 0.0, 1.0], 10).unwrap();
        let count_x = hits.iter().filter(|(id, _)| id == "x").count();
        assert_eq!(count_x, 1, "merge_insert on id must replace, not duplicate");
        assert_eq!(hits[0].0, "x");
    }

    #[test]
    fn data_persists_across_reopen() {
        let dir = tempdir().unwrap();
        {
            let store = LanceStore::open(dir.path(), 3).unwrap();
            store.upsert("p", &[1.0, 2.0, 3.0]).unwrap();
        }
        let store = LanceStore::open(dir.path(), 3).unwrap();
        let hits = store.search(&[1.0, 2.0, 3.0], 1).unwrap();
        assert_eq!(hits[0].0, "p");
    }

    #[test]
    fn dimension_mismatch_is_rejected() {
        let dir = tempdir().unwrap();
        let store = LanceStore::open(dir.path(), 3).unwrap();
        assert!(store.upsert("a", &[1.0, 0.0]).is_err());
        assert!(store.search(&[1.0, 0.0], 1).is_err());
    }

    #[test]
    fn reopen_with_wrong_dim_is_rejected() {
        let dir = tempdir().unwrap();
        {
            let store = LanceStore::open(dir.path(), 3).unwrap();
            store.upsert("p", &[1.0, 2.0, 3.0]).unwrap();
        }
        // Reopening the same dataset with a different dim must fail fast.
        let err = LanceStore::open(dir.path(), 4);
        assert!(err.is_err(), "reopen with mismatched dim must be rejected");
    }

    #[test]
    fn zero_or_huge_dim_is_rejected() {
        let dir = tempdir().unwrap();
        assert!(LanceStore::open(dir.path(), 0).is_err());
        assert!(LanceStore::open(dir.path(), (i32::MAX as usize) + 1).is_err());
    }

    #[test]
    fn empty_id_is_rejected() {
        let dir = tempdir().unwrap();
        let store = LanceStore::open(dir.path(), 3).unwrap();
        assert!(store.upsert("", &[1.0, 2.0, 3.0]).is_err());
    }

    #[test]
    fn non_finite_values_are_rejected() {
        let dir = tempdir().unwrap();
        let store = LanceStore::open(dir.path(), 3).unwrap();
        assert!(store.upsert("a", &[1.0, f32::NAN, 3.0]).is_err(), "NaN embedding");
        assert!(store.upsert("b", &[1.0, f32::INFINITY, 3.0]).is_err(), "inf embedding");
        store.upsert("c", &[1.0, 2.0, 3.0]).unwrap();
        assert!(store.search(&[f32::NAN, 2.0, 3.0], 1).is_err(), "NaN query");
    }
}
