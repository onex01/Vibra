---
name: Vibra Kernel
description: Work on the Vibra x86_64 no_std Rust kernel — architecture, build, boot path, shell commands and safe change rules.
---

# Vibra Kernel

Vibra is an experimental monolithic `no_std` x86_64 kernel built for
`x86_64-unknown-none` and booted by Limine/UEFI. It is **not** an OS
distribution: userland, GUI and ARM64 are out of scope for 0.2.x.

## Layout

```text
kernel/src/lib.rs        entry, boot(), shell_loop(), panic handler
kernel/src/main.rs       thin _start
kernel/src/version.rs    single source of KERNEL_VERSION/CODENAME
kernel/src/fs/           VFS, RamFS, FAT32, ext2/3/4, NTFS, partitions
kernel/src/commands/     shell commands (top, tree, mount, vktst, ...)
kernel/src/shell/        line editor, history, tab completion
kernel/src/drivers/      AHCI (SATA+ATAPI), NVMe, xHCI, PCI, e1000
kernel/src/{memory,timer,task,interrupts,acpi}/
```

`vibra/` and `apps/` are legacy and are not part of the workspace build.

## Rules

- Never use `std`; only `core` + `alloc`.
- Do not print from interrupt context (`println!` takes spin locks). Use
  counters and print from the main path.
- Do not hold a spin lock across `hlt`, blocking disk I/O, or user input.
- Long commands must call `crate::poll_cancel()` and stop on `true`.
- New syscalls/ABI changes are out of scope unless explicitly requested.
- Filesystem write support: only FAT32 and only with explicit `rw`.
  ext2/3/4 and NTFS are read-only by design.
- Version changes only in `kernel/src/version.rs` and `kernel/Cargo.toml`.

## Build and test

```sh
./build.sh doctor
./build.sh setup
make check                  # warning budgets
./build.sh smoke            # UEFI boot in QEMU
./build.sh fs-test
./build.sh vktst-test
./build.sh debug-image-test
./build.sh iso-debug-test
```

`make build` installs `kernel.elf` into `build/hdd.img`. QEMU tests use
`build/hdd.img`, `OVMF.fd` and serial logs in `build/`.

## Where to read first

- `docs/architecture.md` — boot path and subsystems.
- `docs/testing.md` — acceptance tests.
- `docs/roadmap.md` — scope boundaries.
- `CHANGELOG.md` — current release notes.
