# Forgecode merge train

Goal: make the KooshaPari forgecode fork evaluable from `main` with open PRs merged and quality gates green.

Merged lanes:

- #97 Windows release forge3d repair.
- #99 Windows forge daemon Unix-socket cfg repair.
- #100 Windows winterminal release compile repair.
- #101 helioslite version/branding repair.
- #102 follow-up test-target compile repair for #101.
- #94 provider error resilience and provider removal.

Current completion gate: latest `main` commit must have GitHub Actions CI, lint, test, dependency, and secret-scan evidence on the latest head SHA.
