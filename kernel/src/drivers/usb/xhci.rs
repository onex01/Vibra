// xHCI — рабочий lifecycle для BSP: BIOS handoff, reset, DCBAA, command ring,
// event ring (ERST), перечисление портов и USB HID boot keyboard.
//
// Драйвер намеренно polling-based: во время boot прерывания ещё выключены,
// а на реальном железе MSI/MSI-X для xHCI требует отдельной инфраструктуры.
// `poll_keyboard()` вызывается из `keyboard::poll_key()` и добирает отчёты.
//
// Все ожидания ограничены по числу итераций: неотвечающий контроллер не должен
// вешать boot (это уже случалось с EHCI/BIOS handoff на ноутбуках).

use crate::memory::dma::DmaBuffer;
use crate::println;
use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

const TRB_SIZE: usize = 16;
const RING_TRBS: usize = 256;

// TRB types (xHCI 1.1, table 6-91)
const TRB_NORMAL: u32 = 1;
const TRB_SETUP_STAGE: u32 = 2;
const TRB_DATA_STAGE: u32 = 3;
const TRB_STATUS_STAGE: u32 = 4;
const TRB_LINK: u32 = 6;
const TRB_ENABLE_SLOT: u32 = 9;
const TRB_ADDRESS_DEVICE: u32 = 11;
const TRB_CONFIGURE_ENDPOINT: u32 = 12;
const TRB_TRANSFER_EVENT: u32 = 32;
const TRB_COMMAND_COMPLETION: u32 = 33;
const TRB_PORT_STATUS_CHANGE: u32 = 34;

const CC_SUCCESS: u32 = 1;
const CC_SHORT_PACKET: u32 = 13;

// Operational registers
const USBCMD: u64 = 0x00;
const USBSTS: u64 = 0x04;
const CRCR: u64 = 0x18;
const DCBAAP: u64 = 0x30;
const CONFIG: u64 = 0x38;

const USBCMD_RS: u32 = 1 << 0;
const USBCMD_HCRST: u32 = 1 << 1;
const USBSTS_HCH: u32 = 1 << 0;
const USBSTS_CNR: u32 = 1 << 11;

// Interrupter 0 (runtime)
const IR0_ERSTSZ: u64 = 0x28;
const IR0_ERSTBA: u64 = 0x30;
const IR0_ERDP: u64 = 0x38;

// PORTSC
const PORTSC_CCS: u32 = 1 << 0;
const PORTSC_PED: u32 = 1 << 1;
const PORTSC_PR: u32 = 1 << 4;
const PORTSC_PP: u32 = 1 << 9;
const PORTSC_SPEED_SHIFT: u32 = 10;
const PORTSC_PRC: u32 = 1 << 21;
const PORTSC_WPR: u32 = 1 << 31;
/// Write-1-to-clear bits (CSC..CEC).
const PORTSC_RW1C: u32 = 0x7F << 17;

const SPEED_FULL: u32 = 1;
const SPEED_LOW: u32 = 2;
const SPEED_HIGH: u32 = 3;

fn speed_name(speed: u32) -> &'static str {
    match speed {
        SPEED_FULL => "full",
        SPEED_LOW => "low",
        SPEED_HIGH => "high",
        4 => "super",
        _ => "unknown",
    }
}

#[inline]
unsafe fn rd32(addr: u64) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

#[inline]
unsafe fn wr32(addr: u64, value: u32) {
    core::ptr::write_volatile(addr as *mut u32, value);
}

#[inline]
unsafe fn wr64(addr: u64, value: u64) {
    wr32(addr, value as u32);
    wr32(addr + 4, (value >> 32) as u32);
}

/// Выделяет zeroed DMA-буфер и «забывает» его: буферы живут до перезагрузки,
/// а хранить `DmaBuffer` в статике нельзя (NonNull не Send).
fn alloc_dma(size: usize) -> Option<(u64, u64)> {
    let buffer = DmaBuffer::allocate(size)?;
    let phys = buffer.physical_address() as u64;
    let virt = buffer.as_ptr() as u64;
    core::mem::forget(buffer);
    Some((phys, virt))
}

/// Небольшая задержка без таймера (USB init идёт до timer::init).
fn delay(spins: u32) {
    for _ in 0..spins {
        core::hint::spin_loop();
    }
}

/// Кольцо TRB (command / event / endpoint).
struct Ring {
    phys: u64,
    virt: u64,
    /// Индекс следующего TRB для записи (producer) или чтения (consumer).
    index: usize,
    /// Producer/consumer Cycle State.
    cycle: bool,
}

impl Ring {
    fn new() -> Option<Ring> {
        let (phys, virt) = alloc_dma(RING_TRBS * TRB_SIZE)?;
        Some(Ring { phys, virt, index: 0, cycle: true })
    }

    unsafe fn write_trb(&self, index: usize, param: u64, status: u32, control: u32) {
        let base = (self.virt as *mut u32).add(index * 4);
        core::ptr::write_volatile(base, param as u32);
        core::ptr::write_volatile(base.add(1), (param >> 32) as u32);
        core::ptr::write_volatile(base.add(2), status);
        core::ptr::write_volatile(base.add(3), control);
    }

    unsafe fn read_trb(&self, index: usize) -> (u64, u32, u32) {
        let base = (self.virt as *const u32).add(index * 4);
        let lo = core::ptr::read_volatile(base) as u64;
        let hi = core::ptr::read_volatile(base.add(1)) as u64;
        let status = core::ptr::read_volatile(base.add(2));
        let control = core::ptr::read_volatile(base.add(3));
        ((hi << 32) | lo, status, control)
    }

    /// Инициализация Link TRB на последнем слоте (замыкает кольцо).
    unsafe fn init_link(&mut self) {
        self.write_trb(
            RING_TRBS - 1,
            self.phys,
            0,
            (TRB_LINK << 10) | (1 << 1) | self.cycle as u32, // TC=1
        );
    }

    /// Записывает TRB с текущим cycle bit и сдвигает producer.
    unsafe fn push(&mut self, param: u64, status: u32, control: u32) {
        self.write_trb(self.index, param, status, control | self.cycle as u32);
        self.index += 1;
        if self.index >= RING_TRBS - 1 {
            // Link TRB завершает сегмент и должен нести ТЕКУЩИЙ cycle state:
            // контроллер проверит его тем же CCS, с которым читал сегмент,
            // и только потом переключит свой Cycle State.
            let link = (self.virt as *mut u32).add((RING_TRBS - 1) * 4 + 3);
            let link_control = core::ptr::read_volatile(link);
            core::ptr::write_volatile(link, (link_control & !1) | self.cycle as u32);
            self.index = 0;
            self.cycle = !self.cycle;
        }
    }
}

#[derive(Clone, Copy)]
struct Event {
    param: u64,
    status: u32,
    control: u32,
}

impl Event {
    fn trb_type(&self) -> u32 {
        (self.control >> 10) & 0x3F
    }
    fn completion_code(&self) -> u32 {
        (self.status >> 24) & 0xFF
    }
    fn slot(&self) -> u8 {
        ((self.control >> 24) & 0xFF) as u8
    }
    fn endpoint(&self) -> u8 {
        ((self.control >> 16) & 0x1F) as u8
    }
}

struct HidKeyboard {
    slot: u8,
    /// Device Context Index (EP1 IN = 3).
    dci: u8,
    ring: Ring,
    report_phys: u64,
    report_virt: u64,
    max_packet: u16,
    pending: bool,
    reports: u32,
}

struct Controller {
    op: u64,
    rt: u64,
    db: u64,
    max_ports: u8,
    dcbaa_virt: u64,
    cmd: Ring,
    ev: Ring,
    keys: Vec<HidKeyboard>,
    /// Сколько страниц scratchpad зарезервировано (нужно для Address Device).
    scratchpad_pages: usize,
    /// Перечисление портов отложено до первого опроса клавиатуры.
    enumerated: bool,
}

impl Controller {
    fn port_reg(&self, port: u8) -> u64 {
        self.op + 0x400 + (port as u64 - 1) * 0x10
    }

    unsafe fn doorbell(&self, slot: u8, target: u32) {
        wr32(self.db + (slot as u64) * 4, target);
    }

    /// Запись PORTSC: PED (RW1CS) нельзя выставлять в 1 — это отключит порт.
    unsafe fn port_write(&self, port: u8, value: u32) {
        wr32(self.port_reg(port), value & !PORTSC_PED);
    }

    unsafe fn port_clear_changes(&self, port: u8) {
        let value = rd32(self.port_reg(port));
        self.port_write(port, value & !PORTSC_PR);
    }

    /// Читает следующее событие из event ring.
    unsafe fn next_event(&mut self) -> Option<Event> {
        let (param, status, control) = self.ev.read_trb(self.ev.index);
        if (control & 1) != self.ev.cycle as u32 {
            return None;
        }
        self.ev.index += 1;
        if self.ev.index >= RING_TRBS {
            self.ev.index = 0;
            self.ev.cycle = !self.ev.cycle;
        }
        // ERDP: dequeue pointer + EHB=0
        let erdp = self.ev.phys + (self.ev.index as u64) * TRB_SIZE as u64;
        wr64(self.rt + IR0_ERDP, erdp);
        Some(Event { param, status, control })
    }

    /// Отправляет команду в command ring и ждёт Command Completion Event.
    unsafe fn command(&mut self, param: u64, trb_type: u32, slot: u8) -> Option<Event> {
        self.cmd.push(param, 0, (trb_type << 10) | ((slot as u32) << 24));
        self.doorbell(0, 0);

        for _ in 0..2_000_000 {
            if let Some(event) = self.next_event() {
                match event.trb_type() {
                    TRB_COMMAND_COMPLETION => {
                        if slot == 0 || event.slot() == slot {
                            return Some(event);
                        }
                    }
                    TRB_PORT_STATUS_CHANGE => {
                        // Во время boot порты могут рапортовать об изменениях.
                        // Читаем и очищаем, чтобы событие не копилось.
                        let port = ((event.param >> 24) & 0xFF) as u8;
                        if port >= 1 && port <= self.max_ports {
                            self.port_clear_changes(port);
                        }
                    }
                    _ => {}
                }
            }
        }
        None
    }

    /// Ждёт Transfer Event для указанного slot/DCI.
    unsafe fn wait_transfer(&mut self, slot: u8, dci: u8, limit: u32) -> Option<u32> {
        for _ in 0..limit {
            if let Some(event) = self.next_event() {
                if event.trb_type() == TRB_TRANSFER_EVENT
                    && event.slot() == slot
                    && event.endpoint() == dci
                {
                    return Some(event.completion_code());
                }
            }
        }
        None
    }
}

/// Control transfer на EP0. `data` — (буфер, физический адрес буфера).
/// Возвращает длину переданных данных при успехе.
unsafe fn control_transfer(
    ctrl: &mut Controller,
    slot: u8,
    ring: *mut Ring,
    bm: u8,
    request: u8,
    value: u16,
    index: u16,
    data: Option<(&mut [u8], u64)>,
) -> Option<usize> {
    let ring = &mut *ring;
    let len = data.as_ref().map(|(buffer, _)| buffer.len()).unwrap_or(0);
    let dir_in = bm & 0x80 != 0;

    let setup = (bm as u64)
        | ((request as u64) << 8)
        | ((value as u64) << 16)
        | ((index as u64) << 32)
        | ((len as u64) << 48);
    // IDT (bit 6) обязателен для Setup Stage.
    ring.push(setup, 8, (TRB_SETUP_STAGE << 10) | (1 << 6));

    if let Some((_, phys)) = data.as_ref() {
        if len > 0 {
            let control = (TRB_DATA_STAGE << 10) | if dir_in { 1 << 16 } else { 0 };
            ring.push(*phys, len as u32, control);
        }
    }

    // Status stage идёт в противоположном направлении к data stage.
    let status_dir = if len == 0 || !dir_in { 1 << 16 } else { 0 };
    ring.push(0, 0, (TRB_STATUS_STAGE << 10) | (1 << 5) | status_dir);

    ctrl.doorbell(slot, 1);
    let code = ctrl.wait_transfer(slot, 1, 4_000_000)?;
    if code != CC_SUCCESS && code != CC_SHORT_PACKET {
        return None;
    }
    Some(len)
}

unsafe fn get_descriptor(
    ctrl: &mut Controller,
    slot: u8,
    ring: *mut Ring,
    desc_type: u8,
    desc_index: u8,
    buf: &mut [u8],
    phys: u64,
) -> Option<usize> {
    control_transfer(
        ctrl,
        slot,
        ring,
        0x80,
        6, // GET_DESCRIPTOR
        ((desc_type as u16) << 8) | desc_index as u16,
        0,
        Some((buf, phys)),
    )
    .filter(|received| *received >= 8)
}

#[derive(Clone, Copy)]
struct EndpointInfo {
    address: u8,
    max_packet: u16,
    interval: u8,
}

/// Ищет HID-интерфейс клавиатуры и его interrupt IN endpoint.
fn find_hid_keyboard(config: &[u8]) -> Option<(u8, EndpointInfo)> {
    let mut offset = 0usize;
    let mut current: Option<(u8, bool)> = None;

    while offset + 2 <= config.len() {
        let len = config[offset] as usize;
        let dtype = config[offset + 1];
        if len < 2 || offset + len > config.len() {
            break;
        }

        match dtype {
            4 if len >= 9 => {
                let number = config[offset + 2];
                let class = config[offset + 5];
                let subclass = config[offset + 6];
                let protocol = config[offset + 7];
                // class 3 = HID; boot interface (subclass 1) или protocol 1 = keyboard.
                let is_keyboard = class == 3
                    && (subclass == 1 || protocol == 1)
                    && protocol <= 1;
                current = Some((number, is_keyboard));
            }
            5 if len >= 7 => {
                if let Some((number, is_keyboard)) = current {
                    let address = config[offset + 2];
                    let attributes = config[offset + 3];
                    // bit 7 = IN, bits 0-1 = 3 (interrupt)
                    if is_keyboard && address & 0x80 != 0 && attributes & 0x03 == 3 {
                        let max_packet = u16::from_le_bytes([config[offset + 4], config[offset + 5]])
                            & 0x07FF;
                        return Some((
                            number,
                            EndpointInfo {
                                address,
                                max_packet,
                                interval: config[offset + 6],
                            },
                        ));
                    }
                }
            }
            _ => {}
        }

        offset += len;
    }
    None
}

/// Заполняет input context для Address Device (slot + EP0).
unsafe fn fill_address_context(
    in_virt: u64,
    speed: u32,
    port: u8,
    max_packet: u16,
    ep0_phys: u64,
) {
    // Input Control Context: add slot (bit 0) + EP0 (bit 1)
    wr32(in_virt + 0x04, 0x3);

    // Slot Context (0x20)
    wr32(in_virt + 0x20, (speed << 20) | (1 << 27));
    wr32(in_virt + 0x24, (port as u32) << 16);

    // EP0 Context (0x40)
    let ep0 = in_virt + 0x40;
    wr32(ep0 + 0x00, 0);
    wr32(ep0 + 0x04, (3 << 1) | (4 << 3) | ((max_packet as u32) << 16));
    wr32(ep0 + 0x08, (ep0_phys as u32) | 1);
    wr32(ep0 + 0x0C, (ep0_phys >> 32) as u32);
    wr32(ep0 + 0x10, 8);
}

/// Configure Endpoint для interrupt IN клавиатуры.
unsafe fn configure_keyboard_endpoint(
    ctrl: &mut Controller,
    slot: u8,
    speed: u32,
    port: u8,
    dci: u8,
    max_packet: u16,
    interval: u8,
    ring_phys: u64,
) -> bool {
    let Some((in_phys, in_virt)) = alloc_dma(4096) else {
        return false;
    };

    wr32(in_virt + 0x04, (1 << 0) | (1 << dci));

    // Slot Context: context entries = DCI
    wr32(in_virt + 0x20, (speed << 20) | ((dci as u32) << 27));
    wr32(in_virt + 0x24, (port as u32) << 16);

    let ep = in_virt + 0x20 + (dci as u64) * 0x20;
    wr32(ep + 0x00, (interval as u32) << 16);
    wr32(ep + 0x04, (3 << 1) | (7 << 3) | ((max_packet as u32) << 16)); // Interrupt IN
    wr32(ep + 0x08, (ring_phys as u32) | 1);
    wr32(ep + 0x0C, (ring_phys >> 32) as u32);
    wr32(ep + 0x10, 8);

    match ctrl.command(in_phys, TRB_CONFIGURE_ENDPOINT, slot) {
        Some(event) => event.completion_code() == CC_SUCCESS,
        None => false,
    }
}

impl Controller {
    /// Сброс порта. Возвращает скорость (PORTSC bits 10-13).
    unsafe fn reset_port(&mut self, port: u8) -> Option<u32> {
        let reg = self.port_reg(port);
        let portsc = rd32(reg);
        if portsc & PORTSC_CCS == 0 {
            return None;
        }

        if portsc & PORTSC_PP == 0 {
            self.port_write(port, (portsc & !PORTSC_RW1C) | PORTSC_PP);
            delay(20_000);
        }

        // USB2-порт сбрасывается PR, USB3 — WPR. Пробуем оба: контроллер
        // игнорирует неподдерживаемый бит, а таймаут ограничен.
        for attempt in 0..2 {
            let reset_bit = if attempt == 0 { PORTSC_PR } else { PORTSC_WPR };
            let portsc = rd32(reg);
            self.port_write(
                port,
                (portsc & !(PORTSC_RW1C | PORTSC_PR | PORTSC_WPR)) | reset_bit,
            );

            let mut completed = false;
            for _ in 0..2_000_000 {
                let value = rd32(reg);
                if value & PORTSC_PRC != 0 || (value & PORTSC_PR == 0 && value & PORTSC_PED != 0) {
                    completed = true;
                    break;
                }
                delay(50);
            }

            if completed {
                self.port_clear_changes(port);
                let value = rd32(reg);
                if value & PORTSC_PED != 0 {
                    return Some((value >> PORTSC_SPEED_SHIFT) & 0xF);
                }
            }
        }

        None
    }
}

/// Evaluate Context: обновляет EP0 Max Packet Size после чтения
/// bMaxPacketSize0 из первых 8 байт device descriptor.
unsafe fn evaluate_ep0(
    ctrl: &mut Controller,
    slot: u8,
    speed: u32,
    port: u8,
    max_packet: u16,
    ep0_phys: u64,
) -> bool {
    let Some((in_phys, in_virt)) = alloc_dma(4096) else {
        return false;
    };

    wr32(in_virt + 0x04, 0x3); // slot + EP0
    wr32(in_virt + 0x20, (speed << 20) | (1 << 27));
    wr32(in_virt + 0x24, (port as u32) << 16);

    let ep0 = in_virt + 0x40;
    wr32(ep0 + 0x04, (3 << 1) | (4 << 3) | ((max_packet as u32) << 16));
    wr32(ep0 + 0x08, (ep0_phys as u32) | 1);
    wr32(ep0 + 0x0C, (ep0_phys >> 32) as u32);
    wr32(ep0 + 0x10, 8);

    match ctrl.command(in_phys, 13, slot) {
        Some(event) => event.completion_code() == CC_SUCCESS,
        None => false,
    }
}

/// Перечисляет устройство на порту. Клавиатуры складываются в `ctrl.keys`.
unsafe fn enumerate_device(ctrl: &mut Controller, port: u8, speed: u32) {
    let Some(event) = ctrl.command(0, TRB_ENABLE_SLOT, 0) else {
        println!("[xHCI] port {}: Enable Slot timeout", port);
        return;
    };
    if event.completion_code() != CC_SUCCESS {
        println!("[xHCI] port {}: Enable Slot failed (cc={})", port, event.completion_code());
        return;
    }
    let slot = event.slot();
    if slot == 0 {
        return;
    }

    let Some((dev_phys, _)) = alloc_dma(4096) else { return };
    let Some((in_phys, in_virt)) = alloc_dma(4096) else { return };
    let Some(mut ep0) = Ring::new() else { return };
    ep0.init_link();
    let ep0_ring = &mut ep0 as *mut Ring;

    // DCBAA[slot] → device context
    wr64(ctrl.dcbaa_virt + (slot as u64) * 8, dev_phys);

    // Начальный EP0 max packet: 64 только для HS/Super, иначе 8 (USB 2.0:
    // FS-устройства по умолчанию используют 8-байтовые пакеты).
    let max_packet: u16 = if speed == SPEED_HIGH || speed == 4 { 64 } else { 8 };
    fill_address_context(in_virt, speed, port, max_packet, ep0.phys);

    let Some(event) = ctrl.command(in_phys, TRB_ADDRESS_DEVICE, slot) else {
        println!("[xHCI] slot {}: Address Device timeout", slot);
        return;
    };
    if event.completion_code() != CC_SUCCESS {
        println!("[xHCI] slot {}: Address Device failed (cc={})", slot, event.completion_code());
        return;
    }

    let Some((buf_phys, buf_virt)) = alloc_dma(512) else { return };
    let buf = core::slice::from_raw_parts_mut(buf_virt as *mut u8, 512);

    // Первые 8 байт device descriptor: bMaxPacketSize0 лежит в offset 7.
    if get_descriptor(ctrl, slot, ep0_ring, 1, 0, &mut buf[..8], buf_phys).is_none() {
        println!("[xHCI] slot {}: GET_DESCRIPTOR(device, 8) failed", slot);
        return;
    }
    let device_max_packet = buf[7] as u16;
    if device_max_packet != max_packet && (8..=64).contains(&device_max_packet) {
        // Evaluate Context применяет новый размер в контроллере; локальную
        // переменную обновлять не нужно — дальше она не используется.
        if !evaluate_ep0(ctrl, slot, speed, port, device_max_packet, ep0.phys) {
            println!(
                "[xHCI] slot {}: Evaluate Context failed, keeping EP0 mps={}",
                slot, max_packet
            );
        }
    }

    // Device descriptor (18 bytes)
    if get_descriptor(ctrl, slot, ep0_ring, 1, 0, &mut buf[..18], buf_phys).is_none() {
        println!("[xHCI] slot {}: GET_DESCRIPTOR(device) failed", slot);
        return;
    }
    let vendor = u16::from_le_bytes([buf[8], buf[9]]);
    let product = u16::from_le_bytes([buf[10], buf[11]]);
    let num_configs = buf[17];

    if num_configs == 0 {
        return;
    }

    // Configuration descriptor: сначала 9 байт, затем целиком.
    if get_descriptor(ctrl, slot, ep0_ring, 2, 0, &mut buf[..9], buf_phys).is_none() {
        println!("[xHCI] slot {}: GET_DESCRIPTOR(config header) failed", slot);
        return;
    }
    let total = u16::from_le_bytes([buf[2], buf[3]]) as usize;
    if total < 9 || total > 512 {
        return;
    }
    if get_descriptor(ctrl, slot, ep0_ring, 2, 0, &mut buf[..total], buf_phys).is_none() {
        println!("[xHCI] slot {}: GET_DESCRIPTOR(config) failed", slot);
        return;
    }

    let config_value = buf[5];
    let Some((interface, endpoint)) = find_hid_keyboard(&buf[..total]) else {
        println!(
            "[xHCI] slot {}: device {:04x}:{:04x} is not a boot keyboard",
            slot, vendor, product
        );
        return;
    };

    // SET_CONFIGURATION
    if control_transfer(ctrl, slot, ep0_ring, 0x00, 9, config_value as u16, 0, None).is_none() {
        println!("[xHCI] slot {}: SET_CONFIGURATION failed", slot);
        return;
    }

    // HID boot protocol + idle rate 0 (report only on change)
    let _ = control_transfer(ctrl, slot, ep0_ring, 0x21, 0x0B, 0, interface as u16, None);
    let _ = control_transfer(ctrl, slot, ep0_ring, 0x21, 0x0A, 0, interface as u16, None);

    let Some(mut ring) = Ring::new() else { return };
    ring.init_link();
    // Буфер с запасом: wMaxPacketSize может быть больше 8 байт.
    let report_len: u16 = if endpoint.max_packet == 0 {
        8
    } else {
        endpoint.max_packet.min(64)
    };
    let Some((report_phys, report_virt)) = alloc_dma(64) else { return };

    // DCI: EP number * 2 + 1 (IN)
    let dci = ((endpoint.address & 0x0F) << 1) | 1;
    if !configure_keyboard_endpoint(
        ctrl,
        slot,
        speed,
        port,
        dci,
        endpoint.max_packet,
        endpoint.interval,
        ring.phys,
    ) {
        println!("[xHCI] slot {}: Configure Endpoint failed", slot);
        return;
    }

    // Первый interrupt IN transfer.
    ring.push(report_phys, report_len as u32, (TRB_NORMAL << 10) | (1 << 5));
    ctrl.doorbell(slot, dci as u32);

    ctrl.keys.push(HidKeyboard {
        slot,
        dci,
        ring,
        report_phys,
        report_virt,
        max_packet: report_len,
        pending: true,
        reports: 0,
    });

    println!(
        "[xHCI] HID keyboard ready: slot {} port {} {:04x}:{:04x} ep=0x{:02x} mps={} interval={}",
        slot, port, vendor, product, endpoint.address, endpoint.max_packet, endpoint.interval
    );
}

static XHCI: Mutex<Vec<Controller>> = Mutex::new(Vec::new());

/// Опрашивает event ring всех контроллеров и разбирает HID-отчёты.
/// Вызывается из `keyboard::poll_key()`.
pub fn poll_keyboard() {
    let mut controllers = match XHCI.try_lock() {
        Some(guard) => guard,
        None => return,
    };

    for ctrl in controllers.iter_mut() {
        // Ленивое перечисление: boot не ждёт USB, а первый опрос клавиатуры
        // (уже после shell) донастраивает устройства.
        if !ctrl.enumerated {
            ctrl.enumerated = true;
            unsafe { enumerate_ports(ctrl) };
        }

        let db = ctrl.db;
        let mut budget = 0;
        loop {
            let Some(event) = (unsafe { ctrl.next_event() }) else { break };
            budget += 1;
            if budget > 64 {
                break;
            }

            match event.trb_type() {
                TRB_TRANSFER_EVENT => {
                    let slot = event.slot();
                    let dci = event.endpoint();
                    let code = event.completion_code();

                    let Some(keyboard) = ctrl
                        .keys
                        .iter_mut()
                        .find(|k| k.slot == slot && k.dci == dci)
                    else {
                        continue;
                    };
                    if !keyboard.pending {
                        continue;
                    }

                    if code == CC_SUCCESS || code == CC_SHORT_PACKET {
                        let report = unsafe {
                            core::slice::from_raw_parts(keyboard.report_virt as *const u8, 8)
                        };
                        crate::drivers::usb::hid::handle_boot_report(report);
                        keyboard.reports = keyboard.reports.wrapping_add(1);
                    }

                    // Переотправляем TRB, иначе endpoint остановится после отчёта.
                    unsafe {
                        keyboard.ring.push(
                            keyboard.report_phys,
                            keyboard.max_packet as u32,
                            (TRB_NORMAL << 10) | (1 << 5),
                        );
                        wr32(db + (keyboard.slot as u64) * 4, keyboard.dci as u32);
                    }
                }
                TRB_PORT_STATUS_CHANGE => {
                    let port = ((event.param >> 24) & 0xFF) as u8;
                    if port >= 1 && port <= ctrl.max_ports {
                        unsafe { ctrl.port_clear_changes(port) };
                    }
                }
                _ => {}
            }
        }
    }
}

/// Краткий статус для `diag keyboard`.
pub fn keyboard_status() -> String {
    let controllers = XHCI.lock();
    if controllers.is_empty() {
        return String::from("no xHCI controller");
    }

    let mut text = String::new();
    for (index, ctrl) in controllers.iter().enumerate() {
        if index > 0 {
            text.push_str("; ");
        }
        text.push_str(&alloc::format!(
            "ctrl{} {} keyboard(s)",
            index,
            ctrl.keys.len()
        ));
        for keyboard in &ctrl.keys {
            text.push_str(&alloc::format!(
                " [slot {} dci {} reports {}]",
                keyboard.slot, keyboard.dci, keyboard.reports
            ));
        }
    }
    text
}

/// BIOS → OS handoff для USB Legacy Support (extended capability ID 1).
unsafe fn bios_handoff(mmio: u64, hccparams1: u32) {
    let mut offset = (((hccparams1 >> 16) & 0xFFFF) as u64) * 4;
    let mut guard = 0;

    while offset != 0 && guard < 32 {
        let capability = rd32(mmio + offset);
        let id = capability & 0xFF;
        let next = (((capability >> 8) & 0xFF) as u64) * 4;

        if id == 1 {
            let legsup = mmio + offset;
            let mut value = rd32(legsup);
            if value & (1 << 16) != 0 {
                // BIOS владеет контроллером — забираем ownership.
                value |= 1 << 24;
                wr32(legsup, value);
                for _ in 0..2_000_000 {
                    value = rd32(legsup);
                    if value & (1 << 16) == 0 {
                        break;
                    }
                    core::hint::spin_loop();
                }
                // Отключаем SMI, иначе прошивка продолжит вмешиваться.
                wr32(legsup + 4, 0);
            }
            println!(
                "[xHCI] legacy handoff: bios_owned={} os_owned={}",
                (value >> 16) & 1,
                (value >> 24) & 1
            );
            return;
        }

        if next == 0 {
            break;
        }
        offset += next;
        guard += 1;
    }
}

/// Полная инициализация контроллера и перечисление портов.
pub fn init_controller(bar_phys: u64, mmio_virt: u64) -> bool {
    unsafe {
        let cap0 = rd32(mmio_virt);
        let cap_len = (cap0 & 0xFF) as u64;
        let hci_version = (cap0 >> 16) as u16;

        if (hci_version >> 8) != 1 {
            println!(
                "[xHCI] {:#x}: interface version {}.{} is not xHCI 1.x",
                bar_phys,
                hci_version >> 8,
                hci_version & 0xFF
            );
            return false;
        }

        let hcsparams1 = rd32(mmio_virt + 0x04);
        let hcsparams2 = rd32(mmio_virt + 0x08);
        let hccparams1 = rd32(mmio_virt + 0x10);
        let dboff = rd32(mmio_virt + 0x14) as u64;
        let rtsoff = rd32(mmio_virt + 0x18) as u64;

        let max_slots = (hcsparams1 & 0xFF) as u8;
        let max_ports = ((hcsparams1 >> 24) & 0xFF) as u8;
        if max_slots == 0 || max_ports == 0 {
            println!("[xHCI] {:#x}: invalid topology (slots={}, ports={})", bar_phys, max_slots, max_ports);
            return false;
        }

        let op = mmio_virt + cap_len;
        let rt = mmio_virt + rtsoff;
        let db = mmio_virt + dboff;

        println!(
            "[xHCI] {:#x}: v{}.{} slots={} ports={} DBOFF={:#x} RTSOFF={:#x}",
            bar_phys,
            hci_version >> 8,
            hci_version & 0xFF,
            max_slots,
            max_ports,
            dboff,
            rtsoff
        );

        bios_handoff(mmio_virt, hccparams1);

        // 1. Останавливаем контроллер, если он запущен.
        if rd32(op + USBSTS) & USBSTS_HCH == 0 {
            wr32(op + USBCMD, rd32(op + USBCMD) & !USBCMD_RS);
            for _ in 0..2_000_000 {
                if rd32(op + USBSTS) & USBSTS_HCH != 0 {
                    break;
                }
                core::hint::spin_loop();
            }
        }

        // 2. Reset
        wr32(op + USBCMD, rd32(op + USBCMD) | USBCMD_HCRST);
        let mut reset_ok = false;
        for _ in 0..4_000_000 {
            if rd32(op + USBCMD) & USBCMD_HCRST == 0 {
                reset_ok = true;
                break;
            }
            core::hint::spin_loop();
        }
        if !reset_ok {
            println!("[xHCI] {:#x}: reset timeout (HCRST stuck)", bar_phys);
            return false;
        }
        for _ in 0..4_000_000 {
            if rd32(op + USBSTS) & USBSTS_CNR == 0 {
                break;
            }
            core::hint::spin_loop();
        }

        // 3. DCBAA + scratchpad buffers (нужны для Address Device на реальном
        //    железе, QEMU обычно обходится без них).
        let Some((dcbaa_phys, dcbaa_virt)) = alloc_dma(4096) else {
            println!("[xHCI] {:#x}: no memory for DCBAA", bar_phys);
            return false;
        };
        wr64(op + DCBAAP, dcbaa_phys);

        let scratchpad_count =
            (((hcsparams2 >> 21) & 0x1F) | (((hcsparams2 >> 27) & 0x1F) << 5)) as usize;
        if scratchpad_count > 0 {
            if let Some((array_phys, array_virt)) = alloc_dma(scratchpad_count * 8) {
                let mut allocated = 0;
                for index in 0..scratchpad_count {
                    match alloc_dma(4096) {
                        Some((phys, _)) => {
                            wr64(array_virt + (index as u64) * 8, phys);
                            allocated += 1;
                        }
                        None => break,
                    }
                }
                if allocated == scratchpad_count {
                    wr64(dcbaa_virt, array_phys);
                    println!("[xHCI] scratchpad: {} page(s)", scratchpad_count);
                } else {
                    println!(
                        "[xHCI] scratchpad: only {}/{} page(s) allocated",
                        allocated, scratchpad_count
                    );
                }
            }
        }

        // 4. Command ring
        let Some(mut cmd) = Ring::new() else { return false };
        cmd.init_link();
        wr64(op + CRCR, cmd.phys | 1);

        // 5. Event ring + ERST
        let Some(ev) = Ring::new() else { return false };
        let Some((erst_phys, erst_virt)) = alloc_dma(4096) else { return false };
        wr64(erst_virt, ev.phys);
        wr32(erst_virt + 8, RING_TRBS as u32);
        wr32(erst_virt + 12, 0);
        wr32(rt + IR0_ERSTSZ, 1);
        wr64(rt + IR0_ERSTBA, erst_phys);
        wr64(rt + IR0_ERDP, ev.phys);

        // 6. CONFIG + Run
        wr32(op + CONFIG, max_slots as u32);
        wr32(op + USBSTS, rd32(op + USBSTS)); // clear pending status bits
        wr32(op + USBCMD, rd32(op + USBCMD) | USBCMD_RS);
        let mut running = false;
        for _ in 0..4_000_000 {
            if rd32(op + USBSTS) & USBSTS_HCH == 0 {
                running = true;
                break;
            }
            core::hint::spin_loop();
        }
        if !running {
            println!("[xHCI] {:#x}: controller did not start", bar_phys);
            return false;
        }

        let controller = Controller {
            op,
            rt,
            db,
            max_ports,
            dcbaa_virt,
            cmd,
            ev,
            keys: Vec::new(),
            scratchpad_pages: scratchpad_count,
            enumerated: false,
        };

        // Перечисление портов отложено до первого опроса клавиатуры:
        // на реальном железе оно занимает секунды и не должно задерживать boot.
        println!(
            "[xHCI] {:#x}: running, enumeration deferred (scratchpad {} page(s))",
            bar_phys, controller.scratchpad_pages
        );
        XHCI.lock().push(controller);
        true
    }
}

/// Перечисляет подключённые порты и настраивает найденные HID-клавиатуры.
unsafe fn enumerate_ports(ctrl: &mut Controller) {
    for port in 1..=ctrl.max_ports {
        let portsc = rd32(ctrl.port_reg(port));
        if portsc & PORTSC_CCS == 0 {
            continue;
        }
        ctrl.port_clear_changes(port);
        match ctrl.reset_port(port) {
            Some(speed) => {
                println!(
                    "[xHCI] port {}: connected (speed {} = {})",
                    port,
                    speed,
                    speed_name(speed)
                );
                enumerate_device(ctrl, port, speed);
            }
            None => println!("[xHCI] port {}: reset failed", port),
        }
    }
}

