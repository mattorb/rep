# Tested Linux release support

## Goal

Re-enable Linux release archives only after the project has an end-to-end
smoke test proving that a published Linux archive can be installed and run.

The target outcome is:

- GitHub releases publish Linux archives again.
- The installer supports Linux instead of failing early.
- A Linux release archive is tested as an archive, not just as a build output.
- Local Apple Silicon Macs can smoke test the Linux archive through Apple's
  `container` runtime.
- README platform claims match CI, installer behavior, and release artifacts.

## Current state

Linux release targets are intentionally paused in `.github/workflows/release.yml`:

```yaml
# Linux release artifacts are paused until release/install smoke tests
# cover the published archives.
# - target: x86_64-unknown-linux-musl
#   os: ubuntu-latest
#   use_cross: true
# - target: aarch64-unknown-linux-musl
#   os: ubuntu-latest
#   use_cross: true
```

`install.sh` also hard-fails on Linux:

```sh
Linux)
  fail "Linux release archives are not published yet. Build from source with: cargo install --git https://github.com/${REPO}. You can also clone the repository and run ./build.sh."
  ;;
```

The README says Linux release archives are not published yet, and the platform
support table only lists macOS targets. That is accurate today and should stay
accurate until the smoke tests exist.

Local environment note from July 2026: this Mac is Apple Silicon on macOS 26,
which satisfies Apple's documented `container` requirements, but the `container`
CLI is not currently installed here. Docker and Podman are also not installed.

## Support shape

Start with these release targets:

| Platform | Target | Test priority | Notes |
| --- | --- | --- | --- |
| Linux arm64 | `aarch64-unknown-linux-musl` | First | Native architecture for Apple Silicon container smoke tests. |
| Linux x86_64 | `x86_64-unknown-linux-musl` | Second | Useful for common Linux hosts; local Apple-container testing may require Rosetta or explicit `linux/amd64` support. |

Both targets are static MUSL builds, so the smoke test should not depend on a
particular distro's libc.

## What the smoke test must prove

The release smoke test should verify the exact user-facing artifact contract:

1. The archive exists with the expected name:
   `rep-${VERSION}-${TARGET}.tar.gz`.
2. `checksums.txt` contains an entry for that archive.
3. The archive SHA-256 matches `checksums.txt`.
4. The archive extracts cleanly.
5. The extracted binary is executable.
6. `rep --help` exits successfully.
7. The bundled `.agents/skills/rep` directory exists in the archive.
8. The installer can install the archive to an isolated `REP_INSTALL_DIR`.
9. The installer can install the bundled skill to an isolated `REP_SKILLS_DIR`.
10. The installed `rep --help` exits successfully.

This deliberately tests packaging and installer behavior. `cargo build
--target ...` alone is not enough.

## CI plan

### 1. Add a release smoke script

Create a script such as `scripts/smoke-release-archive.sh` with this interface:

```sh
scripts/smoke-release-archive.sh \
  --archive path/to/rep-v0.3.1-aarch64-unknown-linux-musl.tar.gz \
  --checksums path/to/checksums.txt \
  --target aarch64-unknown-linux-musl \
  --version v0.3.1
```

Responsibilities:

- Validate the archive name.
- Validate checksum presence and hash.
- Extract into a temp directory.
- Assert `rep`, `LICENSE`, `README.md`, and `.agents/skills/rep/SKILL.md`
  exist.
- Run `./rep --help`.
- Optionally run a non-interactive CLI path if one exists or is added later.

Keep the script POSIX shell unless Bash materially simplifies the test. The
installer is POSIX shell, so a POSIX smoke script is a useful compatibility
check.

### 2. Test the installer without touching the real home directory

The smoke script should support an installer mode that runs `install.sh` with
isolated paths:

```sh
REP_INSTALL_DIR="$tmp/install/bin" \
REP_SKILLS_DIR="$tmp/install/skills" \
REP_VERSION="$version" \
REP_INSTALL_REPO="$repo" \
./install.sh
```

For local archive smoke tests, avoid relying on GitHub release URLs. Prefer one
of these approaches:

- Add installer overrides for `REP_ARCHIVE_URL` and `REP_CHECKSUMS_URL`.
- Or add `REP_RELEASE_BASE_URL` so tests can point at a local file server.

Recommended: add `REP_RELEASE_BASE_URL`. That preserves the current URL
construction model while making local and CI smoke tests straightforward.

Example:

```sh
base_url="${REP_RELEASE_BASE_URL:-https://github.com/${REPO}/releases/download/${VERSION}}"
archive_url="${base_url}/${archive_name}"
checksums_url="${base_url}/checksums.txt"
```

Then CI can serve the packaged archive and checksums from a temp directory with
Python's built-in HTTP server.

### 3. Add smoke testing to the release workflow

In `.github/workflows/release.yml`, run the smoke script immediately after the
`Package` step and before attestation/upload:

```yaml
- name: Smoke release archive
  if: runner.os == 'Linux'
  run: |
    scripts/smoke-release-archive.sh \
      --archive "${{ steps.package.outputs.archive }}" \
      --checksums "checksums-${{ matrix.target }}.txt" \
      --target "${{ matrix.target }}" \
      --version "${{ github.ref_name }}"
```

The current workflow creates `checksums-${{ matrix.target }}.txt` after upload.
Move checksum file creation before the smoke step, or have the package step
write both the archive and a per-target checksum file.

Run the smoke step for Linux targets first. It can also run for macOS later, but
Linux support should not be blocked on expanding macOS coverage.

### 4. Re-enable Linux targets

After the smoke test exists, uncomment the Linux matrix entries:

```yaml
- target: x86_64-unknown-linux-musl
  os: ubuntu-latest
  use_cross: true
- target: aarch64-unknown-linux-musl
  os: ubuntu-latest
  use_cross: true
```

Keep `cross` for both targets unless direct `cargo build` is proven simpler and
equally reproducible in GitHub-hosted Ubuntu.

## Installer changes

### 1. Detect Linux targets

Change `detect_target` so Linux maps to MUSL targets instead of failing:

```sh
Linux)
  os="unknown-linux-musl"
  ;;
Darwin)
  os="apple-darwin"
  ;;
```

Then map architecture:

```sh
case "$uname_m" in
  x86_64|amd64)
    arch="x86_64"
    ;;
  aarch64|arm64)
    arch="aarch64"
    ;;
  *)
    fail "Unsupported architecture: ${uname_m} (supported: x86_64, aarch64)."
    ;;
esac

TARGET="${arch}-${os}"
```

### 2. Keep unsupported OS messaging precise

Update unsupported OS errors to say release installer OS support is macOS and
Linux, not macOS only.

### 3. Avoid hidden macOS assumptions

The existing installer already uses portable commands where possible:

- `curl` or `wget`
- `sha256sum`, `shasum`, or `openssl`
- `install` or `cp && chmod`
- `tar -xzf`

The smoke test should run in a minimal Linux image to catch missing assumptions.

## Apple container local smoke test

Apple's `container` CLI can run Linux containers as lightweight VMs on Apple
Silicon Macs. Once installed and started, use it to test the Linux archive from
this macOS workspace.

### 1. Prerequisites

Install Apple's `container` CLI from the official Apple GitHub release package,
then start the service:

```sh
container system start
```

Verify:

```sh
container --version
container run --rm alpine:latest uname -a
```

### 2. Native arm64 Linux archive smoke test

After a local or CI build produces `rep-vX.Y.Z-aarch64-unknown-linux-musl.tar.gz`
and `checksums.txt`, run:

```sh
container run --rm \
  --platform linux/arm64 \
  --volume "$PWD:/work" \
  --workdir /work \
  alpine:latest \
  sh -lc '
    apk add --no-cache ca-certificates tar coreutils &&
    scripts/smoke-release-archive.sh \
      --archive rep-vX.Y.Z-aarch64-unknown-linux-musl.tar.gz \
      --checksums checksums.txt \
      --target aarch64-unknown-linux-musl \
      --version vX.Y.Z
  '
```

This validates the native Linux artifact on the same architecture as the Mac.

### 3. x86_64 Linux archive smoke test

If Apple `container` supports the needed emulation path on the installed
version, test the x86_64 archive with:

```sh
container run --rm \
  --platform linux/amd64 \
  --rosetta \
  --volume "$PWD:/work" \
  --workdir /work \
  alpine:latest \
  sh -lc '
    apk add --no-cache ca-certificates tar coreutils &&
    scripts/smoke-release-archive.sh \
      --archive rep-vX.Y.Z-x86_64-unknown-linux-musl.tar.gz \
      --checksums checksums.txt \
      --target x86_64-unknown-linux-musl \
      --version vX.Y.Z
  '
```

If this does not work reliably, keep x86_64 validation in GitHub-hosted Ubuntu
CI and document that local Apple-container smoke testing is arm64-first.

## Documentation changes

Update README only after the Linux smoke test and installer changes are merged:

- Replace "Linux release archives are not published yet" with the normal
  installer path.
- Add Linux rows to the platform support table.
- State that Linux archives are MUSL static builds.
- Keep source install instructions as a fallback, not the primary Linux path.

Suggested platform rows:

| Platform | Release artifact | CI coverage | Support status |
| --- | --- | --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-musl` | Build, package, archive smoke test, installer smoke test on GitHub-hosted Ubuntu | Supported |
| Linux arm64 | `aarch64-unknown-linux-musl` | Cross build, package, archive smoke test, installer smoke test on GitHub-hosted Ubuntu; local Apple-container smoke test when available | Supported |

Keep release support claims aligned with what CI actually does.

## Validation before submitting changes

Run the standard project gate:

```sh
./build.sh
```

Because `mise.toml` exists, Rust commands should run through `mise exec --`
when invoked directly. `build.sh` already does this when `mise` is available.

Also run the new smoke script locally against at least one generated archive.
If Apple `container` is installed, run the native arm64 Linux container smoke
test before merging Linux support.

## Rollout order

1. Add `scripts/smoke-release-archive.sh`.
2. Add installer URL override support needed by the smoke test.
3. Update `install.sh` Linux target detection.
4. Wire archive smoke testing into `.github/workflows/release.yml`.
5. Re-enable `aarch64-unknown-linux-musl`.
6. Re-enable `x86_64-unknown-linux-musl`.
7. Update README platform and install documentation.
8. Install Apple `container` locally and run the arm64 Linux archive smoke test.
9. Try the x86_64 Apple-container smoke test; keep CI as the source of truth if
   local emulation is not reliable.

## Open questions

- Should the release workflow smoke test macOS archives too, for symmetry?
- Should `install.sh` support local `file://` URLs, or is a temporary HTTP
  server enough for tests?
- Do we want a non-interactive `rep` command beyond `--help` that can validate
  basic markdown parsing without launching the TUI?
- Is Apple-container x86_64/Rosetta testing reliable enough to document, or
  should local smoke testing stay arm64-only?
