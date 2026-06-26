/// Database operations for conversation compression backfill
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection};
use std::path::Path;
use tracing::debug;

use crate::codec;
use crate::report::Report;

#[derive(Debug, Clone)]
pub struct CompressionStats {
    pub total_rows: usize,
    pub compressed_rows: usize,
    pub uncompressed_rows: usize,
    #[allow(dead_code)]
    pub compressed_bytes: u64,
    #[allow(dead_code)]
    pub uncompressed_bytes: u64,
    pub total_size: u64,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_mode(path, false)
    }

    pub fn open_readonly(path: &Path) -> Result<Self> {
        Self::open_with_mode(path, true)
    }

    fn open_with_mode(path: &Path, readonly: bool) -> Result<Self> {
        let conn = Connection::open(path)?;

        // Set query_only mode for dry-run (read-only)
        if readonly {
            conn.execute("PRAGMA query_only = ON;", [])?;
        }

        // Enable WAL mode and busy timeout
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 10000;
             PRAGMA temp_store = MEMORY;",
        )?;

        Ok(Self { conn })
    }

    /// Count rows where is_compressed = 0 AND context IS NOT NULL
    pub fn count_uncompressed_rows(&self) -> Result<usize> {
        let mut stmt =
            self.conn
                .prepare("SELECT COUNT(*) FROM conversations WHERE is_compressed = 0 AND context IS NOT NULL")?;
        let count: usize = stmt.query_row([], |row| row.get(0))?;
        Ok(count)
    }

    /// Get compression statistics
    pub fn get_compression_stats(&self) -> Result<CompressionStats> {
        let mut stmt = self.conn.prepare(
            "SELECT
                COUNT(*) as total,
                SUM(CASE WHEN is_compressed = 1 THEN 1 ELSE 0 END) as compressed,
                SUM(CASE WHEN is_compressed = 0 THEN 1 ELSE 0 END) as uncompressed,
                SUM(CASE WHEN is_compressed = 1 THEN COALESCE(LENGTH(context_zstd), 0) ELSE 0 END) as compressed_bytes,
                SUM(CASE WHEN is_compressed = 0 THEN COALESCE(LENGTH(context), 0) ELSE 0 END) as uncompressed_bytes
             FROM conversations",
        )?;

        let stats = stmt.query_row([], |row| {
            let total: usize = row.get(0)?;
            let compressed: usize = row.get::<_, Option<usize>>(1)?.unwrap_or(0);
            let uncompressed: usize = row.get::<_, Option<usize>>(2)?.unwrap_or(0);
            let compressed_bytes: u64 = row.get::<_, Option<u64>>(3)?.unwrap_or(0);
            let uncompressed_bytes: u64 = row.get::<_, Option<u64>>(4)?.unwrap_or(0);

            Ok(CompressionStats {
                total_rows: total,
                compressed_rows: compressed,
                uncompressed_rows: uncompressed,
                compressed_bytes,
                uncompressed_bytes,
                total_size: 0, // Will be filled in separately
            })
        })?;

        Ok(stats)
    }

    /// Compress a batch of uncompressed rows (WHERE is_compressed = 0 AND context IS NOT NULL)
    ///
    /// Returns the number of rows successfully compressed.
    /// Rows that fail round-trip verification are skipped and logged.
    pub fn compress_batch(&mut self, batch_size: usize, report: &mut Report) -> Result<usize> {
        // Fetch uncompressed rows in a separate scope so statement is dropped before transaction
        let rows: Vec<(String, String)> = {
            let mut stmt = self.conn.prepare(
                "SELECT conversation_id, context FROM conversations
                 WHERE is_compressed = 0 AND context IS NOT NULL
                 LIMIT ?",
            )?;

            stmt.query_map(params![batch_size], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?
        };

        if rows.is_empty() {
            return Ok(0);
        }

        debug!("Fetched {} rows for compression", rows.len());

        let mut tx = self.conn.transaction()?;
        let mut compressed_count = 0;
        let mut skipped_count = 0;

        for (conv_id, context) in rows {
            match compress_row_in_tx(&mut tx, &conv_id, &context, report) {
                Ok(_) => {
                    compressed_count += 1;
                }
                Err(e) => {
                    debug!(
                        "Skipping row {}: failed round-trip verification: {}",
                        conv_id, e
                    );
                    skipped_count += 1;
                    report.skip_row(&conv_id, &format!("{}", e));
                }
            }
        }

        tx.commit()?;

        if skipped_count > 0 {
            debug!(
                "Batch: {} compressed, {} skipped (failed verification)",
                compressed_count, skipped_count
            );
        }

        Ok(compressed_count)
    }
}

/// Compress a single row within a transaction: read context, compress, round-trip verify, write back
fn compress_row_in_tx(
    tx: &mut rusqlite::Transaction<'_>,
    conv_id: &str,
    context: &str,
    report: &mut Report,
) -> Result<()> {
    // Compress
    let compressed = codec::compress(context)?;

    // Lossless verification: decompress and compare to original
    let decompressed = codec::decompress(&compressed)?;
    if decompressed != context {
        return Err(anyhow!(
            "Round-trip verification failed: decompressed != original"
        ));
    }

    // Record stats before write
    let before_size = context.len() as u64;
    let after_size = compressed.len() as u64;
    let saving = before_size.saturating_sub(after_size);

    // Write to database
    tx.execute(
        "UPDATE conversations
         SET context_zstd = ?, is_compressed = 1, context = NULL
         WHERE conversation_id = ?",
        params![&compressed, conv_id],
    )?;

    report.compress_row(before_size, after_size, saving);

    debug!(
        "Compressed row {} ({} → {} bytes, saved {} bytes)",
        conv_id, before_size, after_size, saving
    );

    Ok(())
}
