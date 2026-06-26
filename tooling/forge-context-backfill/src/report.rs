/// Reporting and statistics collection during backfill
use humansize::{format_size, BINARY};
use std::time::Duration;

use crate::db::CompressionStats;

pub struct Report {
    #[allow(dead_code)]
    total_rows: usize,
    compressed_rows: usize,
    skipped_rows: usize,
    total_bytes_before: u64,
    total_bytes_after: u64,
    total_savings: u64,
    skipped_details: Vec<(String, String)>,
}

impl Report {
    pub fn new(expected_rows: usize) -> Self {
        Self {
            total_rows: expected_rows,
            compressed_rows: 0,
            skipped_rows: 0,
            total_bytes_before: 0,
            total_bytes_after: 0,
            total_savings: 0,
            skipped_details: Vec::new(),
        }
    }

    pub fn compress_row(&mut self, before: u64, after: u64, saving: u64) {
        self.compressed_rows += 1;
        self.total_bytes_before += before;
        self.total_bytes_after += after;
        self.total_savings += saving;
    }

    pub fn skip_row(&mut self, conv_id: &str, reason: &str) {
        self.skipped_rows += 1;
        self.skipped_details.push((conv_id.to_string(), reason.to_string()));
    }

    pub fn print(
        &self,
        initial: &CompressionStats,
        final_stats: &CompressionStats,
        elapsed: Duration,
    ) {
        eprintln!(
            "\n╔════════════════════════════════════════════════════════════════╗\n\
             ║ COMPRESSION REPORT                                             ║\n\
             ╠════════════════════════════════════════════════════════════════╣"
        );

        eprintln!(
            "║ Rows processed:           {:>44} ║",
            self.compressed_rows
        );
        eprintln!(
            "║ Rows skipped (failed):    {:>44} ║",
            self.skipped_rows
        );

        eprintln!(
            "║ Space before:             {:>44} ║",
            format_size(self.total_bytes_before, BINARY)
        );
        eprintln!(
            "║ Space after:              {:>44} ║",
            format_size(self.total_bytes_after, BINARY)
        );
        eprintln!(
            "║ Space saved:              {:>44} ║",
            format_size(self.total_savings, BINARY)
        );

        if self.total_bytes_before > 0 {
            let ratio = self.total_savings as f64 / self.total_bytes_before as f64 * 100.0;
            eprintln!(
                "║ Compression ratio:        {:>44} ║",
                format!("{:.1}% reduction", ratio)
            );
        }

        eprintln!(
            "║ Time elapsed:             {:>44} ║",
            format!("{:.2}s", elapsed.as_secs_f64())
        );

        if self.compressed_rows > 0 {
            let rows_per_sec = self.compressed_rows as f64 / elapsed.as_secs_f64();
            eprintln!(
                "║ Throughput:               {:>44} ║",
                format!("{:.1} rows/sec", rows_per_sec)
            );
        }

        eprintln!(
            "╠════════════════════════════════════════════════════════════════╣"
        );

        eprintln!(
            "║ Initial state:            {} total, {} compressed, {} uncompressed ║",
            initial.total_rows,
            initial.compressed_rows,
            initial.uncompressed_rows
        );
        eprintln!(
            "║ Final state:              {} total, {} compressed, {} uncompressed ║",
            final_stats.total_rows,
            final_stats.compressed_rows,
            final_stats.uncompressed_rows
        );

        if !self.skipped_details.is_empty() {
            eprintln!("╠════════════════════════════════════════════════════════════════╣");
            eprintln!("║ Skipped rows (failed round-trip verification):                 ║");
            for (conv_id, reason) in &self.skipped_details {
                let truncated_id = if conv_id.len() > 30 {
                    format!("{}...", &conv_id[..27])
                } else {
                    conv_id.clone()
                };
                eprintln!(
                    "║   {} ({})                        ║",
                    truncated_id,
                    reason
                );
            }
        }

        eprintln!("╚════════════════════════════════════════════════════════════════╝");
    }
}
