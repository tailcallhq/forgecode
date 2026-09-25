# Native file and conversation history

## Rewind in the terminal

Checkpoints are saved automatically before new prompts. In the chat input, type
`/rewind`, select a prompt with the arrow keys, press Enter, and confirm. Both
workspace files and the conversation return to before that prompt. `/redo`
restores both to before the last rewind; repeated redo walks successive rewinds.
A new prompt clears redo. Esc cancels the selector.

Wait for the agent to finish before restoring. If a restore is interrupted, run
`/rewind-recover` before continuing. Recovery is journaled before file writes;
conversation changes follow successful file restoration. New combined history
starts with prompts captured by this build, not legacy file-only checkpoints.

Capture is bounded. Ignored, remote, or uncaptured files are not protected.
Known local write/edit paths are declared before mutation, including absent files;
binary files are restored as bytes. Shell/MCP writes need pre-existing capture
coverage. These commands do not call a model.

`ConversationHistory` holds a workspace lease across a turn and snapshots the
serialized host conversation before dispatch. Local write/patch/remove operations
attach pre-images through SnapshotRepository. A recovery journal blocks new turns
until both filesystem restoration and conversation persistence have committed.

## Per-file compatibility API


Forge uses `filesnap` as an in-process content-addressed storage engine. The
existing per-file undo API is unchanged. New `.filesnap` references point to
snapshots in the shared store; old raw `.snap` files remain readable, and both
formats are consumed newest first. Failed restores keep their reference for retry.

Each capture contains exactly the requested path, including its absence and
permissions. Undo cannot delete a neighboring untracked file. The backend does
not enable workspace-wide scanning or conversation rewind. File selection remains
Forge's responsibility, so it uses the low-level filesnap checkpoint interface,
not its three-bucket automatic workspace scanner or `.filesnapignore` policy.

Before restoring, filesnap retains a safety point under that checkpoint's unique
engine session. Failed restores keep both the checkpoint and its safety history.
Successful undo retires the host marker, deletes that engine session, and runs
whole-store garbage collection; content still referenced by other checkpoints is
preserved. Retired markers make interrupted cleanup retryable on a later capture
or undo of the same file. Recently written blobs remain subject to filesnap's GC
grace period. Existing histories require no eager conversion.
