# Benchmark Runner

This directory contains the benchmark setup for running Forge against [Harbor](https://harborframework.com) terminal benchmarks.

## Table of Contents

- [Benchmark Runner](#benchmark-runner)
  - [Table of Contents](#table-of-contents)
  - [Prerequisites](#prerequisites)
    - [Rust and Cargo](#rust-and-cargo)
    - [Rust MUSL Target](#rust-musl-target)
    - [cross (macOS only)](#cross-macos-only)
  - [Clone the Project](#clone-the-project)
  - [Compilation](#compilation)
  - [Configure Forge Credentials](#configure-forge-credentials)
  - [Set Binary Path](#set-binary-path)
  - [Install Harbor](#install-harbor)
  - [Run Benchmarks](#run-benchmarks)
  - [How forge\_agent.py Works](#how-forge_agentpy-works)

---

## Prerequisites

### Rust and Cargo

Install Rust and Cargo with `rustup`:

```shell
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Source Cargo's environment file so `cargo` is available in the current shell:

```shell
. "$HOME/.cargo/env"
```

### Rust MUSL Target

Add the MUSL target required by the benchmark build:

```shell
rustup target add x86_64-unknown-linux-musl
```

### cross (macOS only)

On macOS, install `cross` to cross-compile for Linux MUSL:

```shell
cargo install cross --git https://github.com/cross-rs/cross
```

---

## Clone the Project

```shell
git clone https://github.com/tailcallhq/forgecode-beta.git
cd forgecode-beta
```

---

## Compilation

The benchmarks require a `x86_64-unknown-linux-musl` binary. Use the appropriate command for your host OS.

**On Linux:**

```shell
cargo build --release --target x86_64-unknown-linux-musl
```

**On macOS:**

```shell
cross build --release --target x86_64-unknown-linux-musl
```

> Note: only the x86_64 Linux MUSL build works for all benchmarks. Some benchmark
> images do not have a compatible GLIBC version for the normal Linux binary.

---

## Configure Forge Credentials

A [`.forge.toml`](.forge.toml) tuned for autonomous, non-interactive operation is included in this directory. Copy it to your Forge config directory before running benchmarks:

```shell
cp .forge.toml ~/.forge/.forge.toml
```

Log in to the provider and select the model, then log in to Forge services:

```shell
forge provider login openai
forge provider login forge_services
```

The benchmark agent copies the current Forge config into each benchmark container. By default it uses `~/.forge`, falling back to `~/forge`; set `FORGE_CONFIG` to use a different config directory. The copied files include `.forge.toml`, `.credentials.json`, and `.mcp.json` when present.

---

## Set Binary Path

Point `$FORGE_BIN` at the MUSL binary produced by the compilation step. Replace `<path_to_repo>` with the absolute path to your local checkout:

```shell
export FORGE_BIN=<path_to_repo>/target/x86_64-unknown-linux-musl/release/forge
```

For example, if you cloned into `/home/user/forgecode-beta`:

```shell
export FORGE_BIN=/home/user/forgecode-beta/target/x86_64-unknown-linux-musl/release/forge
```

---

## Install Harbor

Refer: https://harborframework.com/docs/getting-started

---

## Run Benchmarks

```shell
harbor run -d terminal-bench@2.0 --agent-import-path bench.forge_agent:ForgeAgent --export-traces --export-verifier-metadata --force-build --max-retries 5 --debug -n 32
```

> Note: `-n 32` runs 32 tests in parallel, which works well on M* Pro chips.

---

## How forge_agent.py Works

In [forge_agent.py](forge_agent.py), since there is no way to provide API credentials interactively inside each benchmark container, it copies the current Forge configuration and credentials files from the host config directory into `/root/.forge` in the container.
