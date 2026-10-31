---
name: Vibra Release
description: Prepare a Vibra release — version bump, changelog, artifact naming, tags, GitHub Release workflow and verification.
---

# Vibra Release

## Version sources

- `kernel/src/version.rs` — `KERNEL_VERSION`, `KERNEL_CODENAME` (single source).
- `kernel/Cargo.toml` — must match.
- `kernel/build.rs` — injects `BUILD_DATE`, `GIT_COMMIT`, `BUILD_PROFILE`.
- `Makefile` reads `KERNEL_VERSION` from `version.rs` for artifact names.

Bump all of them together, plus badges in `README.md`, `docs/README.md`,
`docs/architecture.md`, `docs/roadmap.md`, `docs/usb_and_boot.md`.

## Artifacts

```text
vibra_<DATE>_<VERSION>.iso
vibra-release_<DATE>_<VERSION>.iso
vibra-debug_<DATE>_<VERSION>.iso
vibra-uefi_<DATE>_<VERSION>.img
vibra-usb_<DATE>_<VERSION>.img
vibra-usb-debug_<DATE>_<VERSION>.img
```

Date is UTC `YYYY-MM-DD`. Stable symlinks without a date are created for
scripts. `SOURCE_DATE_EPOCH` pins the date for reproducible builds.

## Checklist

```sh
# 1. version + changelog + docs
vi kernel/src/version.rs kernel/Cargo.toml
vi CHANGELOG.md
vi README.md docs/README.md docs/architecture.md docs/roadmap.md

# 2. verify
./build.sh doctor
./build.sh setup
make check
./build.sh smoke
./build.sh fs-test
./build.sh vktst-test
./build.sh debug-image-test
./build.sh iso-debug-test
./build.sh iso-release

# 3. commit and tag
git add -A
git commit -m "release: <VERSION>"
git tag v<VERSION>
git push origin main --tags
```

## GitHub Actions

`.github/workflows/release.yml` triggers on tag `v*` or manual dispatch with
a `tag` input. It builds release/debug ISO, UEFI/USB images, tests debug
images in QEMU, writes `dist/SHA256SUMS`, and publishes a GitHub Release
with generated notes. `permissions: contents: write` is required.

Do not tag a release when tests are red: push commits if needed, but leave
the tag for a green run.

## Scope guard

Do not claim hardware support based on QEMU. Note ext2/3/4 and NTFS as
read-only, and SMP as not enabled, in the release notes.
