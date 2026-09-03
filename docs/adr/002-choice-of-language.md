# ADR 002: Choice of Languages

## Status
Accepted

## Context
The project requires a robust set of tools and services. We need to choose languages that balance performance, developer productivity, and ecosystem support.

## Decision
We will use a polyglot approach: **Rust** for core logic, **Python** for scripting and orchestration, and **TypeScript** for web interfaces.

- **Rust**: Chosen for the core engine due to its memory safety, high performance, and excellent concurrency model. It's ideal for the critical path and low-level operations.
- **Python**: Chosen for its vast ecosystem of libraries and ease of use in data processing, automation scripts, and glue code. It allows for rapid prototyping and iteration.
- **TypeScript**: Chosen for the frontend and any web-based tooling, leveraging JavaScript's ubiquity and adding static typing for better maintainability.

## Consequences
- **Pros**:
    - Each part of the system uses the most appropriate tool for the job.
    - High performance where it matters (Rust).
    - High development velocity for scripts and web UIs.
- **Cons**:
    - Increased complexity in the build system and CI/CD pipeline.
    - Team members need to be proficient in multiple languages.
    - Integration points between languages require careful design (FFI, IPC, etc.).
