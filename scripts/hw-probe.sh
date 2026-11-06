#!/usr/bin/env bash
# Backticks внутри printf — это разметка Markdown в отчёте, а не подстановка.
# shellcheck disable=SC2016
# hw-probe.sh — снять паспорт железа под Linux для отладки Vibra Kernel.
#
# Зачем: Vibra видит только то, что умеет её собственный драйвер, и по
# `diag usb` приходится угадывать. Linux на той же машине отвечает на
# вопросы точно: к какому хосту подключено устройство, какая скорость,
# какой режим SATA, есть ли 64-битная адресация у EHCI, какие порты
# отданы companion-контроллеру.
#
# Скрипт ТОЛЬКО читает: он не пишет в регистры устройств, не меняет
# прошивку и не трогает диски. Единственное, что создаётся, — каталог с
# отчётом.
#
# Использование:
#   ./scripts/hw-probe.sh                 # отчёт в ./hw-probe-<модель>-<дата>
#   ./scripts/hw-probe.sh -o /tmp/probe   # явный каталог
#   ./scripts/hw-probe.sh --no-sudo       # без root (часть данных будет пустой)
#   ./scripts/hw-probe.sh --no-raw        # только SUMMARY.md
#
# Для чтения MMIO-регистров EHCI/xHCI нужен root и доступ к /dev/mem.
# Если ядро собрано с CONFIG_IO_STRICT_DEVMEM=y (Fedora, Ubuntu), нужен
# параметр загрузки `iomem=relaxed`: в GRUB нажмите `e`, допишите его в
# строку, начинающуюся с `linux`, и загрузитесь. Для live USB это разово
# и ничего не устанавливает.

set -Eeuo pipefail
shopt -s nullglob


trap 'printf "failed: line %s: %s\n" "$LINENO" "$BASH_COMMAND" >&2' ERR

readonly SCRIPT_VERSION="1.0"
usage() {
    sed -n '/^# Использование:/,/^$/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    printf 'Версия скрипта: %s\n' "$SCRIPT_VERSION"
}

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

note() {
    printf '  %s\n' "$*" >&2
}

# ---------------------------------------------------------------------------
# Аргументы
# ---------------------------------------------------------------------------

out_dir=""
use_sudo=1
want_raw=1

while (($# > 0)); do
    case "$1" in
        -o | --output)
            [[ $# -ge 2 ]] || die "нужен аргумент для $1"
            out_dir="$2"
            shift 2
            ;;
        --no-sudo)
            use_sudo=0
            shift
            ;;
        --no-raw)
            want_raw=0
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            printf 'неизвестный аргумент: %s\n' "$1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

# ---------------------------------------------------------------------------
# Root
# ---------------------------------------------------------------------------

if [[ "$(id -u)" -ne 0 && "$use_sudo" -eq 1 ]]; then
    if command -v sudo >/dev/null 2>&1; then
        note "нужен root: перезапуск через sudo (пароль спросят один раз)"
        reexec=(sudo -E -- "$0")
        [[ -n "$out_dir" ]] && reexec+=(-o "$out_dir")
        reexec+=(--no-sudo)
        exec "${reexec[@]}"
    fi
    note "sudo недоступен: часть данных будет недоступна (--no-sudo)"
fi

is_root=0
[[ "$(id -u)" -eq 0 ]] && is_root=1

# ---------------------------------------------------------------------------
# Вспомогательное
# ---------------------------------------------------------------------------

have() { command -v "$1" >/dev/null 2>&1; }

# Значение sysfs-атрибута или пустая строка. Без этого `<missing 2>/dev/null`
# всё равно печатает ошибку редиректа: её выдаёт оболочка, а не команда.
sysfs_attr() {
    local dir="${1%/}" attr="$2"
    [[ -r "$dir/$attr" ]] || return 0
    tr -d '\n' <"$dir/$attr" 2>/dev/null || true
}

slug() {
    printf '%s' "$1" | tr '[:upper:]' '[:lower:]' | tr -cs '[:alnum:]' '-' |
        sed 's/^-//; s/-$//'
}

ts_date="$(date -u +%Y-%m-%d)"
ts_iso="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

# Модель нужна и для имени каталога, и для SUMMARY.
# Значение DMI: сначала sysfs (доступен без root), затем dmidecode.
# Имена в sysfs и у `dmidecode -s` разные, поэтому таблица соответствий.
dmi_field() {
    local key="$1"
    local sysfs_value=""
    if [[ -r "/sys/class/dmi/id/$key" ]]; then
        sysfs_value="$(tr -d '\n' <"/sys/class/dmi/id/$key" 2>/dev/null || true)"
        [[ -n "$sysfs_value" ]] && {
            printf '%s' "$sysfs_value"
            return 0
        }
    fi
    local dmi_key=""
    case "$key" in
        sys_vendor) dmi_key="system-manufacturer" ;;
        product_name) dmi_key="system-product-name" ;;
        product_version) dmi_key="system-version" ;;
        board_vendor) dmi_key="baseboard-manufacturer" ;;
        board_name) dmi_key="baseboard-product-name" ;;
        bios_vendor) dmi_key="bios-vendor" ;;
        bios_version) dmi_key="bios-version" ;;
        bios_date) dmi_key="bios-release-date" ;;
        chassis_type) dmi_key="chassis-type" ;;
        *) dmi_key="" ;;
    esac
    if [[ -n "$dmi_key" ]] && [[ "$is_root" -eq 1 ]] && have dmidecode; then
        dmidecode -s "$dmi_key" 2>/dev/null | head -1 || true
    fi
}

sys_vendor="$(dmi_field sys_vendor)"
product_name="$(dmi_field product_name)"
board_name="$(dmi_field board_name)"
bios_vendor="$(dmi_field bios_vendor)"
bios_version="$(dmi_field bios_version)"
bios_date="$(dmi_field bios_date)"
machine_label="$(printf '%s %s' "$sys_vendor" "$product_name" | sed 's/^ *//; s/ *$//')"
[[ -n "$machine_label" ]] || machine_label="unknown-machine"

if [[ -z "$out_dir" ]]; then
    out_dir="./hw-probe-$(slug "$machine_label")-$ts_date"
fi

[[ -e "$out_dir" ]] && die "каталог уже существует: $out_dir"
raw_dir="$out_dir/raw"
mkdir -p "$out_dir"
[[ "$want_raw" -eq 1 ]] && mkdir -p "$raw_dir"

readonly REDACTED_SERIAL="REDACTED-SERIAL"
readonly REDACTED_UUID="REDACTED-UUID"
readonly REDACTED_MAC="REDACTED-MAC"
readonly REDACTED_HOST="probe-host"

# Список того, что не удалось снять, и почему.
declare -a missing=()

skip() {
    missing+=("$1 — $2")
    note "пропущено: $1 ($2)"
}

# run <имя> <команда...>
# Пишет в raw/<имя>.txt, не падает на ошибке: недоступное просто пусто.
run() {
    local name="$1"
    shift
    [[ "$want_raw" -eq 1 ]] || return 0
    if ! have "$1"; then
        skip "$name" "нет утилиты $1"
        return 0
    fi
    {
        printf '# %s\n' "$*"
        printf '# date: %s\n\n' "$ts_iso"
        "$@" 2>&1 || printf '\n# exit code: %s\n' "$?"
    } >"$raw_dir/$name.txt" 2>/dev/null || true
}

# ---------------------------------------------------------------------------
# Заголовок
# ---------------------------------------------------------------------------

printf '\n==> hw-probe %s\n' "$SCRIPT_VERSION"
printf '  машина:  %s\n' "$machine_label"
printf '  каталог: %s\n\n' "$out_dir"

# ---------------------------------------------------------------------------
# Мета
# ---------------------------------------------------------------------------

kernel="$(uname -r)"
distro="$(sed -n 's/^PRETTY_NAME="\(.*\)"/\1/p' /etc/os-release 2>/dev/null | head -1)"
[[ -n "$distro" ]] || distro="unknown"

run uname uname -a
[[ "$want_raw" -eq 1 ]] && cp -f /etc/os-release "$raw_dir/os-release.txt" 2>/dev/null || true

# ---------------------------------------------------------------------------
# Система, CPU, прошивка
# ---------------------------------------------------------------------------

run dmi-system dmidecode -t system
run dmi-board dmidecode -t baseboard
run dmi-bios dmidecode -t bios
run dmi-chassis dmidecode -t chassis
run dmi-memory dmidecode -t memory
run dmi-cache dmidecode -t cache
run lscpu lscpu
[[ "$want_raw" -eq 1 ]] && cp -f /proc/cpuinfo "$raw_dir/cpuinfo.txt" 2>/dev/null || true

if [[ "$is_root" -eq 1 ]]; then
    run dmesg dmesg
else
    skip dmesg "нужен root (dmesg.restrict)"
fi

secure_boot="unknown"
if [[ -d /sys/firmware/efi ]]; then
    boot_mode="UEFI"
    # mokutil отвечает в stderr: при 2>/dev/null от него остаётся пустота,
    # поэтому забираем оба потока и разбираем текст.
    if have mokutil; then
        sb_text="$(mokutil --sb-state 2>&1 | tr '\n' ' ' || true)"
        case "$sb_text" in
            *"SecureBoot enabled"*) secure_boot="enabled" ;;
            *"SecureBoot disabled"*) secure_boot="disabled" ;;
            *"doesn't support"*) secure_boot="not supported by firmware" ;;
        esac
    fi
    sb_var="/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c"
    if [[ "$secure_boot" == "unknown" && -r "$sb_var" ]]; then
        # Первые 4 байта — атрибуты, затем значение: 1 = включён.
        sb_byte="$(od -An -tu1 -j4 -N1 "$sb_var" 2>/dev/null | tr -d ' \n')"
        if [[ "$sb_byte" == "1" ]]; then
            secure_boot="enabled"
        elif [[ "$sb_byte" == "0" ]]; then
            secure_boot="disabled"
        fi
    fi
    if [[ "$secure_boot" == "unknown" ]]; then
        secure_boot="unknown (прошивка не сообщила, efivarfs закрыт)"
    fi
else
    boot_mode="BIOS/legacy"
    secure_boot="n/a (no UEFI)"
fi

# ---------------------------------------------------------------------------
# PCI
# ---------------------------------------------------------------------------

run lspci lspci -nn
run lspci-vv lspci -nnvvv
run lspci-tree lspci -tv
run lspci-mmio lspci -mm -nn

# Дампы config space для устройств, которые важны драйверам Vibra.
if [[ "$want_raw" -eq 1 ]] && have lspci; then
    : >"$raw_dir/lspci-configspace.txt"
    while read -r bdf; do
        [[ -n "$bdf" ]] || continue
        {
            printf '\n===== %s =====\n' "$bdf"
            lspci -s "$bdf" -xxx 2>&1 || true
        } >>"$raw_dir/lspci-configspace.txt"
    done < <(lspci -D -nn 2>/dev/null |
        grep -iE 'usb controller|sata|raid|non-volatile|ethernet|vga|display' |
        awk '{print $1}')
fi

# ---------------------------------------------------------------------------
# USB
# ---------------------------------------------------------------------------

run lsusb lsusb
run lsusb-tree lsusb -t
run lsusb-verbose-tree lsusb -tv

if [[ "$want_raw" -eq 1 ]]; then
    # Снимок sysfs по каждому USB-устройству: скорость, версия, классы.
    : >"$raw_dir/usb-sysfs.txt"
    for dev in /sys/bus/usb/devices/*/; do
        [[ -r "$dev/idVendor" ]] || continue
        {
            printf '===== %s =====\n' "$(basename "$dev")"
            for attr in idVendor idProduct bcdDevice bDeviceClass bDeviceSubClass \
                bDeviceProtocol speed version busnum devnum maxchild bNumInterfaces \
                product manufacturer serial removable; do
                [[ -r "$dev/$attr" ]] || continue
                printf '  %-16s %s\n' "$attr" "$(tr -d '\n' <"$dev/$attr" 2>/dev/null)"
            done
        } >>"$raw_dir/usb-sysfs.txt"
    done
fi

# ---------------------------------------------------------------------------
# Регистры USB-контроллеров (MMIO)
# ---------------------------------------------------------------------------
#
# Самый ценный кусок: HCCPARAMS (64-битная адресация EHCI) и живой PORTSC
# каждого порта (CCS/CSC/PED/PR/PP/OWNER/LINESTAT/SPEED). Именно это
# отвечает на «почему флешка не видна».

mmio_probe() {
    local py
    py="$(cat <<'PY'
import mmap, os, struct, sys

def r32(m, off):
    return struct.unpack_from("<I", m, off)[0]

def open_mmio(phys, size):
    fd = os.open("/dev/mem", os.O_RDONLY | os.O_SYNC)
    try:
        return mmap.mmap(fd, size, mmap.MAP_SHARED, mmap.PROT_READ, offset=phys)
    finally:
        os.close(fd)

EHCI_SPEED = {0: "full", 1: "low", 2: "high"}
XHCI_SPEED = {1: "full", 2: "low", 3: "high", 4: "super", 5: "super+"}

def decode_ehci_portsc(v):
    bits = []
    if v & (1 << 0): bits.append("CCS")
    if v & (1 << 1): bits.append("CSC")
    if v & (1 << 2): bits.append("PED")
    if v & (1 << 3): bits.append("PEC")
    if v & (1 << 4): bits.append("OCA")
    if v & (1 << 5): bits.append("OCC")
    if v & (1 << 6): bits.append("FPR")
    if v & (1 << 7): bits.append("SUSP")
    if v & (1 << 8): bits.append("PR")
    if v & (1 << 12): bits.append("PP")
    if v & (1 << 13): bits.append("OWNER=companion")
    linestat = (v >> 10) & 3
    speed = (v >> 26) & 3
    return f"linestat={linestat} speed={speed}({EHCI_SPEED.get(speed, '?')}) {' '.join(bits)}"

def decode_xhci_portsc(v):
    bits = []
    if v & (1 << 0): bits.append("CCS")
    if v & (1 << 1): bits.append("PED")
    if v & (1 << 3): bits.append("OCA")
    if v & (1 << 4): bits.append("PR")
    if v & (1 << 9): bits.append("PP")
    if v & (1 << 17): bits.append("CSC")
    if v & (1 << 21): bits.append("PRC")
    if v & (1 << 22): bits.append("PLC")
    pls = (v >> 5) & 0xF
    speed = (v >> 10) & 0xF
    return f"pls={pls} speed={speed}({XHCI_SPEED.get(speed, '?')}) {' '.join(bits)}"

def probe_ehci(name, phys, size):
    m = open_mmio(phys, size)
    cap0 = r32(m, 0)
    caplen = cap0 & 0xFF
    hci = (cap0 >> 16) & 0xFFFF
    hcs = r32(m, 4)
    hcc = r32(m, 8)
    n_ports = hcs & 0xF
    p_indicator = (hcs >> 16) & 1
    print(f"[EHCI {name}] BAR0={phys:#x} HCIVERSION={hci >> 8}.{hci & 0xFF} "
          f"CAPLEN={caplen} N_PORTS={n_ports} PPC={(hcs >> 4) & 1} "
          f"P_INDICATOR={p_indicator}")
    print(f"  HCCPARAMS={hcc:#010x} 64bit={'YES' if hcc & 1 else 'no'} "
          f"EECP={((hcc >> 8) & 0xFF) * 4:#x} FLFZ={(hcc >> 7) & 1} "
          f"SPC={(hcc >> 8) & 0xFF}")
    op = caplen
    usbcmd = r32(m, op + 0x00)
    usbsts = r32(m, op + 0x04)
    configflag = r32(m, op + 0x40)
    print(f"  USBCMD={usbcmd:#010x} (RS={usbcmd & 1} ASE={(usbcmd >> 5) & 1} "
          f"PSE={(usbcmd >> 4) & 1}) USBSTS={usbsts:#010x} "
          f"(ASS={(usbsts >> 15) & 1} HCHalted={(usbsts >> 12) & 1}) "
          f"CONFIGFLAG={configflag:#x} (CFL={configflag & 1})")
    for port in range(1, n_ports + 1):
        v = r32(m, op + 0x44 + (port - 1) * 4)
        print(f"  port {port}: {v:#010x} {decode_ehci_portsc(v)}")

def probe_xhci(name, phys, size):
    m = open_mmio(phys, size)
    cap0 = r32(m, 0)
    caplen = cap0 & 0xFF
    hci = (cap0 >> 16) & 0xFFFF
    hcs1 = r32(m, 4)
    max_slots = hcs1 & 0xFF
    max_ports = (hcs1 >> 24) & 0xFF
    print(f"[xHCI {name}] BAR0={phys:#x} HCIVERSION={hci >> 8}.{hci & 0xFF} "
          f"CAPLEN={caplen} MaxSlots={max_slots} MaxPorts={max_ports}")
    if max_ports == 0:
        return
    op = caplen
    for port in range(1, max_ports + 1):
        v = r32(m, op + 0x400 + (port - 1) * 0x10)
        print(f"  port {port}: {v:#010x} {decode_xhci_portsc(v)}")

def main():
    entries = []
    for line in sys.stdin.read().splitlines():
        parts = line.split()
        if len(parts) >= 4:
            entries.append(parts)
    failures = 0
    for parts in entries:
        bdf, kind, phys_s, size_s = parts[0], parts[1], parts[2], parts[3]
        phys = int(phys_s, 16)
        size = int(size_s, 16)
        size = min(size, 0x1000) if size else 0x1000
        try:
            if kind == "ehci":
                probe_ehci(bdf, phys, size)
            elif kind == "xhci":
                probe_xhci(bdf, phys, size)
        except Exception as exc:
            failures += 1
            print(f"[{bdf}] MMIO НЕДОСТУПЕН: {type(exc).__name__}: {exc}")
    if failures:
        print()
        print("MMIO не читается. Причины и что делать:")
        print("  CONFIG_IO_STRICT_DEVMEM=y — /dev/mem закрыт для ресурсов,")
        print("  занятых драйвером. Загрузитесь с параметром ядра iomem=relaxed")
        print("  (GRUB: e -> дописать в строку linux -> F10).")

main()
PY
)"

    [[ "$want_raw" -eq 1 ]] || return 0

    # Собираем список контроллеров: BDF, тип, BAR0, размер.
    local list=""
    local dev bdf cls phys size
    for dev in /sys/bus/pci/devices/*/; do
        [[ -r "$dev/class" ]] || continue
        cls="$(cat "$dev/class")"
        bdf="$(basename "$dev")"
        phys="$(awk 'NR==1 {print $1}' "$dev/resource" 2>/dev/null)"
        size="$(awk 'NR==1 {print $3}' "$dev/resource" 2>/dev/null)"
        [[ -n "$phys" && -n "$size" ]] || continue
        case "$cls" in
            0x0c0320) list+="$bdf ehci $phys $size"$'\n' ;;
            0x0c0330) list+="$bdf xhci $phys $size"$'\n' ;;
        esac
    done

    if [[ -z "$list" ]]; then
        printf '# EHCI/xHCI не найдены\n' >"$raw_dir/usb-mmio.txt"
        return 0
    fi

    if [[ "$is_root" -eq 0 ]]; then
        printf '# нужен root для /dev/mem\n' >"$raw_dir/usb-mmio.txt"
        skip usb-mmio "нужен root для чтения MMIO"
        return 0
    fi

    {
        printf '# регистры EHCI/xHCI, снято %s\n' "$ts_iso"
        printf '# только чтение (/dev/mem O_RDONLY)\n\n'
        if have python3; then
            printf '%s\n' "$list" | python3 -c "$py" 2>&1 ||
                printf '\n# python-проба завершилась с ошибкой\n'
        else
            printf '# python3 не найден — нечем прочитать MMIO\n'
        fi
    } >"$raw_dir/usb-mmio.txt" 2>&1 || true
}

mmio_probe

# ---------------------------------------------------------------------------
# Хранилище
# ---------------------------------------------------------------------------

run lsblk lsblk -o NAME,SIZE,TYPE,TRAN,ROTA,LOG-SEC,PHY-SEC,MODEL,SERIAL,MOUNTPOINT
run lsblk-json lsblk -J -o NAME,SIZE,TYPE,TRAN,LOG-SEC,MODEL,MOUNTPOINT
run blockdev-list blockdev --report
[[ "$want_raw" -eq 1 && -r /proc/partitions ]] && cp -f /proc/partitions "$raw_dir/partitions.txt" || true

if have smartctl && [[ "$is_root" -eq 1 ]]; then
    if [[ "$want_raw" -eq 1 ]]; then
        : >"$raw_dir/smartctl.txt"
        while read -r name type; do
            [[ -n "$name" ]] || continue
            {
                printf '\n===== %s (%s) =====\n' "$name" "$type"
                smartctl -i -H -A "$name" 2>&1 || true
            } >>"$raw_dir/smartctl.txt"
        done < <(lsblk -d -n -o NAME,TYPE 2>/dev/null | grep -v loop)
    fi
else
    skip smartctl "нет smartctl или root"
fi

# Режим SATA: контроллер, порты, скорость линка.
run ahci-hosts ls /sys/class/scsi_host
if [[ "$want_raw" -eq 1 ]]; then
    : >"$raw_dir/scsi-hosts.txt"
    for host in /sys/class/scsi_host/host*/; do
        {
            printf '===== %s =====\n' "$host"
            for attr in proc_name unique_id version_str host_busy can_queue; do
                [[ -r "$host/$attr" ]] || continue
                printf '  %-14s %s\n' "$attr" "$(tr -d '\n' <"$host/$attr" 2>/dev/null)"
            done
        } >>"$raw_dir/scsi-hosts.txt"
    done
fi

# ---------------------------------------------------------------------------
# ACPI и память
# ---------------------------------------------------------------------------

if [[ -d /sys/firmware/acpi/tables ]]; then
    if [[ "$want_raw" -eq 1 ]]; then
        {
            printf '# ACPI-таблицы из sysfs\n\n'
            ls -l /sys/firmware/acpi/tables 2>&1 || true
        } >"$raw_dir/acpi-tables.txt"
        mkdir -p "$raw_dir/acpi"
        for table in FADT MADT MCFG HPET DSDT FACS SRAT SLIT DMAR IVRS BERT ECDT; do
            [[ -r "/sys/firmware/acpi/tables/$table" ]] || continue
            cp -f "/sys/firmware/acpi/tables/$table" "$raw_dir/acpi/$table" 2>/dev/null || true
        done
    fi
else
    skip acpi "нет /sys/firmware/acpi/tables"
fi

run acpidump acpidump -b -o /dev/stdout
run iomem cat /proc/iomem
run ioports cat /proc/ioports
if [[ -r /proc/mtrr ]]; then
    run mtrr cat /proc/mtrr
else
    skip mtrr "/proc/mtrr отсутствует (ядро без MTRR или нет прав)"
fi
run meminfo cat /proc/meminfo
run efi-systab ls /sys/firmware/efi/systab

# ---------------------------------------------------------------------------
# Экран и сеть
# ---------------------------------------------------------------------------

if [[ "$want_raw" -eq 1 ]]; then
    : >"$raw_dir/drm.txt"
    for card in /sys/class/drm/card*/; do
        [[ -d "$card" ]] || continue
        {
            printf '===== %s =====\n' "$card"
            for attr in status enabled modes; do
                [[ -r "$card/$attr" ]] || continue
                printf '  %-8s %s\n' "$attr" "$(tr '\n' ' ' <"$card/$attr" 2>/dev/null)"
            done
        } >>"$raw_dir/drm.txt"
    done
fi
run framebuffer ls -l /sys/class/graphics
if [[ "$want_raw" -eq 1 ]]; then
    : >"$raw_dir/fb.txt"
    for fb in /sys/class/graphics/fb*/; do
        [[ -d "$fb" ]] || continue
        {
            printf '===== %s =====\n' "$fb"
            for attr in name bits_per_pixel stride virtual_size modes; do
                [[ -r "$fb/$attr" ]] || continue
                printf '  %-16s %s\n' "$attr" "$(tr '\n' ' ' <"$fb/$attr" 2>/dev/null)"
            done
        } >>"$raw_dir/fb.txt"
    done
fi

run ip-addr ip -brief address
run ethtool-driver ethtool -i eth0
if [[ "$want_raw" -eq 1 ]]; then
    : >"$raw_dir/net.txt"
    for net in /sys/class/net/*/; do
        name="$(basename "$net")"
        [[ "$name" == "lo" ]] && continue
        {
            printf '===== %s =====\n' "$name"
            for attr in address address_len type carrier speed duplex mtu operstate; do
                [[ -r "$net/$attr" ]] || continue
                printf '  %-12s %s\n' "$attr" "$(tr -d '\n' <"$net/$attr" 2>/dev/null)"
            done
        } >>"$raw_dir/net.txt"
    done
fi

# ---------------------------------------------------------------------------
# Редакция: убрать серийники, UUID, MAC и прочее личное
# ---------------------------------------------------------------------------

redact_file() {
    local file="$1"
    [[ -f "$file" ]] || return 0
    # Бинарные таблицы ACPI не трогаем.
    case "$file" in
        */raw/acpi/*) return 0 ;;
    esac
    sed -E -i \
        -e 's/(Serial Number:[[:space:]]*).*/\1'"$REDACTED_SERIAL"'/I' \
        -e 's/(SerialNumber[[:space:]]*:[[:space:]]*).*/\1'"$REDACTED_SERIAL"'/I' \
        -e 's/(serial[[:space:]]*:[[:space:]]*)[^[:space:]]{6,}/\1'"$REDACTED_SERIAL"'/I' \
        -e 's/(UUID:[[:space:]]*).*/\1'"$REDACTED_UUID"'/I' \
        -e 's/(Asset Tag:[[:space:]]*).*/\1'"$REDACTED_SERIAL"'/I' \
        -e 's/([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}/'"$REDACTED_MAC"'/g' \
        -e 's/^Hostname:[[:space:]]*.*/Hostname: '"$REDACTED_HOST"'/I' \
        "$file" 2>/dev/null || true
}

if [[ "$want_raw" -eq 1 ]]; then
    for file in "$raw_dir"/*.txt; do
        redact_file "$file"
    done
fi

# ---------------------------------------------------------------------------
# SUMMARY.md — то, что нужно видеть сразу
# ---------------------------------------------------------------------------

cpu_model="$(sed -n 's/^model name[[:space:]]*: //p' /proc/cpuinfo 2>/dev/null | head -1)"
[[ -n "$cpu_model" ]] || cpu_model="$(lscpu 2>/dev/null | sed -n 's/^Model name:[[:space:]]*//p' | head -1)"
cpu_cores="$(lscpu 2>/dev/null | sed -n 's/^CPU(s):[[:space:]]*//p' | head -1)"
mem_total="$(sed -n 's/^MemTotal:[[:space:]]*//p' /proc/meminfo 2>/dev/null | head -1)"

usb_controllers_md() {
    local dev bdf cls phys driver
    for dev in /sys/bus/pci/devices/*/; do
        [[ -r "$dev/class" ]] || continue
        cls="$(cat "$dev/class")"
        case "$cls" in
            0x0c0300 | 0x0c0310 | 0x0c0320 | 0x0c0330) ;;
            *) continue ;;
        esac
        bdf="$(basename "$dev")"
        driver="$(basename "$(readlink -f "$dev/driver" 2>/dev/null)" 2>/dev/null)"
        [[ -n "$driver" && "$driver" != "." ]] || driver="—"
        phys="$(awk 'NR==1 {print $1}' "$dev/resource" 2>/dev/null)"
        local kind
        case "$cls" in
            0x0c0300) kind="UHCI (USB 1.1)" ;;
            0x0c0310) kind="OHCI (USB 1.1)" ;;
            0x0c0320) kind="EHCI (USB 2.0)" ;;
            0x0c0330) kind="xHCI (USB 3.x)" ;;
        esac
        local pci_id
        pci_id="$(lspci -s "$bdf" -nn 2>/dev/null |
            grep -oE '\[[0-9a-f]{4}:[0-9a-f]{4}\]' | head -1 | tr -d '[]')"
        [[ -n "$pci_id" ]] || pci_id="?"
        printf '| %s | %s | `%s` | %s | `%s` |\n' \
            "$bdf" "$kind" "$pci_id" "$driver" "$phys"
    done
}

usb_devices_md() {
    local dev name speed version product vendor
    for dev in /sys/bus/usb/devices/*/; do
        [[ -r "$dev/idVendor" ]] || continue
        name="$(basename "$dev")"
        # Только устройства на шине (имя вида 1-2), не интерфейсы.
        [[ "$name" == *:* ]] && continue
        [[ "$name" == usb* ]] && continue
        speed="$(sysfs_attr "$dev" speed | tr -d ' ')"
        version="$(sysfs_attr "$dev" version | tr -d ' ')"
        product="$(sysfs_attr "$dev" product)"
        vendor="$(sysfs_attr "$dev" manufacturer)"
        printf '| %s | %s:%s | %s | %s | %s |\n' \
            "$name" "$(sysfs_attr "$dev" idVendor)" "$(sysfs_attr "$dev" idProduct)" \
            "${speed:-?}" "${version:-?}" "${vendor:+$vendor }${product:-—}"
    done
}

storage_controllers_md() {
    local dev bdf cls pci_id driver
    for dev in /sys/bus/pci/devices/*/; do
        [[ -r "$dev/class" ]] || continue
        cls="$(cat "$dev/class")"
        # sysfs отдаёт полный 24-битный class: 0x010601 = SATA/AHCI,
        # 0x010802 = NVMe. Сравниваем по префиксу класс+подкласс.
        case "$cls" in
            0x0106*) ;; # SATA (AHCI)
            0x0104*) ;; # RAID
            0x0108*) ;; # NVMe
            0x0101*) ;; # IDE (PIIX)
            *) continue ;;
        esac
        bdf="$(basename "$dev")"
        driver="$(basename "$(readlink -f "$dev/driver" 2>/dev/null)" 2>/dev/null)"
        [[ -n "$driver" && "$driver" != "." ]] || driver="—"
        pci_id="$(lspci -s "$bdf" -nn 2>/dev/null |
            grep -oE '\[[0-9a-f]{4}:[0-9a-f]{4}\]' | head -1 | tr -d '[]')"
        local kind
        case "$cls" in
            0x0106*) kind="SATA (AHCI)" ;;
            0x0104*) kind="RAID" ;;
            0x0108*) kind="NVMe" ;;
            0x0101*) kind="IDE (PIIX)" ;;
            *) kind="?" ;;
        esac
        printf '| %s | %s | `%s` | %s |\n' "$bdf" "$kind" "${pci_id:-?}" "$driver"
    done
}

# Строки линка SATA из dmesg: видно, поднялся ли линк и на какой скорости.
sata_link_lines() {
    local f="$raw_dir/dmesg.txt"
    [[ -f "$f" ]] || return 0
    grep -E 'SATA link (up|down)|ata[0-9]+: .*(Exception|failed|hard reset)' "$f" 2>/dev/null |
        head -20 || true
}

storage_md() {
    lsblk -d -n -o NAME,SIZE,TRAN,LOG-SEC,PHY-SEC,MODEL 2>/dev/null |
        while read -r name size tran logsec physec model; do
            [[ -n "$name" ]] || continue
            [[ "$tran" == "usb" || "$tran" == "sata" || "$tran" == "nvme" || "$tran" == "ata" ]] || continue
            printf '| %s | %s | %s | %s | %s | %s |\n' \
                "$name" "$size" "${tran:-?}" "${logsec:-?}" "${physec:-?}" "${model:-—}"
        done
}

mmio_facts() {
    local f="$raw_dir/usb-mmio.txt"
    [[ -f "$f" ]] || return 0
    local decoded
    decoded="$(grep -E '^\[(EHCI|xHCI) ' "$f" 2>/dev/null || true)"
    if [[ -n "$decoded" ]]; then
        printf '%s\n' "$decoded"
        grep -E '^  (HCCPARAMS|USBCMD|port )' "$f" 2>/dev/null || true
    else
        # Декодировать нечего: показываем причину, а не пустой блок.
        grep -vE '^#|^$' "$f" 2>/dev/null | head -20 || true
    fi
}

{
    printf '# Паспорт железа: %s\n\n' "$machine_label"
    printf 'Снято скриптом `scripts/hw-probe.sh` v%s, %s (UTC).\n\n' \
        "$SCRIPT_VERSION" "$ts_iso"
    printf 'Данные собраны **на Linux**, а не в Vibra Kernel: это описание того,\n'
    printf 'что физически стоит в машине. Результаты прогонов Vibra живут\n'
    printf 'отдельно, в `docs/verified-hardware.md`.\n\n'

    printf '## Машина\n\n'
    printf '| Поле | Значение |\n|---|---|\n'
    printf '| Производитель | %s |\n' "${sys_vendor:-?}"
    printf '| Модель | %s |\n' "${product_name:-?}"
    printf '| Материнская плата | %s |\n' "${board_name:-?}"
    printf '| BIOS/UEFI | %s %s |\n' "${bios_vendor:-?}" "${bios_version:-?}"
    printf '| Прошивка от | %s |\n' "${bios_date:-?}"
    printf '| Режим загрузки | %s |\n' "$boot_mode"
    printf '| Secure Boot | %s |\n' "$secure_boot"
    printf '| CPU | %s |\n' "${cpu_model:-?}"
    printf '| Ядер/потоков | %s |\n' "${cpu_cores:-?}"
    printf '| RAM | %s |\n' "${mem_total:-?}"
    printf '| Ядро Linux | %s |\n' "$kernel"
    printf '| Дистрибутив | %s |\n\n' "$distro"

    printf '## USB-контроллеры\n\n'
    printf '| PCI | Тип | ID | Драйвер Linux | BAR0 |\n|---|---|---|---|---|\n'
    usb_controllers_md
    printf '\n'

    if [[ -f "$raw_dir/usb-mmio.txt" ]]; then
        printf '### Регистры (чтение /dev/mem)\n\n```text\n'
        mmio_facts
        printf '```\n\n'
    fi

    printf '## USB-устройства\n\n'
    printf '| Путь | VID:PID | Мбит/с | USB | Название |\n|---|---|---|---|---|\n'
    usb_devices_md
    printf '\n'

    if [[ -f "$raw_dir/lsusb-tree.txt" ]]; then
        printf '### Дерево (`lsusb -t`)\n\n```text\n'
        sed 's/^# .*//' "$raw_dir/lsusb-tree.txt" | sed '/^$/d'
        printf '```\n\n'
    fi

    printf '## Контроллеры хранения\n\n'
    printf '| PCI | Тип | ID | Драйвер Linux |\n|---|---|---|---|\n'
    storage_controllers_md
    printf '\n'
    if [[ -f "$raw_dir/dmesg.txt" ]]; then
        links="$(sata_link_lines)"
        if [[ -n "$links" ]]; then
            printf 'Линки SATA из dmesg:\n\n```text\n%s\n```\n\n' "$links"
        fi
    fi

    printf '## Накопители\n\n'
    printf '| Устройство | Размер | Шина | Лог. сектор | Физ. сектор | Модель |\n|---|---|---|---|---|---|\n'
    storage_md
    printf '\n'

    printf '## Что не удалось снять\n\n'
    if ((${#missing[@]} == 0)); then
        printf 'Всё снято.\n'
    else
        printf '| Раздел | Причина |\n|---|---|\n'
        for item in "${missing[@]}"; do
            printf '| %s |\n' "$item"
        done
    fi
    printf '\n'

    printf '## Файлы\n\n'
    printf '```\n'
    printf 'SUMMARY.md      этот файл\n'
    printf 'raw/            сырые логи (серийники, UUID и MAC отредактированы)\n'
    printf '```\n'
} >"$out_dir/SUMMARY.md"

# ---------------------------------------------------------------------------
# Архив
# ---------------------------------------------------------------------------

archive=""
if have tar; then
    archive="${out_dir}.tar.gz"
    tar -czf "$archive" -C "$(dirname "$out_dir")" "$(basename "$out_dir")"
fi

# ---------------------------------------------------------------------------
# Владелец отчёта
# ---------------------------------------------------------------------------
#
# Скрипт перезапускается через sudo, поэтому файлы создаются от root и
# обычный пользователь не может их прочитать или удалить. Возвращаем
# владельца тому, кто запустил.

if [[ -n "${SUDO_USER:-}" && "$SUDO_USER" != "root" ]]; then
    if chown -R "$SUDO_USER:" "$out_dir" 2>/dev/null; then
        note "владелец отчёта: $SUDO_USER"
    else
        note "не удалось вернуть владельца $SUDO_USER (группа не определена)"
        chown -R "$SUDO_USER" "$out_dir" 2>/dev/null ||
            note "отчёт остался за root: удалить — sudo rm -rf $out_dir"
    fi
    [[ -n "$archive" && -f "$archive" ]] && chown "$SUDO_USER:" "$archive" 2>/dev/null || true
elif [[ -n "${SUDO_UID:-}" ]]; then
    chown -R "$SUDO_UID:${SUDO_GID:-$SUDO_UID}" "$out_dir" 2>/dev/null || true
    [[ -n "$archive" && -f "$archive" ]] &&
        chown "$SUDO_UID:${SUDO_GID:-$SUDO_UID}" "$archive" 2>/dev/null || true
fi

printf '\n==> готово\n'
printf '  отчёт: %s\n' "$out_dir/SUMMARY.md"
[[ -n "$archive" ]] && printf '  архив: %s\n' "$archive"
if ((${#missing[@]} > 0)); then
    printf '  не снято: %s раздел(ов) — см. SUMMARY.md\n' "${#missing[@]}"
fi
printf '\n  Пришлите архив: он и есть паспорт машины.\n'