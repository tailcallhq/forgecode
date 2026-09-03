# SLSA Build Attestation

This repository publishes build provenance for release artifacts in
accordance with [SLSA (Supply-chain Levels for Software Artifacts)][slsa]
Build specifications. SLSA provenance allows downstream consumers to
verify that an artifact was built from the expected source repository,
at the expected commit, by the expected build platform.

## Target Level

**Current target: SLSA Build L2 (achieved today)**

The release pipeline is hosted on GitHub Actions, an isolated build
platform that is owned and administered by GitHub. Provenance is
generated automatically for every published release using
[`slsa-framework/slsa-github-generator`][slsa-gh-gen] and the
`attest-build-provenance` action. Provenance is signed by a GitHub-
hosted OIDC token and stored in the [GitHub Artifact Attestations][ghaa]
log alongside the artifact.

| Requirement                                 | Status       |
| ------------------------------------------- | ------------ |
| Provenance generated automatically          | ✅ L2        |
| Provenance distributed alongside artifact   | ✅ L2        |
| Build platform hosted and isolated          | ✅ L2        |
| Provenance authenticity (OIDC-signed)       | ✅ L2        |
| Build platform isolated from build request | ⏭ L3 target |
| Hardened build platform                     | ⏭ L3 target |
| Provenance non-forgeable (sigstore/cosign)  | ⏭ L3 target |

## Workflow

The CI workflow lives at
[`.github/workflows/release-attestation.yml`](../.github/workflows/release-attestation.yml)
and is triggered:

- Automatically on every `release: published` event.
- Manually via `workflow_dispatch` for ad-hoc provenance generation.

### Build Steps

1. **Checkout** — full history (`fetch-depth: 0`) so the git revision
   can be embedded in provenance.
2. **Toolchain** — pinned `stable` Rust via
   [`dtolnay/rust-toolchain`][rust-toolchain].
3. **Cache** — cargo registry, git index, and `target/` via
   [`Swatinem/rust-cache`][rust-cache].
4. **Build** — `cargo build --release --locked --workspace --all-targets`.
5. **Stage** — collect built executables, source tarball, and a build
   manifest into `release-artifacts/`.
6. **Upload** — publish `release-artifacts` as a GitHub Actions artifact
   (90 day retention).
7. **Attest** — generate SLSA Build L2 provenance with
   `slsa-framework/slsa-github-generator/attest-build-provenance@v1`.

### CycloneDX SBOM regeneration policy

The repository checks in one CycloneDX JSON SBOM for every workspace package.
Regenerate the full set only through the version-pinned helper:

```bash
SOURCE_DATE_EPOCH="$(git log -1 --format=%ct)" ./scripts/generate-sbom.sh
```

The helper requires `cargo-cyclonedx` **0.5.9**, requires a numeric
`SOURCE_DATE_EPOCH`, and runs `cargo cyclonedx --format json --all`. The epoch
prevents wall-clock time from becoming an untracked generation input. The
resulting `*.cdx.json` files are reviewed with the dependency update that
changed them. This policy does not itself attach an SBOM to a release or alter
the provenance workflow; release attachment remains a separate workflow change.

## Verification

Consumers can verify a release artifact's provenance using the
[GitHub CLI][gh-cli]:

```bash
gh attestation verify <artifact> --owner <org>
```

Or with [`cosign`][cosign]:

```bash
cosign verify-attestation \
  --certificate-identity-regexp 'https://github.com/slsa-framework/slsa-github-generator' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  <artifact>
```

The in-toto provenance attestation (`slsa-github-generator/actions/attest-build-provenance`)
contains:

- `builder.id` — `https://github.com/actions/runner`
- `invocation.config.source.uri` — repository URL
- `invocation.config.source.entryPoint` — build workflow path
- `invocation.config.source.digest.sha1` — git commit SHA
- `invocation.config.source.ref` — git ref (tag / branch)
- `metadata.buildInvocationID` — workflow run ID
- `metadata.completeness.parameters` — whether all inputs are hashed
- `metadata.completeness.environment` — whether environment is fully captured

## Path to SLSA Build L3

The current pipeline satisfies L2. To graduate to L3, the following
additions are required:

1. **Isolated build environment** — move from a hosted runner to
   ephemeral, single-tenant builders (e.g.
   `slsa-framework/slsa-github-generator`'s `generator_containerized_slsa3.yml`
   reusable workflow, or a self-hosted runner with a hardened image).
2. **Provenance non-forgeability** — the generator workflow re-signs
   provenance with a build-platform-held signing key (sigstore / KMS)
   rather than relying on the GitHub OIDC token alone.
3. **Provenance transparency log** — the generator publishes
   provenance to a transparency log (e.g. Rekor) so forgery is
   detectable by the wider community.

To upgrade, switch the `attest-build-provenance` step to invoke the
`slsa-framework/slsa-github-generator/.github/workflows/generator_containerized_slsa3.yml@v2`
reusable workflow with a build image pinned by digest. The reusable
workflow handles ephemeral runners, hardened isolation, and
non-forgeable provenance signing transparently.

## References

- [SLSA Framework][slsa]
- [`slsa-framework/slsa-github-generator`][slsa-gh-gen]
- [GitHub Artifact Attestations][ghaa]
- [GitHub Actions security hardening][ghas]
- [`dtolnay/rust-toolchain`][rust-toolchain]
- [`Swatinem/rust-cache`][rust-cache]
- [`cosign`][cosign]

[slsa]: https://slsa.dev
[slsa-gh-gen]: https://github.com/slsa-framework/slsa-github-generator
[ghaa]: https://docs.github.com/en/security/supply-chain-security/artifact-attestations
[ghas]: https://docs.github.com/en/actions/security-guides/security-hardening-for-github-actions
[gh-cli]: https://cli.github.com
[cosign]: https://github.com/sigstore/cosign
[rust-toolchain]: https://github.com/dtolnay/rust-toolchain
[rust-cache]: https://github.com/Swatinem/rust-cache
