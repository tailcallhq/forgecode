use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::policies::operation::PermissionOperation;

/// A structured "case brief" representing a permission decision request.
///
/// Collects all evidence — tool call details, proposed changes, and policy
/// context — so the user (the "judge") can inspect everything before ruling.
/// Also emitted as a structured tracing event so the complete decision trail
/// is recorded (Prometheus/Grafana/Jaeger-style observability).
#[derive(Debug, Clone)]
pub struct PermissionCase {
    /// Unique case identifier for cross-referencing traces and logs
    pub case_id: String,
    /// ISO-8601 timestamp
    pub timestamp: String,
    /// Type of operation (Write, Patch, Read, Execute, Fetch)
    pub operation_type: &'static str,
    /// The file path the operation targets
    pub file_path: PathBuf,
    /// The full permission operation being evaluated
    pub operation: PermissionOperation,
    /// Rich description of the proposed changes (if available)
    pub changes_description: Option<String>,
    /// The reason or context provided by the caller for this operation.
    /// This is the LLM's justification for the change, extracted from the
    /// tool call's `context` field.
    pub explanation: String,
}

impl PermissionCase {
    /// Build a new case for a permission decision.
    pub fn new(
        operation_type: &'static str,
        operation: PermissionOperation,
        file_path: PathBuf,
        changes_description: Option<String>,
        explanation: String,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let millis = now.as_millis();
        // Case ID: hex-timestamp + atomic counter for uniqueness across threads
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        let case_id = format!(
            "case-{:016x}-{:04x}",
            millis,
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let timestamp = format!(
            "{}.{:03}Z",
            chrono::DateTime::from_timestamp(
                now.as_secs() as i64,
                now.subsec_nanos(),
            )
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%S").to_string())
            .unwrap_or_else(|| "unknown".to_string()),
            now.subsec_millis(),
        );

        Self {
            case_id,
            timestamp,
            operation_type,
            file_path,
            operation,
            changes_description,
            explanation,
        }
    }

    /// Render the case brief as a formatted panel suitable for stdout.
    pub fn format_panel(&self) -> String {
        let divider = "═".repeat(58);
        let thin = "─".repeat(58);

        let mut panel = String::new();
        panel.push_str(&format!("\n{divider}\n"));
        panel.push_str(&format!("  ⚖  Permission Request  │ Case #{}\n", self.case_id));
        panel.push_str(&format!("{thin}\n"));

        // Header fields
        panel.push_str(&format!("  Tool  │ {}\n", self.operation_type));
        panel.push_str(&format!(
            "  File  │ {}\n",
            self.file_path.display()
        ));
        panel.push_str(&format!("  Time  │ {}\n", self.timestamp));

        // Explanation / context from caller
        if !self.explanation.is_empty() {
            panel.push_str(&format!(
                "  Why   │ {}\n",
                self.explanation
            ));
        }

        // Proposed changes section
        if let Some(ref desc) = self.changes_description {
            panel.push_str(&format!("{thin}\n"));
            panel.push_str(&format!("{desc}"));
        }

        panel.push_str(&format!("{divider}\n"));
        panel
    }
}

impl fmt::Display for PermissionCase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format_panel())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policies::PermissionOperation;
    use std::path::PathBuf;

    #[test]
    fn test_permission_case_creation() {
        let op = PermissionOperation::Write {
            path: PathBuf::from("/tmp/test.txt"),
            cwd: PathBuf::from("/tmp"),
            message: "Write file: test.txt".to_string(),
        };

        let case = PermissionCase::new(
            "Write",
            op,
            PathBuf::from("/tmp/test.txt"),
            Some("--- Proposed content ---\nhello world\n---".to_string()),
            "Test explanation".to_string(),
        );

        let panel = case.format_panel();
        assert!(panel.contains("Permission Request"));
        assert!(panel.contains("Write"));
        assert!(panel.contains("test.txt"));
        assert!(panel.contains("hello world"));
        assert!(panel.contains("Test explanation"));
        assert!(panel.contains(&case.case_id));
    }

    #[test]
    fn test_case_id_uniqueness() {
        let op = |p| PermissionOperation::Read {
            path: PathBuf::from(p),
            cwd: PathBuf::from("/tmp"),
            message: "read".to_string(),
        };

        let a = PermissionCase::new("Read", op("/a"), PathBuf::from("/a"), None, String::new());
        let b = PermissionCase::new("Read", op("/b"), PathBuf::from("/b"), None, String::new());
        assert_ne!(a.case_id, b.case_id);
    }
}
