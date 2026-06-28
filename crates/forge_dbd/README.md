# forge_dbd — WIP

> **Status: Work In Progress — not yet wired into the main application.**

SQLite daemon crate for persistent conversation storage. Intended as a background
IPC daemon that serialises conversation history to a local SQLite database.

## Current state

- Protocol types defined (`protocol.rs`)
- Stub server + client skeletons (`server.rs`, `client.rs`)
- Binary entry point exists (`main.rs`)
- **Not depended upon by any other workspace crate**
- **Not included in the shipped binary**

## Planned integration

Part of the SQLite-WAL/FTS epic. Will be wired into `forge_app` once the IPC
contract is finalised. Do not ship or enable without completing that epic.
