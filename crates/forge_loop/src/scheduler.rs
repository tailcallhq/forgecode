//! Loop scheduler - manages periodic autonomous execution

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use tokio::time::interval as tokio_interval;
use uuid::Uuid;

use crate::{LoopError, LoopExecutor, LoopState, Result};

/// Unique identifier for a loop
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LoopId(pub String);

impl std::fmt::Display for LoopId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Loop configuration
#[derive(Debug, Clone)]
pub struct LoopConfig {
    pub id: LoopId,
    pub conversation_id: String,
    pub prompt: String,
    pub interval: Duration,
    pub status: LoopStatus,
    pub created_at: DateTime<Utc>,
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: Option<DateTime<Utc>>,
}

/// Loop status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopStatus {
    Running,
    Paused,
    Stopped,
}

impl Default for LoopStatus {
    fn default() -> Self {
        Self::Stopped
    }
}

/// Core loop scheduler
pub struct LoopScheduler {
    loops: Arc<RwLock<HashMap<LoopId, LoopConfig>>>,
    executor: Arc<dyn LoopExecutor>,
    state: Arc<RwLock<LoopState>>,
}

impl LoopScheduler {
    /// Create a new loop scheduler
    pub fn new(executor: Arc<dyn LoopExecutor>) -> Self {
        let state = LoopState::load().unwrap_or_default();

        Self {
            loops: Arc::new(RwLock::new(HashMap::new())),
            executor,
            state: Arc::new(RwLock::new(state)),
        }
    }

    /// Start a new loop
    pub async fn start_loop(
        &mut self,
        interval: Duration,
        prompt: String,
        conversation_id: String,
    ) -> Result<LoopId> {
        if interval.as_secs() < 60 {
            return Err(LoopError::InvalidInterval(
                "Interval must be at least 60 seconds".to_string(),
            ));
        }

        let id = LoopId(Uuid::new_v4().to_string());
        let now = Utc::now();
        let next_run = now + chrono::Duration::from_std(interval).unwrap_or_default();

        let config = LoopConfig {
            id: id.clone(),
            conversation_id,
            prompt,
            interval,
            status: LoopStatus::Running,
            created_at: now,
            last_run: None,
            next_run: Some(next_run),
        };

        let mut loops = self.loops.write().await;
        loops.insert(id.clone(), config.clone());

        // Persist state
        let mut state = self.state.write().await;
        state.loops.push(config.into());
        state.save()?;

        Ok(id)
    }

    /// Stop a loop by ID
    pub async fn stop_loop(&mut self, id: &LoopId) -> Result<()> {
        let mut loops = self.loops.write().await;
        let mut config = loops.remove(id).ok_or(LoopError::NotFound(id.clone()))?;
        config.status = LoopStatus::Stopped;
        let _ = config; // Mark as used

        // Persist state
        let mut state = self.state.write().await;
        if let Some(entry) = state.loops.iter_mut().find(|l| l.id == id.0) {
            entry.status = "stopped".to_string();
        }
        state.save()?;

        Ok(())
    }

    /// Pause a loop
    pub async fn pause_loop(&mut self, id: &LoopId) -> Result<()> {
        let mut loops = self.loops.write().await;
        let config = loops.get_mut(id).ok_or(LoopError::NotFound(id.clone()))?;

        config.status = LoopStatus::Paused;

        // Persist state
        let mut state = self.state.write().await;
        if let Some(entry) = state.loops.iter_mut().find(|l| l.id == id.0) {
            entry.status = "paused".to_string();
        }
        state.save()?;

        Ok(())
    }

    /// Resume a paused loop
    pub async fn resume_loop(&mut self, id: &LoopId) -> Result<()> {
        let mut loops = self.loops.write().await;
        let config = loops.get_mut(id).ok_or(LoopError::NotFound(id.clone()))?;

        config.status = LoopStatus::Running;
        config.next_run = Some(Utc::now() + chrono::Duration::from_std(config.interval).unwrap_or_default());

        // Persist state
        let mut state = self.state.write().await;
        if let Some(entry) = state.loops.iter_mut().find(|l| l.id == id.0) {
            entry.status = "running".to_string();
        }
        state.save()?;

        Ok(())
    }

    /// List all loops
    pub async fn list_loops(&self) -> Vec<LoopConfig> {
        let loops = self.loops.read().await;
        loops.values().cloned().collect()
    }

    /// Get a specific loop
    pub async fn get_loop(&self, id: &LoopId) -> Option<LoopConfig> {
        let loops = self.loops.read().await;
        loops.get(id).cloned()
    }

    /// Stop all running loops
    pub async fn stop_all(&mut self) {
        let mut loops = self.loops.write().await;
        for (_, config) in loops.iter_mut() {
            config.status = LoopStatus::Stopped;
        }
        loops.clear();

        let mut state = self.state.write().await;
        state.loops.clear();
        let _ = state.save();
    }

    /// Run the scheduler - starts all loops in background
    pub async fn run(&self) {
        let loops = self.loops.clone();
        let executor = self.executor.clone();

        tokio::spawn(async move {
            loop {
                let loops_snapshot = {
                    let loops = loops.read().await;
                    loops
                        .values()
                        .filter(|c| c.status == LoopStatus::Running)
                        .cloned()
                        .collect::<Vec<_>>()
                };

                for config in loops_snapshot {
                    let executor = executor.clone();
                    let id = config.id.clone();
                    let interval = config.interval;
                    let conv_id = config.conversation_id.clone();
                    let prompt = config.prompt.clone();

                    tokio::spawn(async move {
                        let mut ticker = tokio_interval(interval);
                        loop {
                            ticker.tick().await;

                            // Execute the prompt
                            if let Err(e) = executor.execute(&id, &conv_id, &prompt).await {
                                tracing::error!("Loop {} execution failed: {}", id, e);
                            }
                        }
                    });
                }

                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        });
    }
}

impl From<crate::LoopEntry> for LoopConfig {
    fn from(entry: crate::LoopEntry) -> Self {
        let interval = Duration::from_secs(entry.interval_seconds);
        Self {
            id: LoopId(entry.id),
            conversation_id: entry.conversation_id,
            prompt: entry.prompt,
            interval,
            status: match entry.status.as_str() {
                "running" => LoopStatus::Running,
                "paused" => LoopStatus::Paused,
                _ => LoopStatus::Stopped,
            },
            created_at: entry.created_at,
            last_run: entry.last_run,
            next_run: entry.next_run,
        }
    }
}

impl From<LoopConfig> for crate::LoopEntry {
    fn from(config: LoopConfig) -> Self {
        Self {
            id: config.id.0,
            conversation_id: config.conversation_id,
            prompt: config.prompt,
            interval_seconds: config.interval.as_secs(),
            status: match config.status {
                LoopStatus::Running => "running".to_string(),
                LoopStatus::Paused => "paused".to_string(),
                LoopStatus::Stopped => "stopped".to_string(),
            },
            created_at: config.created_at,
            last_run: config.last_run,
            next_run: config.next_run,
        }
    }
}
