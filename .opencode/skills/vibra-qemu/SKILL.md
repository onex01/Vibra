---
name: Vibra QEMU Testing
description: Run and add Vibra QEMU tests — serial automation, timeouts, boot matrix, boot log and CI parity.
---

# Vibra QEMU Testing

## Golden rules

- QEMU tests are the regression gate; real hardware is the acceptance gate.
- CI runners have no KVM, so QEMU runs on TCG and is slow. Always allow
  generous timeouts through env vars (`*_TIMEOUT`, `BOOT_READY_TIMEOUT`).
- Never rely on `-serial stdio` piped through a FIFO on CI; drive the shell
  through the QEMU monitor with `sendkey` (see existing scripts).
- Delete the serial log before booting. A leftover prompt makes the
  readiness loop fire immediately and commands go to a machine that is not
  booted yet.
- Copy images before tests that write (`cp --sparse=always`), so release
  artifacts stay pristine.

## Script template

```sh
log="$repo_root/build/qemu-foo.log"
rm -f "$log"

qemu_key() {
    case "$1" in
        ' ') echo spc;; $'\n') echo ret;; '/') echo slash;;
        '.') echo dot;; '-') echo minus;;
        [A-Z]) printf 'shift-%s\n' "$(printf '%s' "$1" | tr 'A-Z' 'a-z')";;
        *) echo "$1";;
    esac
}
send_text() { for ((i=0;i<${#1};i++)); do echo "sendkey $(qemu_key "${1:i:1}")"; sleep 0.05; done; }
```

Then assert markers with `grep -Fq`, write a `build/<name>-status.txt`, and
dump the tail of the serial log on failure.

## Commands

```sh
make smoke clock-test ramfs-test ro-mount-test bad-fs-test nvme-test
make shell-test fs-test keyboard-test vktst-test
make iso iso-smoke uefi-usb usb-debug
make debug-image-test iso-debug-test
BOOT_ITERATIONS=10 python3 scripts/qemu-boot-matrix.py
```

## Common failures

| Symptom | Cause |
|---|---|
| commands ignored | stale serial log, readiness matched old prompt |
| `[ATAPI] command timeout` | missing `A`/`C` header bits, PIO mode, or non-HHDM DMA buffer |
| FS `unknown` on 4Kn | partition offsets not converted to 512-byte units |
| `VIBRALOG` not found | image has no VIBRALOG partition, or CD read-only |
| heartbeat missing | timer IRQ lost EOI or `hlt` with IF=0 |

## Boot log

Serial log is the source of truth. Kernel also mirrors to
`/var/log/boot.log` and to `/VIBRA.LOG` on a FAT32 volume labeled
`VIBRALOG` (USB/SATA/NVMe/ATAPI). `scripts/read-boot-log.sh <device>`
reads it without mounting.
