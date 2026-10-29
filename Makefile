KERNEL_NAME := vibra
TARGET := x86_64-unknown-none
BUILD_DIR := build
LIMINE_DIR := limine-bin

# Загрузчик и прошивку можно закрепить на конкретной версии и SHA256.
# По умолчанию поведение прежнее (latest), но для воспроизводимой сборки:
#   make setup LIMINE_VERSION=v9.2.0 LIMINE_SHA256=<sha> OVMF_SHA256=<sha>
LIMINE_VERSION ?= latest
LIMINE_SHA256 ?=
OVMF_SHA256 ?=
ifeq ($(LIMINE_VERSION),latest)
LIMINE_URL := https://github.com/limine-bootloader/limine/releases/latest/download/limine-binary.tar.xz
else
LIMINE_URL := https://github.com/limine-bootloader/limine/releases/download/$(LIMINE_VERSION)/limine-binary.tar.xz
endif
OVMF_URL ?= https://retrage.github.io/edk2-nightly/bin/RELEASEX64_OVMF.fd

# QEMU можно переопределить из окружения (например, qemu-system-x86_64 из PATH в CI).
QEMU ?= /usr/bin/qemu-system-x86_64

# QEMU environment wrapper: clear env to avoid snap/core20 libs,
# but preserve display-related vars when run in a GUI session.
QEMU_ENV := env -i PATH=/usr/bin:/bin TERM=$(TERM)
ifdef DISPLAY
QEMU_ENV := env -i PATH=/usr/bin:/bin TERM=$(TERM) DISPLAY=$(DISPLAY) XAUTHORITY=$(XAUTHORITY) XDG_RUNTIME_DIR=$(XDG_RUNTIME_DIR)
endif
CARGO := cargo +nightly
SERIAL_DEBUG ?= 1
RELEASE ?= 0

ifeq ($(SERIAL_DEBUG),0)
CARGO_FEATURES := --no-default-features
else
CARGO_FEATURES :=
endif

ifeq ($(RELEASE),1)
PROFILE := release
CARGO_PROFILE := --release
else
PROFILE := debug
CARGO_PROFILE :=
endif

# Путь к собранному ядру (без привязки к диску — нужен и для ISO, и для hdd.img).
KERNEL_ELF := target/$(TARGET)/$(PROFILE)/$(KERNEL_NAME)

# COM1 привязан к stdin/stdout: можно вводить shell-команды из терминала,
# сохраняя графическое окно QEMU для framebuffer-консоли.
## SERIAL configuration: choose how QEMU exposes the serial port.
# SERIAL_MODE: stdio (default), tcp (host TCP port), file (log to file)
SERIAL_MODE ?= stdio
SERIAL_PORT ?= 4444
SERIAL_FILE ?= $(BUILD_DIR)/serial.log

ifeq ($(SERIAL_MODE),tcp)
QEMU_SERIAL := -serial tcp::$(SERIAL_PORT),server,nowait
else ifeq ($(SERIAL_MODE),file)
QEMU_SERIAL := -serial file:$(SERIAL_FILE)
else
QEMU_SERIAL := -serial stdio
endif

QEMU_FLAGS := -m 256M $(QEMU_SERIAL) -monitor none -drive if=pflash,format=raw,file=OVMF.fd,readonly=on \
              -drive file=$(BUILD_DIR)/hdd.img,format=raw -M q35 \
			  -device e1000,netdev=net0 -netdev user,id=net0,hostfwd=tcp::2222-:22

# Run without graphical display if requested
HEADLESS ?= 0
ifeq ($(HEADLESS),1)
QEMU_FLAGS += -display none
endif

# Optional USB devices (xHCI + basic keyboard/mouse)
ENABLE_USB ?= 0
USB_FLAGS := -device nec-usb-xhci,id=xhci -device usb-kbd,bus=xhci.0 -device usb-mouse,bus=xhci.0
ifeq ($(ENABLE_USB),1)
QEMU_FLAGS += $(USB_FLAGS)
endif

.PHONY: all build build-release run-release kernel smoke run clean setup install-kernel iso iso-release iso-smoke usb uefi-usb run-iso run-bios qemu-usb fs-test keyboard-test clock-test ramfs-test ro-mount-test bad-fs-test nvme-test shell-test check boot-log-kernel boot-log-test usb-storage-test usb-debug iso-debug read-boot-log

all: run

# Только компиляция ядра — без диска. Нужна ISO/USB-сборкам.
# $(CARGO_PROFILE) подставляет --release при RELEASE=1: без этого флаг
# менял только путь KERNEL_ELF, а cargo собирал debug-профиль.
kernel:
	$(CARGO) build -p vibra-kernel $(CARGO_FEATURES) $(CARGO_PROFILE) -Z build-std=core,alloc,compiler_builtins -Z build-std-features

# Сборка ядра И копирование его на диск
build: kernel
	@mkdir -p $(BUILD_DIR)
	@if [ ! -f "$(BUILD_DIR)/hdd.img" ]; then \
		$(MAKE) setup; \
	fi
	@echo "==> Copying kernel to disk image..."
	@mcopy -o -i $(BUILD_DIR)/hdd.img $(KERNEL_ELF) ::/kernel.elf
	@echo "==> Kernel built and installed!"

smoke:
	@QEMU="$(QEMU)" bash scripts/qemu-smoke.sh

# End-to-end проверка FAT32/разделов: второй диск + команды через serial.
fs-test:
	@QEMU="$(QEMU)" bash scripts/qemu-fs-test.sh

# Проверка ввода: PS/2 и USB HID клавиатура (QEMU monitor sendkey).
# KEYBOARD_CASE=ps2|usb|all (по умолчанию all).
keyboard-test: build
	@QEMU="$(QEMU)" bash scripts/qemu-keyboard-test.sh "$(KEYBOARD_CASE)"

# Проверка часов: RTC/date, uptime, diag timer, /proc/uptime и чистота рамфс.
clock-test: build
	@QEMU="$(QEMU)" bash scripts/qemu-clock-test.sh

# Корневой RamFS через VFS: ls, запись/чтение, hostname, /boot.log.
ramfs-test: build
	@QEMU="$(QEMU)" bash scripts/qemu-ramfs-test.sh

# Read-only по умолчанию: запись на FAT32 запрещена без явного `rw`.
ro-mount-test: build
	@QEMU="$(QEMU)" bash scripts/qemu-ro-mount-test.sh

# Повреждённые ФС не должны ронять ядро.
bad-fs-test: build
	@QEMU="$(QEMU)" bash scripts/qemu-bad-fs-test.sh

# NVMe: Identify, I/O queues, block layer, mount auto.
nvme-test: build
	@QEMU="$(QEMU)" bash scripts/qemu-nvme-test.sh

# Интерактивный shell: Tab, история, help, объединённый test.
shell-test: build
	@QEMU="$(QEMU)" bash scripts/qemu-shell-test.sh

# Локальный quality gate: обе сборки + бюджет предупреждений.
check:
	@$(MAKE) kernel
	@$(MAKE) kernel SERIAL_DEBUG=0
	@python3 scripts/check-warning-budget.py

# Проверка загрузки собранного ISO: UEFI (OVMF) и BIOS (SeaBIOS) через El Torito.
iso-smoke: iso
	@QEMU="$(QEMU)" bash scripts/qemu-iso-smoke.sh

setup:
	@echo "==> Setting up boot image..."
	@mkdir -p $(BUILD_DIR)
	@rm -rf $(LIMINE_DIR)
	
	@echo "==> Downloading Limine precompiled binaries..."
	@wget -q "$(LIMINE_URL)"
	@if [ -n "$(LIMINE_SHA256)" ]; then \
		echo "$(LIMINE_SHA256)  limine-binary.tar.xz" | sha256sum -c -; \
	fi
	@tar -xf limine-binary.tar.xz
	@mv limine-binary $(LIMINE_DIR)
	@rm limine-binary.tar.xz

	@if [ ! -f "OVMF.fd" ]; then \
		echo "==> Downloading UEFI BIOS (OVMF)..."; \
		wget -q "$(OVMF_URL)" -O OVMF.fd; \
		if [ -n "$(OVMF_SHA256)" ]; then \
			echo "$(OVMF_SHA256)  OVMF.fd" | sha256sum -c -; \
		fi; \
	fi

	@echo "==> Creating limine.conf..."
	@printf "timeout: 0\n" > limine.conf
	@printf "graphics: yes\n" >> limine.conf
	@printf "\n" >> limine.conf
	@printf "/Vibra\n" >> limine.conf
	@printf "    protocol: limine\n" >> limine.conf
	@printf "    kernel_path: boot():/kernel.elf\n" >> limine.conf
	
	@echo "==> Creating FAT32 disk image (64MB)..."
	@dd if=/dev/zero of=$(BUILD_DIR)/hdd.img bs=1M count=64 status=none
	@mkfs.fat -F 32 -n VIBRA $(BUILD_DIR)/hdd.img
	
	@echo "==> Copying bootloader files to disk image..."
	@mmd -i $(BUILD_DIR)/hdd.img ::/EFI ::/EFI/BOOT ::/limine
	@mcopy -i $(BUILD_DIR)/hdd.img $(LIMINE_DIR)/BOOTX64.EFI ::/EFI/BOOT/
	@mcopy -i $(BUILD_DIR)/hdd.img limine.conf ::/limine/
	
	@echo "==> Verifying disk contents:"
	@mdir -i $(BUILD_DIR)/hdd.img ::/
	@echo "==> Setup complete! Now run 'make build' to compile and install the kernel."

# Быстрое обновление только ядра (без пересборки)
install-kernel:
	@echo "==> Installing kernel to disk image..."
	@mcopy -o -i $(BUILD_DIR)/hdd.img $(shell pwd)/$(KERNEL_ELF) ::/kernel.elf
	@echo "==> Kernel installed!"

# Release-сборка: оптимизация + COM1 выключен.
# Обёртки над build/kernel с RELEASE=1 SERIAL_DEBUG=0 — без дублирования
# флагов cargo (раньше --release/--no-default-features были прописаны здесь
# жёстко, а kernel: собирал debug-профиль при RELEASE=1).
run-release: build-release
	@echo "==> Starting QEMU (release, no serial shell)..."
	$(QEMU_ENV) $(QEMU) -m 256M -serial none -monitor none -drive if=pflash,format=raw,file=OVMF.fd,readonly=on \
	         -drive file=$(BUILD_DIR)/hdd.img,format=raw -M q35

build-release:
	@$(MAKE) RELEASE=1 SERIAL_DEBUG=0 build
	@echo "==> Release kernel built and installed!"

run: build
	@echo "==> Starting QEMU..."
	$(QEMU_ENV) $(QEMU) $(QEMU_FLAGS)

# QEMU с сетью (e1000 + SSH на порт 2222)
qemu-net: build
	@echo "==> Starting QEMU with networking (e1000, SSH on port 2222)..."
	$(QEMU_ENV) $(QEMU) $(QEMU_FLAGS)

run-usb: build
	@echo "==> Starting QEMU with USB devices (xHCI keyboard/mouse)..."
	$(QEMU_ENV) $(QEMU) $(QEMU_FLAGS) $(USB_FLAGS)

run-serial-tcp: build
	@echo "==> Starting QEMU with serial over TCP (port $(SERIAL_PORT))"
	@echo "Connect with: telnet localhost $(SERIAL_PORT)  OR  socat -,raw,echo=0 TCP:localhost:$(SERIAL_PORT)"
	$(MAKE) SERIAL_MODE=tcp run

run-serial-file: build
	@echo "==> Starting QEMU with serial logged to $(SERIAL_FILE)"
	$(MAKE) SERIAL_MODE=file run

run-bios-usb: iso
	@echo "==> Starting QEMU (SeaBIOS) with USB devices..."
	$(QEMU_ENV) $(QEMU) $(QEMU_FLAGS) $(USB_FLAGS) -cdrom $(BUILD_DIR)/vibra.iso -M q35 -display none

run-iso-usb: iso
	@echo "==> Starting QEMU (UEFI) with USB devices..."
	$(QEMU_ENV) $(QEMU) $(QEMU_FLAGS) $(USB_FLAGS) -cdrom $(BUILD_DIR)/vibra.iso -drive if=pflash,format=raw,file=OVMF.fd,readonly=on -M q35 -display none

clean:
	$(CARGO) clean
	rm -rf $(BUILD_DIR) $(LIMINE_DIR) OVMF.fd limine-binary.tar.xz

.PHONY: deps-ubuntu
deps-ubuntu:
	@echo "==> Installing common dependencies on Debian/Ubuntu (requires sudo)"
	@sudo apt update && sudo apt install -y qemu-system-x86 mtools dosfstools xorriso wget mtools mtools

# === ISO image для реального железа ===
# Собирает гибридный ISO (BIOS El Torito + UEFI) с Limine, ядром и конфигом.
# Записывается на USB/CD-R для запуска на реальном PC.
#
# Зависит только от `kernel`: hdd.img для ISO не нужен.
iso: kernel
	@echo "==> Creating ISO image..."
	@mkdir -p $(BUILD_DIR)
	@rm -rf $(BUILD_DIR)/iso_root
	@mkdir -p $(BUILD_DIR)/iso_root/limine
	@mkdir -p $(BUILD_DIR)/iso_root/EFI/BOOT

	@echo "==> Copying kernel to ISO root..."
	@cp $(KERNEL_ELF) $(BUILD_DIR)/iso_root/kernel.elf

	@echo "==> Copying Limine config..."
	@cp limine.conf $(BUILD_DIR)/iso_root/limine/

	@echo "==> Copying Limine bootloaders (UEFI + BIOS CD)..."
	@cp $(LIMINE_DIR)/BOOTX64.EFI $(BUILD_DIR)/iso_root/EFI/BOOT/
	@cp $(LIMINE_DIR)/limine-bios-cd.bin $(BUILD_DIR)/iso_root/limine/
	@cp $(LIMINE_DIR)/limine-uefi-cd.bin $(BUILD_DIR)/iso_root/limine/
	@cp $(LIMINE_DIR)/limine-bios.sys $(BUILD_DIR)/iso_root/limine/

	@echo "==> Creating bootable ISO with xorriso..."
	xorriso -as mkisofs \
		-b limine/limine-bios-cd.bin \
		-no-emul-boot \
		-boot-load-size 4 \
		-boot-info-table \
		--efi-boot limine/limine-uefi-cd.bin \
		-efi-boot-part \
		--efi-boot-image \
		--protective-msdos-label \
		-o $(BUILD_DIR)/vibra.iso \
		$(BUILD_DIR)/iso_root

	@echo "==> Installing Limine BIOS bootloader (hybrid MBR)..."
	@if [ ! -x $(LIMINE_DIR)/limine ]; then \
		echo "    building limine host tool from limine.c"; \
		cc -O2 -o $(LIMINE_DIR)/limine $(LIMINE_DIR)/limine.c; \
	fi
	@$(LIMINE_DIR)/limine bios-install $(BUILD_DIR)/vibra.iso

	@echo "==> ISO image created: $(BUILD_DIR)/vibra.iso"
	@ls -lh $(BUILD_DIR)/vibra.iso
	@echo ""
	@echo "==> Write to USB with Ventoy: just copy .iso to Ventoy USB drive"
	@echo "==> Or burn: sudo dd if=$(BUILD_DIR)/vibra.iso of=/dev/sdX bs=4M status=progress"

# ISO с release-ядром: оптимизация + COM1 полностью выключен.
# Эквивалент `make RELEASE=1 SERIAL_DEBUG=0 iso`; отдельный таргет нужен,
# чтобы release-ISO собиралась одной командой без запоминания переменных.
iso-release:
	@$(MAKE) RELEASE=1 SERIAL_DEBUG=0 iso

run-iso: iso
	@echo "==> Starting QEMU with ISO..."
	$(QEMU_ENV) $(QEMU) -m 512M -serial stdio -monitor none \
		-cdrom $(BUILD_DIR)/vibra.iso \
		-drive if=pflash,format=raw,file=OVMF.fd,readonly=on \
		-M q35 -display none

run-bios: iso
	@echo "==> Starting QEMU with ISO via SeaBIOS..."
	$(QEMU_ENV) $(QEMU) -m 512M -serial stdio -monitor none \
		-cdrom $(BUILD_DIR)/vibra.iso \
		-M q35 -display none

qemu-usb: build
	@echo "==> Starting QEMU with xHCI USB keyboard/mouse..."
	$(QEMU_ENV) $(QEMU) $(QEMU_FLAGS) \
		-device nec-usb-xhci,id=xhci \
		-device usb-kbd,bus=xhci.0 \
		-device usb-mouse,bus=xhci.0

# === USB flash image для реального железа ===
# Создаёт GPT-размеченный FAT32 образ с Limine (BIOS + UEFI).
# Записывается на USB: sudo dd if=build/vibra-usb.img of=/dev/sdX bs=4M status=progress

USB_SIZE_MB ?= 128
USB_IMG := $(BUILD_DIR)/vibra-usb.img
UEFI_USB_OVERWRITE ?= 0

usb: build
	@echo "==> Creating USB image ($(USB_SIZE_MB)MB)..."
	@mkdir -p $(BUILD_DIR)
	@rm -f $(USB_IMG)
	@dd if=/dev/zero of=$(USB_IMG) bs=1M count=$(USB_SIZE_MB) status=none

	@echo "==> Creating MBR partition + FAT32..."
	@printf 'o\nn\np\n1\n\n\na\nt\nc\nw\n' | fdisk $(USB_IMG) > /dev/null 2>&1 || true
	@PART_SECTORS=$$(( $(USB_SIZE_MB) * 2048 - 2048 )); \
	dd if=/dev/zero of=$(BUILD_DIR)/usb_part.img bs=512 count=$$PART_SECTORS status=none; \
	mkfs.fat -F 32 -n VIBRA $(BUILD_DIR)/usb_part.img; \
	dd if=$(BUILD_DIR)/usb_part.img of=$(USB_IMG) bs=512 seek=2048 conv=notrunc status=none; \
	rm -f $(BUILD_DIR)/usb_part.img

	@echo "==> Copying files to partition..."
	@mmd -i $(USB_IMG)@@1048576 ::/EFI ::/EFI/BOOT ::/limine
	@mcopy -i $(USB_IMG)@@1048576 $(KERNEL_ELF) ::/kernel.elf
	@mcopy -i $(USB_IMG)@@1048576 limine.conf ::/limine/
	@mcopy -i $(USB_IMG)@@1048576 $(LIMINE_DIR)/BOOTX64.EFI ::/EFI/BOOT/

	@echo "==> Installing Limine BIOS bootloader..."
	@$(LIMINE_DIR)/limine bios-install $(USB_IMG) 1 --force

	@echo ""
	@echo "==> USB image ready: $(USB_IMG)"
	@ls -lh $(USB_IMG)
	@echo ""
	@echo "==> Flash: sudo dd if=$(USB_IMG) of=/dev/sdX bs=4M status=progress"
	@echo "==> ThinkPad T450: Disable Secure Boot in BIOS!"

# UEFI-only GPT image with a standards-compliant EFI System Partition.
uefi-usb:
	$(CARGO) build -p vibra-kernel $(CARGO_FEATURES) $(CARGO_PROFILE) -Z build-std=core,alloc,compiler_builtins -Z build-std-features
	@UEFI_USB_OVERWRITE="$(UEFI_USB_OVERWRITE)" USB_SIZE_MB="$(USB_SIZE_MB)" bash scripts/build-uefi-usb.sh


# === Boot-лог на USB (feature boot-log-to-usb) ===
#
# Ядро пишет кольцевой boot-лог в /VIBRA.LOG на FAT32-раздел с меткой
# VIBRALOG. Двойное согласие: feature при сборке + метка тома.

LOG_PARTITION_MB ?= 64
USB_DEBUG_SIZE_MB ?= 256
USB_DEBUG_ESP_MB ?= 64

# Ядро с записью лога на USB (для образов usb-debug / iso-debug).
boot-log-kernel:
	$(CARGO) build -p vibra-kernel $(CARGO_FEATURES) $(CARGO_PROFILE) --features boot-log-to-usb \
		-Z build-std=core,alloc,compiler_builtins -Z build-std-features

# End-to-end: USB-накопитель с разделом VIBRALOG, проверка содержимого лога.
boot-log-test:
	@QEMU="$(QEMU)" bash scripts/qemu-boot-log-test.sh

# USB-накопитель через VFS: mount fat32, ls, cat, запись (`cp`).
usb-storage-test: build
	@QEMU="$(QEMU)" bash scripts/qemu-usb-storage-test.sh

# GPT-образ флешки: ESP (VIBRA-ESP) + раздел для лога (VIBRALOG).
usb-debug: boot-log-kernel
	@USB_DEBUG_SIZE_MB="$(USB_DEBUG_SIZE_MB)" USB_DEBUG_ESP_MB="$(USB_DEBUG_ESP_MB)" \
		KERNEL_ELF="$(KERNEL_ELF)" bash scripts/build-usb-debug.sh

# Гибридный ISO (UEFI+BIOS) + приложенный раздел VIBRALOG (xorriso).
#
# Загрузка: UEFI — с CD и с флешки (ESP внутри GPT); BIOS — только по El Torito
# (CD). Гибридный MBR для BIOS-загрузки с флешки не ставится: limine bios-install
# требует отдельный BIOS boot partition, а GPT уже занят ESP и VIBRALOG.
# Для флешки на реальном железе есть `make usb-debug`.
iso-debug: boot-log-kernel
	@echo "==> Creating debug ISO with VIBRALOG partition..."
	@mkdir -p $(BUILD_DIR)
	@rm -rf $(BUILD_DIR)/iso_debug_root
	@mkdir -p $(BUILD_DIR)/iso_debug_root/limine
	@mkdir -p $(BUILD_DIR)/iso_debug_root/EFI/BOOT
	@cp $(KERNEL_ELF) $(BUILD_DIR)/iso_debug_root/kernel.elf
	@cp limine.conf $(BUILD_DIR)/iso_debug_root/limine/
	@cp $(LIMINE_DIR)/BOOTX64.EFI $(BUILD_DIR)/iso_debug_root/EFI/BOOT/
	@cp $(LIMINE_DIR)/limine-bios-cd.bin $(BUILD_DIR)/iso_debug_root/limine/
	@cp $(LIMINE_DIR)/limine-uefi-cd.bin $(BUILD_DIR)/iso_debug_root/limine/
# Без limine-bios.sys BIOS-ветка El Torito не находит stage2 — образ грузится
# только через UEFI. Файл должен лежать внутри ISO рядом с limine-bios-cd.bin.
	@cp $(LIMINE_DIR)/limine-bios.sys $(BUILD_DIR)/iso_debug_root/limine/
	@LOG_IMG=$$(bash scripts/build-log-partition.sh "$(LOG_PARTITION_MB)" "$(BUILD_DIR)/log-partition.img"); \
	xorriso -as mkisofs \
		-b limine/limine-bios-cd.bin -no-emul-boot -boot-load-size 4 -boot-info-table \
		--efi-boot limine/limine-uefi-cd.bin -efi-boot-part --efi-boot-image \
		--protective-msdos-label \
		-append_partition 2 0x0c "$$LOG_IMG" \
		-o $(BUILD_DIR)/vibra-debug.iso $(BUILD_DIR)/iso_debug_root
# Гибридный MBR (limine bios-install) здесь не ставим: у образа GPT
# (EFI-раздел + добавленный VIBRALOG), и limine требует отдельный BIOS boot
# partition. Для загрузки с CD достаточно El Torito (`-b limine-bios-cd.bin`
# + limine-bios.sys внутри ISO) — именно так грузится SeaBIOS.
	@echo "==> debug ISO: $(BUILD_DIR)/vibra-debug.iso"
	@echo "    GPT partition 4 = VIBRALOG ($(LOG_PARTITION_MB)MB, xorriso -append_partition 2)"
	@ls -lh $(BUILD_DIR)/vibra-debug.iso

# Прочитать лог с образа/флешки без монтирования: make read-boot-log LOG_DEVICE=/dev/sdX
read-boot-log:
	@bash scripts/read-boot-log.sh "$(LOG_DEVICE)"
