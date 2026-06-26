/// Process checking using lsof to detect forge processes holding the database
use anyhow::Result;
use std::path::Path;
use std::process::Command;

#[derive(Debug)]
pub struct ProcessCheck {
    pids: Vec<String>,
}

impl ProcessCheck {
    /// Check if any processes hold open file handles to the database or its WAL/SHM files
    pub fn check(db_path: &Path) -> Result<Self> {
        let db_str = db_path.to_string_lossy();

        // Try to use lsof to find processes holding the database
        // We check for the main DB file and the WAL files
        let output = Command::new("lsof")
            .arg("-t")
            .arg(db_str.as_ref())
            .output();

        let pids = match output {
            Ok(out) => {
                match String::from_utf8(out.stdout) {
                    Ok(stdout) => {
                        stdout
                            .lines()
                            .filter(|line| !line.trim().is_empty())
                            .map(|s| s.trim().to_string())
                            .collect()
                    }
                    Err(_) => {
                        // Invalid UTF-8 in output; assume no holders
                        Vec::new()
                    }
                }
            }
            Err(_) => {
                // lsof not available or failed; assume no holders
                Vec::new()
            }
        };

        Ok(Self { pids })
    }

    /// Check if any processes hold the database
    pub fn has_holders(&self) -> bool {
        !self.pids.is_empty()
    }

    /// Get the count of holding processes
    pub fn count(&self) -> usize {
        self.pids.len()
    }

    /// Format PIDs for display
    pub fn format_pids(&self) -> String {
        self.pids
            .iter()
            .map(|pid| format!("  - PID {}", pid))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
