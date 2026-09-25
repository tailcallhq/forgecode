use std::fs::{self, File};
use std::hash::Hasher;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use forge_domain::Conversation;
use serde::{Deserialize, Serialize};

/// A prompt boundary whose conversation and file checkpoint share one turn ID.
#[derive(Clone, Serialize, Deserialize)]
pub struct HistoryPoint {
    /// Identifier shared by the file checkpoint and conversation boundary.
    pub turn: String,
    /// Prompt text displayed in the rewind selector.
    pub label: String,
    /// Capture time formatted as an RFC 3339 timestamp.
    pub timestamp: String,
    conversation: Conversation,
}

impl std::fmt::Display for HistoryPoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label: String = self
            .label
            .chars()
            .filter(|c| !c.is_control())
            .take(120)
            .collect();
        write!(f, "{}  {}", self.timestamp, label)
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Recovery {
    #[serde(default)]
    policy: String,
    point: HistoryPoint,
    points: Vec<HistoryPoint>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct State {
    points: Vec<HistoryPoint>,
    redo: Vec<Recovery>,
    pending: Option<Recovery>,
}
#[derive(Serialize, Deserialize)]
struct Active {
    session: String,
    turn: String,
}

/// Exclusive workspace lease held across the model run or a rewind transaction.
pub struct HistoryLease {
    _lock: File,
    active: PathBuf,
}
impl Drop for HistoryLease {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.active);
    }
}

/// Native conversation history backed by filesnap's three capture partitions.
pub struct ConversationHistory {
    base: PathBuf,
    cwd: PathBuf,
    session: String,
}
impl ConversationHistory {
    /// Construct a history in the existing host snapshot directory.
    ///
    /// # Arguments
    /// * `base` - Snapshot storage directory outside the workspace.
    /// * `cwd` - Workspace directory whose files are captured.
    /// * `session` - Host conversation UUID used to identify its history.
    pub fn new(base: PathBuf, cwd: PathBuf, session: String) -> Self {
        Self { base, cwd, session }
    }

    fn directory(base: &Path, cwd: &Path) -> Result<PathBuf> {
        let mut hash = fnv_rs::Fnv64::default();
        hash.write(cwd.canonicalize()?.to_string_lossy().as_bytes());
        let dir = base
            .join("conversations")
            .join(format!("{:x}", hash.finish()));
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }
    fn dir(&self) -> Result<PathBuf> {
        Self::directory(&self.base, &self.cwd)
    }
    fn record(&self) -> Result<PathBuf> {
        uuid::Uuid::parse_str(&self.session).context("Invalid conversation ID")?;
        Ok(self.dir()?.join(format!("{}.json", self.session)))
    }
    fn state(&self) -> Result<State> {
        match fs::read(self.record()?) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
            Err(e) => Err(e.into()),
        }
    }
    fn save(&self, state: &State) -> Result<()> {
        Self::write(&self.record()?, state)
    }
    fn write(path: &Path, value: &impl Serialize) -> Result<()> {
        use std::io::Write;
        let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&serde_json::to_vec(value)?)?;
        file.sync_all()?;
        fs::rename(temporary, path)?;
        Ok(())
    }
    fn store(&self) -> Result<filesnap::WorkspaceStore> {
        ensure!(
            !self
                .base
                .canonicalize()?
                .starts_with(self.cwd.canonicalize()?),
            "History storage must be outside the workspace"
        );
        Ok(filesnap::WorkspaceStore::open(&self.base, &self.cwd)?)
    }
    /// Refuse concurrent model turns or restores in the same workspace.
    /// Keep the returned lease alive until the turn or restore finishes.
    ///
    /// # Errors
    /// Returns an error if the workspace cannot be resolved, storage cannot
    /// be accessed, another operation holds the lock, or a stale active
    /// marker cannot be removed.
    pub fn acquire(&self) -> Result<HistoryLease> {
        let dir = self.dir()?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("workspace.lock"))?;
        lock.try_lock()
            .context("Another Forge turn or rewind is using this workspace")?;
        let active = dir.join("active.json");
        // Once the lock is acquired any previous marker belongs to an interrupted run.
        match fs::remove_file(&active) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        Ok(HistoryLease { _lock: lock, active })
    }
    /// Capture before a prompt and publish the edit-hook association.
    /// The caller must hold the workspace lease returned by `acquire`.
    ///
    /// # Arguments
    /// * `conversation` - Host conversation before dispatching the prompt.
    /// * `label` - Prompt text displayed in the rewind selector.
    ///
    /// # Errors
    /// Returns an error if recovery is pending, history or storage is invalid
    /// or inaccessible, capture fails or skips paths, or the history state
    /// and active marker cannot be persisted.
    pub fn begin(&self, conversation: Conversation, label: String) -> Result<()> {
        let mut state = self.state()?;
        ensure!(
            state.pending.is_none(),
            "Interrupted rewind. Run /rewind-recover before sending another prompt"
        );
        let point = HistoryPoint {
            turn: uuid::Uuid::new_v4().to_string(),
            label,
            timestamp: chrono::Utc::now().to_rfc3339(),
            conversation,
        };
        let store = self.store()?;
        let captured = filesnap::capture_turn(
            &store,
            &self.session,
            &point.turn,
            &filesnap::TurnScope::at(&self.cwd),
        )?;
        ensure!(
            captured.stats.dropped == 0,
            "Checkpoint skipped paths; cannot safely start this turn"
        );
        state.redo.clear();
        state.points.push(point.clone());
        self.save(&state)?;
        Self::write(
            &self.dir()?.join("active.json"),
            &Active { session: self.session.clone(), turn: point.turn },
        )
    }
    /// Attach local tool pre-images, including files that do not yet exist.
    /// Returns without capturing when no active turn marker exists.
    ///
    /// # Arguments
    /// * `base` - Snapshot storage directory used by the active history.
    /// * `cwd` - Workspace directory associated with the active turn.
    /// * `path` - Local file path resolved by the tool before mutation.
    ///
    /// # Errors
    /// Returns an error if history storage or the active marker cannot be
    /// read, the marker is invalid, file contents cannot be read, or the
    /// pre-image cannot be recorded in the snapshot store.
    pub fn declare(base: &Path, cwd: &Path, path: &Path) -> Result<()> {
        let active = match fs::read(Self::directory(base, cwd)?.join("active.json")) {
            Ok(bytes) => serde_json::from_slice::<Active>(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        let image = match fs::read(path) {
            Ok(bytes) => filesnap::PreEditImage::Existed(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                filesnap::PreEditImage::DidNotExist
            }
            Err(e) => return Err(e.into()),
        };
        let store = filesnap::WorkspaceStore::open(base, cwd)?;
        filesnap::declare_edits(
            &store,
            &active.session,
            &active.turn,
            &filesnap::TurnScope::at(cwd),
            vec![(path.into(), image)],
        )?;
        Ok(())
    }
    /// List current-branch prompts, newest first.
    ///
    /// # Errors
    /// Returns an error if the workspace or conversation ID is invalid, or
    /// the history state cannot be read or decoded.
    pub fn points(&self) -> Result<Vec<HistoryPoint>> {
        Ok(self.state()?.points.into_iter().rev().collect())
    }
    fn pinned_ignore(&self, policy: &str) -> Result<filesnap::Gitignore> {
        ensure!(policy.len() <= 1024 * 1024, "Ignore policy is too large");
        let mut builder = filesnap::GitignoreBuilder::new(&self.cwd);
        for line in policy.lines() {
            builder.add_line(None, line)?;
        }
        Ok(builder.build()?)
    }
    fn restore_files(&self, turn: &str, ignore: &filesnap::Gitignore) -> Result<()> {
        let store = self.store()?;
        let target = store
            .target_for_turn(turn)?
            .context("File checkpoint is missing")?;
        let paths =
            filesnap::restore_scope(&store, &self.session, &filesnap::TurnScope::at(&self.cwd))?;
        let result = store.restore_to(
            &self.session,
            &target,
            filesnap::RestoreKind::Rewind { undo_for: Some(&self.session) },
            paths,
            ignore,
        )?;
        ensure!(
            result.stats.failed.is_empty(),
            "Could not restore files: {:?}",
            result.stats.failed
        );
        Ok(())
    }
    /// Restore files first, keeping a durable journal until the host commits
    /// the conversation. Returns the destination conversation and an opaque
    /// state token for `commit`.
    /// The caller must hold the workspace lease returned by `acquire`.
    ///
    /// # Arguments
    /// * `target` - Current-branch turn ID to rewind to, or `None` to redo.
    /// * `current` - Host conversation saved with the recovery checkpoint.
    ///
    /// # Errors
    /// Returns an error if history or ignore policy cannot be loaded,
    /// recovery is pending, the target or redo entry is unavailable, a
    /// recovery checkpoint or journal cannot be saved, or file restoration
    /// fails. A failed rollback retains the journal for `recover`.
    pub fn restore(
        &self,
        target: Option<&str>,
        current: Conversation,
    ) -> Result<(Conversation, String)> {
        let policy = match fs::read_to_string(self.cwd.join(".filesnapignore")) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error.into()),
        };
        let ignore = self.pinned_ignore(&policy)?;
        let mut state = self.state()?;
        ensure!(
            state.pending.is_none(),
            "Interrupted rewind. Run /rewind-recover first"
        );
        let (turns, destination, points) = if let Some(turn) = target {
            let index = state
                .points
                .iter()
                .position(|p| p.turn == turn)
                .context("Checkpoint is not on the current branch")?;
            let (retained, rewound) = state
                .points
                .split_at_checked(index)
                .context("Invalid checkpoint position")?;
            let destination = rewound
                .first()
                .context("Checkpoint is not on the current branch")?
                .conversation
                .clone();
            (
                rewound
                    .iter()
                    .rev()
                    .map(|p| p.turn.clone())
                    .collect::<Vec<_>>(),
                destination,
                retained.to_vec(),
            )
        } else {
            let redo = state
                .redo
                .last()
                .context("Nothing to redo; a new prompt clears redo")?;
            (
                vec![redo.point.turn.clone()],
                redo.point.conversation.clone(),
                redo.points.clone(),
            )
        };
        let turn = uuid::Uuid::new_v4().to_string();
        let store = self.store()?;
        let paths =
            filesnap::restore_scope(&store, &self.session, &filesnap::TurnScope::at(&self.cwd))?;
        let captured = store.checkpoint(&self.session, &turn, paths)?;
        ensure!(
            captured.stats.dropped == 0,
            "Could not save recovery files; rewind cancelled"
        );
        let recovery = Recovery {
            policy,
            point: HistoryPoint {
                turn,
                conversation: current,
                label: "Before rewind".into(),
                timestamp: chrono::Utc::now().to_rfc3339(),
            },
            points: state.points.clone(),
        };
        state.pending = Some(recovery.clone());
        self.save(&state)?;
        for turn in turns {
            if let Err(error) = self.restore_files(&turn, &ignore) {
                self.restore_files(&recovery.point.turn, &ignore)
                    .context(format!(
                        "Rewind failed ({error}); /rewind-recover is required"
                    ))?;
                state.pending = None;
                self.save(&state)?;
                return Err(error.context("Original files recovered; conversation unchanged"));
            }
        }
        state.pending = None;
        state.points = points;
        if target.is_some() {
            state.redo.push(recovery);
        } else {
            state.redo.pop();
        }
        Ok((destination, serde_json::to_string(&state)?))
    }
    /// Finish after the conversation has been persisted successfully.
    /// The caller must keep the workspace lease until this operation ends.
    ///
    /// # Arguments
    /// * `token` - Unmodified state token returned by `restore` or `recover`.
    ///
    /// # Errors
    /// Returns an error if the token is invalid or the history state cannot
    /// be persisted.
    pub fn commit(&self, token: &str) -> Result<()> {
        self.save(&serde_json::from_str::<State>(token)?)
    }
    /// Restore the journal's files; the host must persist the returned
    /// conversation and commit.
    /// The caller must hold the workspace lease returned by `acquire`.
    ///
    /// # Errors
    /// Returns an error if history cannot be loaded, no recovery is pending,
    /// the saved ignore policy is invalid, or file restoration or state
    /// serialization fails. The persisted journal remains until `commit`.
    pub fn recover(&self) -> Result<(Conversation, String)> {
        let mut state = self.state()?;
        let recovery = state
            .pending
            .take()
            .context("No interrupted rewind to recover")?;
        self.restore_files(&recovery.point.turn, &self.pinned_ignore(&recovery.policy)?)?;
        Ok((recovery.point.conversation, serde_json::to_string(&state)?))
    }
}

#[cfg(test)]
mod tests {
    use forge_domain::ConversationId;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::*;
    fn fixture() -> (TempDir, ConversationHistory, Conversation) {
        let root = TempDir::new().unwrap();
        let cwd = root.path().join("workspace");
        fs::create_dir(&cwd).unwrap();
        let id = ConversationId::generate();
        let history = ConversationHistory::new(root.path().join("store"), cwd, id.to_string());
        (root, history, Conversation::new(id))
    }
    #[test]
    fn rewinds_binary_files_and_conversation_and_supports_nested_redo() {
        let (_root, history, original) = fixture();
        let lease = history.acquire().unwrap();
        let asset = history.cwd.join("asset.bin");
        fs::write(&asset, [0, 255, 7]).unwrap();
        history.begin(original.clone(), "first".into()).unwrap();
        fs::write(&asset, "first result").unwrap();
        let mut second = original.clone();
        second.title = Some("second".into());
        history.begin(second.clone(), "second".into()).unwrap();
        let created = history.cwd.join(".created");
        ConversationHistory::declare(&history.base, &history.cwd, &created).unwrap();
        fs::write(&created, "second result").unwrap();
        let mut current = original.clone();
        current.title = Some("current".into());
        let points = history.points().unwrap();
        let (restored, token) = history
            .restore(Some(&points[0].turn), current.clone())
            .unwrap();
        history.commit(&token).unwrap();
        assert_eq!(restored.title, second.title);
        assert!(!created.exists());
        let (restored, token) = history.restore(Some(&points[1].turn), restored).unwrap();
        history.commit(&token).unwrap();
        assert_eq!(restored.title, original.title);
        assert_eq!(fs::read(&asset).unwrap(), vec![0, 255, 7]);
        let (restored, token) = history.restore(None, restored).unwrap();
        history.commit(&token).unwrap();
        assert_eq!(restored.title, second.title);
        let (restored, token) = history.restore(None, restored).unwrap();
        history.commit(&token).unwrap();
        assert_eq!(restored.title, current.title);
        assert_eq!(fs::read_to_string(&created).unwrap(), "second result");
        drop(lease);
    }
    #[test]
    fn interrupted_conversation_commit_blocks_prompts_and_can_recover() {
        let (_root, history, original) = fixture();
        let _lease = history.acquire().unwrap();
        let asset = history.cwd.join("a");
        fs::write(&asset, "before").unwrap();
        history.begin(original.clone(), "edit".into()).unwrap();
        fs::write(&asset, "after").unwrap();
        let turn = history.points().unwrap()[0].turn.clone();
        let mut current = original.clone();
        current.title = Some("current".into());
        let _uncommitted = history.restore(Some(&turn), current.clone()).unwrap();
        assert!(history.begin(original, "must be blocked".into()).is_err());
        let (restored, token) = history.recover().unwrap();
        history.commit(&token).unwrap();
        assert_eq!(restored.title, current.title);
        assert_eq!(fs::read_to_string(asset).unwrap(), "after");
    }
}
