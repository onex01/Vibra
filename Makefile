KERNEL_NAME := vibra
TARGET := x86_64-unknown-none
BUILD_DIR := build
LIMINE_DIR := limine-bin

QEMU := qemu-system-x86_64
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
else
PROFILE := debug
endif

# COM1 привязан к stdin/stdout: можно вводить shell-команды из терминала,
# сохраняя графическое окно QEMU для framebuffer-консоли.
QEMU_FLAGS := -m 256M -serial stdio -monitor none -drive if=pflash,format=raw,file=OVMF.fd,readonly=on \
              -drive file=$(BUILD_DIR)/hdd.img,format=raw -M q35

.PHONY: all build run clean setup install-kernel

all: run

# Сборка ядра И копирование его на диск
build:
	@mkdir -p $(BUILD_DIR)
	$(CARGO) build $(CARGO_FEATURES) -Z build-std=core,alloc,compiler_builtins -Z build-std-features
	@echo "==> Copying kernel to disk image..."
	@mcopy -o -i $(BUILD_DIR)/hdd.img $(shell pwd)/target/$(TARGET)/debug/$(KERNEL_NAME) ::/kernel.elf
	@echo "==> Kernel built and installed!"

setup:
	@echo "==> Setting up boot image..."
	@mkdir -p $(BUILD_DIR)
	@rm -rf $(LIMINE_DIR)
	
	@echo "==> Downloading Limine precompiled binaries..."
	@wget -q https://github.com/limine-bootloader/limine/releases/latest/download/limine-binary.tar.xz
	@tar -xf limine-binary.tar.xz
	@mv limine-binary $(LIMINE_DIR)
	@rm limine-binary.tar.xz

	@if [ ! -f "OVMF.fd" ]; then \
		echo "==> Downloading UEFI BIOS (OVMF)..."; \
		wget -q https://retrage.github.io/edk2-nightly/bin/RELEASEX64_OVMF.fd -O OVMF.fd; \
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
	@mkfs.fat -F 32 $(BUILD_DIR)/hdd.img
	
	@echo "==> Copying bootloader files to disk image..."
	@mmd -i $(BUILD_DIR)/hdd.img ::/EFI ::/EFI/BOOT ::/limine
	@mcopy -i $(BUILD_DIR)/hdd.img $(LIMINE_DIR)/limine-bios.sys ::/limine/ || true
	@mcopy -i $(BUILD_DIR)/hdd.img $(LIMINE_DIR)/BOOTX64.EFI ::/EFI/BOOT/
	@mcopy -i $(BUILD_DIR)/hdd.img limine.conf ::/limine/
	
	@echo "==> Verifying disk contents:"
	@mdir -i $(BUILD_DIR)/hdd.img ::/
	@echo "==> Setup complete! Now run 'make build' to compile and install the kernel."

# Быстрое обновление только ядра (без пересборки)
install-kernel:
	@echo "==> Installing kernel to disk image..."
	@mcopy -o -i $(BUILD_DIR)/hdd.img $(shell pwd)/target/$(TARGET)/release/$(KERNEL_NAME) ::/kernel.elf
	@echo "==> Kernel installed!"

# Target for optimized release builds without serial shell
run-release: build-release
	@echo "==> Starting QEMU (release, no serial shell)..."
	@$(CARGO) build --release --no-default-features -Z build-std=core,alloc,compiler_builtins -Z build-std-features
	@mcopy -o -i $(BUILD_DIR)/hdd.img $(shell pwd)/target/$(TARGET)/release/$(KERNEL_NAME) ::/kernel.elf
	$(QEMU) -m 256M -serial none -monitor none -drive if=pflash,format=raw,file=OVMF.fd,readonly=on \
	         -drive file=$(BUILD_DIR)/hdd.img,format=raw -M q35

build-release:
	@$(CARGO) build --release --no-default-features -Z build-std=core,alloc,compiler_builtins -Z build-std-features
	@echo "==> Copying release kernel to disk image..."
	@mcopy -o -i $(BUILD_DIR)/hdd.img $(shell pwd)/target/$(TARGET)/release/$(KERNEL_NAME) ::/kernel.elf
	@echo "==> Release kernel built and installed!"

run: build
	@echo "==> Starting QEMU..."
	$(QEMU) $(QEMU_FLAGS)

clean:
	$(CARGO) clean
	rm -rf $(BUILD_DIR) $(LIMINE_DIR) OVMF.fd limine-binary.tar.xz
