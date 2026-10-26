// xHCI — рабочий lifecycle для BSP: BIOS handoff, reset, DCBAA, command ring,
// event ring (ERST), перечисление портов, USB HID boot keyboard и USB
// mass storage (bulk endpoints).
//
// Драйвер намеренно polling-based: во время boot прерывания ещё выключены,
// а на реальном железе MSI/MSI-X для xHCI требует отдельной инфраструктуры.
// `poll_events()` вызывается из `keyboard::poll_key()` и добирает события;
// `bulk_transfer()` / `control()` — синхронные, с ожиданием Transfer Event,
// при этом «чужие» события (отчёты клавиатуры) не теряются.
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
const TRB_EVALUATE_CONTEXT: u32 = 13;
const TRB_TRANSFER_EVENT: u32 = 32;
const TRB_COMMAND_COMPLETION: u32 = 33;
const TRB_PORT_STATUS_CHANGE: u32 = 34;

// Completion codes
const CC_SUCCESS: u32 = 1;
const CC_TRB_ERROR: u32 = 5;
const CC_STALL: u32 = 6;
const CC_SHORT_PACKET: u32 = 13;

// Endpoint types (xHCI 6.2.3, table 6-7)
const EP_TYPE_BULK_OUT: u32 = 2;
const EP_TYPE_INTERRUPT_IN: u32 = 7;
const EP_TYPE_BULK_IN: u32 = 6;

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

/// Максимум данных в одном bulk TRB (17-битное поле длины TRB).
const MAX_BULK_CHUNK: usize = 32 * 1024;

fn speed_name(speed: u32) -> &'static str {
    match speed {
        SPEED_FULL => "full",
        SPEED_LOW => "low",
        SPEED_HIGH => "high",
        4 => "super",
        _ => "unknown",
    }
}

/// Ошибки HCD-уровня. Классовые драйверы (storage) переводят их в `FsError`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsbError {
    /// Устройство/контроллер исчезли (или слот не найден).
    NoDevice,
    /// Transfer/command не завершился за отведённый бюджет итераций.
    Timeout,
    /// Endpoint ответил STALL.
    Stall,
    /// Ненулевой completion code.
    TransferFailed,
    /// Не удалось выделить DMA-буфер.
    NoMemory,
    /// Запрос не поддержан драйвером.
    Unsupported,
}

/// Описание найденного USB-устройства (для `diag usb` и реестра устройств).
#[derive(Clone)]
pub struct UsbDeviceInfo {
    pub controller: usize,
    pub slot: u8,
    pub port: u8,
    pub speed: u32,
    pub vendor: u16,
    pub product: u16,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    /// `keyboard`, `storage` или `unsupported`.
    pub kind: &'static str,
}

/// Параметры mass storage устройства, которое можно использовать как диск.
#[derive(Clone, Copy)]
pub struct StorageInfo {
    pub controller: usize,
    pub slot: u8,
    pub port: u8,
    pub vendor: u16,
    pub product: u16,
    pub max_transfer: usize,
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
    /// Остаток непереданных байт (Transfer Event, status bits 0-23).
    fn transfer_length(&self) -> usize {
        (self.status & 0x00FF_FFFF) as usize
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

/// Bulk endpoint массового запоминающего устройства.
struct BulkEndpoint {
    dci: u8,
    max_packet: u16,
    /// `None` только на время выполнения transfer (кольцо временно «взято»).
    ring: Option<Ring>,
}

/// USB mass storage (Bulk-Only Transport) устройство.
struct StorageDevice {
    slot: u8,
    port: u8,
    vendor: u16,
    product: u16,
    interface: u8,
    /// Адреса endpoint'ов нужны для ClearFeature(HALT).
    in_address: u8,
    out_address: u8,
    in_ep: BulkEndpoint,
    out_ep: BulkEndpoint,
    /// Максимальный размер одного bulk transfer (кратно max_packet).
    max_transfer: usize,
}

impl StorageDevice {
    fn endpoint(&mut self, dir_in: bool) -> &mut BulkEndpoint {
        if dir_in { &mut self.in_ep } else { &mut self.out_ep }
    }
}

/// Состояние слота: EP0 нужен классовым драйверам и после перечисления
/// (GET_MAX_LUN, Bulk-Only Reset), поэтому кольцо живёт в контроллере.
struct SlotState {
    slot: u8,
    port: u8,
    speed: u32,
    /// `None` только на время выполнения control transfer.
    ep0: Option<Ring>,
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
    storage: Vec<StorageDevice>,
    slots: Vec<SlotState>,
    devices: Vec<UsbDeviceInfo>,
    /// Сколько страниц scratchpad зарезервировано (нужно для Address Device).
    scratchpad_pages: usize,
    /// Перечисление портов уже выполнено.
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

    /// Обрабатывает «чужие» события, пока ждём своё: иначе отчёты клавиатуры
    /// и изменения портов терялись бы на время длинного bulk transfer.
    unsafe fn dispatch_event(&mut self, event: Event) {
        match event.trb_type() {
            TRB_TRANSFER_EVENT => self.handle_keyboard_event(event),
            TRB_PORT_STATUS_CHANGE => {
                let port = ((event.param >> 24) & 0xFF) as u8;
                if port >= 1 && port <= self.max_ports {
                    self.port_clear_changes(port);
                }
            }
            _ => {}
        }
    }

    /// Transfer Event клавиатуры: разбор отчёта + переотправка TRB.
    unsafe fn handle_keyboard_event(&mut self, event: Event) {
        let (slot, dci) = (event.slot(), event.endpoint());
        let db = self.db;
        let Some(keyboard) = self
            .keys
            .iter_mut()
            .find(|k| k.slot == slot && k.dci == dci)
        else {
            return;
        };
        if !keyboard.pending {
            return;
        }

        let code = event.completion_code();
        if code == CC_SUCCESS || code == CC_SHORT_PACKET {
            // Boot-протокол всегда 8 байт; отчёт может быть короче только
            // при ошибке, поэтому читаем ровно 8.
            let report = core::slice::from_raw_parts(keyboard.report_virt as *const u8, 8);
            crate::drivers::usb::hid::handle_boot_report(report);
            keyboard.reports = keyboard.reports.wrapping_add(1);
        } else if code == CC_STALL {
            // Boot-клавиатура не должна STALL'ить; отключаем опрос, чтобы
            // не крутить ошибку в цикле.
            keyboard.pending = false;
            return;
        }

        // Переотправляем TRB, иначе endpoint остановится после отчёта.
        keyboard.ring.push(
            keyboard.report_phys,
            keyboard.max_packet as u32,
            (TRB_NORMAL << 10) | (1 << 5),
        );
        wr32(db + (keyboard.slot as u64) * 4, keyboard.dci as u32);
    }

    /// Ждёт Transfer Event для указанного slot/DCI.
    /// Возвращает `(completion_code, residual)`.
    unsafe fn wait_transfer_event(
        &mut self,
        slot: u8,
        dci: u8,
        limit: u32,
    ) -> Option<(u32, usize)> {
        for _ in 0..limit {
            let Some(event) = self.next_event() else {
                core::hint::spin_loop();
                continue;
            };
            if event.trb_type() == TRB_TRANSFER_EVENT
                && event.slot() == slot
                && event.endpoint() == dci
            {
                return Some((event.completion_code(), event.transfer_length()));
            }
            self.dispatch_event(event);
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
    control_transfer_raw(ctrl, slot, ring, bm, request, value, index, data).ok()
}

/// Как `control_transfer`, но с кодом ошибки: классовые драйверы должны
/// отличать STALL (нормальный ответ на GET_MAX_LUN) от таймаута.
unsafe fn control_transfer_raw(
    ctrl: &mut Controller,
    slot: u8,
    ring: *mut Ring,
    bm: u8,
    request: u8,
    value: u16,
    index: u16,
    data: Option<(&mut [u8], u64)>,
) -> Result<usize, UsbError> {
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
    match ctrl.wait_transfer_event(slot, 1, 4_000_000) {
        Some((code, _)) if code == CC_SUCCESS || code == CC_SHORT_PACKET => Ok(len),
        Some((code, _)) if code == CC_STALL => Err(UsbError::Stall),
        Some(_) => Err(UsbError::TransferFailed),
        None => Err(UsbError::Timeout),
    }
}

/// Control transfer с автоматическим DMA-буфером и владением EP0-кольцом.
unsafe fn slot_control(
    ctrl: &mut Controller,
    slot: u8,
    bm: u8,
    request: u8,
    value: u16,
    index: u16,
    data: Option<&mut [u8]>,
) -> Result<usize, UsbError> {
    let Some(position) = ctrl.slots.iter().position(|s| s.slot == slot) else {
        return Err(UsbError::NoDevice);
    };
    let Some(mut ring) = ctrl.slots[position].ep0.take() else {
        return Err(UsbError::NoDevice);
    };
    let ring_ptr = &mut ring as *mut Ring;

    let result = match data {
        None => control_transfer_raw(ctrl, slot, ring_ptr, bm, request, value, index, None),
        Some(buffer) if buffer.is_empty() => {
            control_transfer_raw(ctrl, slot, ring_ptr, bm, request, value, index, None).map(|_| 0)
        }
        Some(buffer) => {
            let len = buffer.len();
            match DmaBuffer::allocate(len) {
                Some(mut dma) => {
                    let dir_in = bm & 0x80 != 0;
                    if !dir_in {
                        dma.as_mut_slice().copy_from_slice(buffer);
                    }
                    let phys = dma.physical_address() as u64;
                    let transferred = control_transfer_raw(
                        ctrl,
                        slot,
                        ring_ptr,
                        bm,
                        request,
                        value,
                        index,
                        Some((dma.as_mut_slice(), phys)),
                    );
                    match transferred {
                        Ok(n) if dir_in => {
                            let n = n.min(len);
                            buffer[..n].copy_from_slice(&dma.as_slice()[..n]);
                            Ok(n)
                        }
                        other => other,
                    }
                }
                None => Err(UsbError::NoMemory),
            }
        }
    };

    if let Some(state) = ctrl.slots.iter_mut().find(|s| s.slot == slot) {
        state.ep0 = Some(ring);
    }
    result
}

/// Информация об endpoint из configuration descriptor.
#[derive(Clone, Copy)]
struct EndpointInfo {
    address: u8,
    max_packet: u16,
    interval: u8,
    attributes: u8,
}

impl EndpointInfo {
    /// Device Context Index: номер endpoint * 2 + направление (IN = 1).
    fn dci(&self) -> u8 {
        ((self.address & 0x0F) << 1) | ((self.address >> 7) & 1)
    }

    fn is_in(&self) -> bool {
        self.address & 0x80 != 0
    }
}

/// Разобранный интерфейс конфигурации.
#[derive(Clone, Copy)]
struct InterfaceInfo {
    number: u8,
    class: u8,
    subclass: u8,
    protocol: u8,
    in_ep: Option<EndpointInfo>,
    out_ep: Option<EndpointInfo>,
}

/// Разбирает configuration descriptor на интерфейсы с их endpoints.
///
/// Раньше здесь был поиск «первого HID-интерфейса»: для mass storage и для
/// составных устройств (хаб + клавиатура) этого недостаточно.
fn parse_interfaces(config: &[u8], out: &mut Vec<InterfaceInfo>) {
    let mut current: Option<InterfaceInfo> = None;
    let mut offset = 0usize;

    while offset + 2 <= config.len() {
        let len = config[offset] as usize;
        let dtype = config[offset + 1];
        if len < 2 || offset + len > config.len() {
            break;
        }

        match dtype {
            4 if len >= 9 => {
                if let Some(iface) = current.take() {
                    out.push(iface);
                }
                current = Some(InterfaceInfo {
                    number: config[offset + 2],
                    class: config[offset + 5],
                    subclass: config[offset + 6],
                    protocol: config[offset + 7],
                    in_ep: None,
                    out_ep: None,
                });
            }
            5 if len >= 7 => {
                if let Some(iface) = current.as_mut() {
                    let info = EndpointInfo {
                        address: config[offset + 2],
                        attributes: config[offset + 3],
                        max_packet: u16::from_le_bytes([config[offset + 4], config[offset + 5]])
                            & 0x07FF,
                        interval: config[offset + 6],
                    };
                    // bits 0-1: 0 = control, 1 = isochronous, 2 = bulk,
                    // 3 = interrupt.
                    match info.attributes & 0x03 {
                        2 | 3 => {
                            if info.is_in() {
                                if iface.in_ep.is_none() {
                                    iface.in_ep = Some(info);
                                }
                            } else if iface.out_ep.is_none() {
                                iface.out_ep = Some(info);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }

        offset += len;
    }

    if let Some(iface) = current.take() {
        out.push(iface);
    }
}

/// Boot-клавиатура: HID boot interface с interrupt IN endpoint.
fn find_keyboard_interface(interfaces: &[InterfaceInfo]) -> Option<InterfaceInfo> {
    interfaces.iter().copied().find(|i| {
        i.class == 3
            && i.protocol <= 1
            && (i.subclass == 1 || i.protocol == 1)
            && i.in_ep.map_or(false, |e| e.attributes & 0x03 == 3)
    })
}

/// USB Mass Storage, Bulk-Only Transport: class 8, subclass 6, protocol 0x50.
fn find_storage_interface(interfaces: &[InterfaceInfo]) -> Option<InterfaceInfo> {
    interfaces.iter().copied().find(|i| {
        i.class == 8
            && i.subclass == 6
            && i.protocol == 0x50
            && i.in_ep.map_or(false, |e| e.attributes & 0x03 == 2)
            && i.out_ep.map_or(false, |e| e.attributes & 0x03 == 2)
    })
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

/// Описание endpoint для Configure Endpoint.
struct EpConfig {
    dci: u8,
    ep_type: u32,
    max_packet: u16,
    interval: u8,
    ring_phys: u64,
}

/// Configure Endpoint для набора endpoint'ов (клавиатура, bulk storage).
unsafe fn configure_endpoints(
    ctrl: &mut Controller,
    slot: u8,
    speed: u32,
    port: u8,
    eps: &[EpConfig],
) -> bool {
    if eps.is_empty() {
        return true;
    }
    let Some((in_phys, in_virt)) = alloc_dma(4096) else {
        return false;
    };

    let mut add_flags: u32 = 1; // bit 0 = slot context
    let mut max_dci = 0u8;
    for ep in eps {
        add_flags |= 1u32 << ep.dci;
        max_dci = max_dci.max(ep.dci);
    }
    wr32(in_virt + 0x04, add_flags);

    // Slot Context: context entries = максимальный DCI
    wr32(in_virt + 0x20, (speed << 20) | ((max_dci as u32) << 27));
    wr32(in_virt + 0x24, (port as u32) << 16);

    for ep in eps {
        let ctx = in_virt + 0x20 + (ep.dci as u64) * 0x20;
        // Dword 0: interval (bits 16-23). Dword 1: CErr=3 (bits 1-2),
        // EP type (bits 3-5), MaxPacketSize (bits 16-31).
        wr32(ctx + 0x00, (ep.interval as u32) << 16);
        wr32(
            ctx + 0x04,
            (3 << 1) | (ep.ep_type << 3) | ((ep.max_packet as u32) << 16),
        );
        wr32(ctx + 0x08, (ep.ring_phys as u32) | 1);
        wr32(ctx + 0x0C, (ep.ring_phys >> 32) as u32);
        // Средняя длина TRB: 8 для interrupt, 1024 для bulk.
        let avg_trb = if ep.ep_type == EP_TYPE_BULK_IN || ep.ep_type == EP_TYPE_BULK_OUT {
            1024
        } else {
            8
        };
        wr32(ctx + 0x10, avg_trb);
    }

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

    match ctrl.command(in_phys, TRB_EVALUATE_CONTEXT, slot) {
        Some(event) => event.completion_code() == CC_SUCCESS,
        None => false,
    }
}

/// Перечисляет устройство на порту: регистрирует его в реестре и настраивает
/// class-specific endpoints (HID keyboard, mass storage).
unsafe fn enumerate_device(
    ctrl: &mut Controller,
    controller_index: usize,
    port: u8,
    speed: u32,
) {
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
    let Some(mut ep0) = Ring::new() else { return };
    ep0.init_link();
    let ep0_phys = ep0.phys;

    // DCBAA[slot] → device context
    wr64(ctrl.dcbaa_virt + (slot as u64) * 8, dev_phys);

    let Some((in_phys, in_virt)) = alloc_dma(4096) else { return };
    // Начальный EP0 max packet: 64 только для HS/Super, иначе 8 (USB 2.0:
    // FS-устройства по умолчанию используют 8-байтовые пакеты).
    let max_packet: u16 = if speed == SPEED_HIGH || speed == 4 { 64 } else { 8 };
    fill_address_context(in_virt, speed, port, max_packet, ep0_phys);

    let Some(event) = ctrl.command(in_phys, TRB_ADDRESS_DEVICE, slot) else {
        println!("[xHCI] slot {}: Address Device timeout", slot);
        return;
    };
    if event.completion_code() != CC_SUCCESS {
        println!("[xHCI] slot {}: Address Device failed (cc={})", slot, event.completion_code());
        return;
    }

    // Слот живёт в контроллере: EP0-кольцо нужно и после перечисления
    // (class requests: GET_MAX_LUN, Bulk-Only Reset).
    ctrl.slots.push(SlotState { slot, port, speed, ep0: Some(ep0) });

    // Первые 8 байт device descriptor: bMaxPacketSize0 лежит в offset 7.
    let mut short = [0u8; 8];
    if slot_control(ctrl, slot, 0x80, 6, 0x0100, 0, Some(&mut short)).is_err() {
        println!("[xHCI] slot {}: GET_DESCRIPTOR(device, 8) failed", slot);
        return;
    }
    let device_max_packet = short[7] as u16;
    if device_max_packet != max_packet && (8..=64).contains(&device_max_packet) {
        // Evaluate Context применяет новый размер в контроллере.
        if !evaluate_ep0(ctrl, slot, speed, port, device_max_packet, ep0_phys) {
            println!(
                "[xHCI] slot {}: Evaluate Context failed, keeping EP0 mps={}",
                slot, max_packet
            );
        }
    }

    // Device descriptor (18 bytes)
    let mut device = [0u8; 18];
    if slot_control(ctrl, slot, 0x80, 6, 0x0100, 0, Some(&mut device)).is_err() {
        println!("[xHCI] slot {}: GET_DESCRIPTOR(device) failed", slot);
        return;
    }
    let vendor = u16::from_le_bytes([device[8], device[9]]);
    let product = u16::from_le_bytes([device[10], device[11]]);
    if device[17] == 0 {
        return;
    }

    // Регистрируем устройство в общем реестре (`usb::register_device_from_descriptor`
    // раньше не вызывался ниоткуда — команда `devices`/`diag usb` была пустой).
    if let Some(descriptor) = crate::drivers::usb::parse_device_descriptor(&device) {
        crate::drivers::usb::register_device_from_descriptor(&descriptor);
    }

    // Configuration descriptor: сначала 9 байт, затем целиком.
    let mut header = [0u8; 9];
    if slot_control(ctrl, slot, 0x80, 6, 0x0200, 0, Some(&mut header)).is_err() {
        println!("[xHCI] slot {}: GET_DESCRIPTOR(config header) failed", slot);
        return;
    }
    let total = u16::from_le_bytes([header[2], header[3]]) as usize;
    if total < 9 || total > 4096 {
        println!("[xHCI] slot {}: config descriptor length {} is invalid", slot, total);
        return;
    }
    // Раньше буфер был ограничен 512 байтами: составные устройства и
    // несколько интерфейсов в него не влезали.
    let mut config = alloc::vec![0u8; total];
    if slot_control(ctrl, slot, 0x80, 6, 0x0200, 0, Some(&mut config)).is_err() {
        println!("[xHCI] slot {}: GET_DESCRIPTOR(config) failed", slot);
        return;
    }
    let config_value = config[5];

    let mut interfaces = Vec::new();
    parse_interfaces(&config, &mut interfaces);

    // SET_CONFIGURATION
    if slot_control(ctrl, slot, 0x00, 9, config_value as u16, 0, None).is_err() {
        println!("[xHCI] slot {}: SET_CONFIGURATION failed", slot);
        return;
    }

    let keyboard = find_keyboard_interface(&interfaces);
    let storage = find_storage_interface(&interfaces);

    let mut kind = "unsupported";
    if let Some(interface) = keyboard {
        if setup_keyboard(ctrl, slot, speed, port, &interface) {
            kind = "keyboard";
        }
    }
    if kind == "unsupported" {
        if let Some(interface) = storage {
            if setup_storage(ctrl, slot, speed, port, vendor, product, &interface) {
                kind = "storage";
            }
        }
    }

    let (class, subclass, protocol) = match keyboard {
        Some(i) => (i.class, i.subclass, i.protocol),
        None => match storage {
            Some(i) => (i.class, i.subclass, i.protocol),
            None => (device[4], device[5], device[6]),
        },
    };

    println!(
        "[xHCI] slot {}: {:04x}:{:04x} class {:02x}/{:02x}/{:02x} -> {}{}",
        slot,
        vendor,
        product,
        class,
        subclass,
        protocol,
        kind,
        if kind == "unsupported" {
            alloc::format!(" ({} interface(s), none supported)", interfaces.len())
        } else {
            String::new()
        }
    );

    ctrl.devices.push(UsbDeviceInfo {
        controller: controller_index,
        slot,
        port,
        speed,
        vendor,
        product,
        class,
        subclass,
        protocol,
        kind,
    });
}

/// Настраивает interrupt IN endpoint boot-клавиатуры и запускает опрос.
unsafe fn setup_keyboard(
    ctrl: &mut Controller,
    slot: u8,
    speed: u32,
    port: u8,
    interface: &InterfaceInfo,
) -> bool {
    let Some(endpoint) = interface.in_ep else {
        return false;
    };

    // HID boot protocol + idle rate 0 (report only on change)
    let _ = slot_control(ctrl, slot, 0x21, 0x0B, 0, interface.number as u16, None);
    let _ = slot_control(ctrl, slot, 0x21, 0x0A, 0, interface.number as u16, None);

    let Some(mut ring) = Ring::new() else {
        return false;
    };
    ring.init_link();
    // Boot-отчёт всегда 8 байт; буфер с запасом на случай большего
    // wMaxPacketSize (некоторые устройства объявляют 64).
    let report_len: u16 = 8;
    let Some((report_phys, report_virt)) = alloc_dma(64) else {
        return false;
    };

    let dci = endpoint.dci();
    let config = [EpConfig {
        dci,
        ep_type: EP_TYPE_INTERRUPT_IN,
        max_packet: endpoint.max_packet,
        interval: endpoint.interval,
        ring_phys: ring.phys,
    }];
    if !configure_endpoints(ctrl, slot, speed, port, &config) {
        println!("[xHCI] slot {}: Configure Endpoint (interrupt IN) failed", slot);
        return false;
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
        "[xHCI] HID keyboard ready: slot {} port {} ep=0x{:02x} mps={} interval={}",
        slot, port, endpoint.address, endpoint.max_packet, endpoint.interval
    );
    true
}

/// Настраивает bulk IN/OUT endpoints USB mass storage (BOT).
unsafe fn setup_storage(
    ctrl: &mut Controller,
    slot: u8,
    speed: u32,
    port: u8,
    vendor: u16,
    product: u16,
    interface: &InterfaceInfo,
) -> bool {
    let (Some(in_ep), Some(out_ep)) = (interface.in_ep, interface.out_ep) else {
        return false;
    };

    let Some(mut in_ring) = Ring::new() else {
        return false;
    };
    in_ring.init_link();
    let Some(mut out_ring) = Ring::new() else {
        return false;
    };
    out_ring.init_link();

    let configs = [
        EpConfig {
            dci: in_ep.dci(),
            ep_type: EP_TYPE_BULK_IN,
            max_packet: in_ep.max_packet,
            interval: 0,
            ring_phys: in_ring.phys,
        },
        EpConfig {
            dci: out_ep.dci(),
            ep_type: EP_TYPE_BULK_OUT,
            max_packet: out_ep.max_packet,
            interval: 0,
            ring_phys: out_ring.phys,
        },
    ];
    if !configure_endpoints(ctrl, slot, speed, port, &configs) {
        println!("[xHCI] slot {}: Configure Endpoint (bulk) failed", slot);
        return false;
    }

    let max_transfer = MAX_BULK_CHUNK.min((in_ep.max_packet as usize).max(512) * 16);
    ctrl.storage.push(StorageDevice {
        slot,
        port,
        vendor,
        product,
        interface: interface.number,
        in_address: in_ep.address,
        out_address: out_ep.address,
        in_ep: BulkEndpoint {
            dci: in_ep.dci(),
            max_packet: in_ep.max_packet,
            ring: Some(in_ring),
        },
        out_ep: BulkEndpoint {
            dci: out_ep.dci(),
            max_packet: out_ep.max_packet,
            ring: Some(out_ring),
        },
        max_transfer,
    });

    println!(
        "[xHCI] mass storage ready: slot {} port {} iface {} bulk in=0x{:02x}/{} out=0x{:02x}/{} (chunk {})",
        slot, port, interface.number,
        in_ep.address, in_ep.max_packet,
        out_ep.address, out_ep.max_packet,
        max_transfer
    );
    true
}

static XHCI: Mutex<Vec<Controller>> = Mutex::new(Vec::new());

/// Есть ли хотя бы один инициализированный контроллер.
pub fn controller_count() -> usize {
    XHCI.lock().len()
}

/// Опрашивает event ring всех контроллеров и разбирает события.
/// Вызывается из `keyboard::poll_key()` и из shell idle path.
pub fn poll_events() {
    let mut controllers = match XHCI.try_lock() {
        Some(guard) => guard,
        None => return,
    };

    for ctrl in controllers.iter_mut() {
        let mut budget = 0;
        loop {
            let Some(event) = (unsafe { ctrl.next_event() }) else { break };
            budget += 1;
            if budget > 64 {
                break;
            }
            unsafe { ctrl.dispatch_event(event) };
        }
    }
}

/// Совместимость: опрос клавиатуры = опрос всех событий.
pub fn poll_keyboard() {
    poll_events();
}

/// Перечисляет все порты всех контроллеров (вызывается после `timer::init()`).
///
/// Раньше перечисление было «одноразовым» и происходило при первом опросе
/// клавиатуры: USB-накопитель до этого не существовал, а зависший порт
/// блокировал shell. Теперь enumeration явный.
pub fn enumerate_now() {
    let mut controllers = match XHCI.try_lock() {
        Some(guard) => guard,
        None => {
            println!("[xHCI] enumeration skipped: controller lock is busy");
            return;
        }
    };

    if controllers.is_empty() {
        return;
    }

    println!("[xHCI] enumerating ports...");
    for index in 0..controllers.len() {
        if controllers[index].enumerated {
            continue;
        }
        controllers[index].enumerated = true;
        unsafe { enumerate_ports(&mut controllers[index], index) };
    }

    let (devices, keyboards) = controllers
        .iter()
        .fold((0, 0), |(d, k), c| (d + c.devices.len(), k + c.keys.len()));
    println!(
        "[xHCI] enumeration done: {} device(s), {} keyboard(s)",
        devices, keyboards
    );
}

/// Все найденные устройства (для `diag usb`).
pub fn devices() -> Vec<UsbDeviceInfo> {
    XHCI
        .lock()
        .iter()
        .flat_map(|ctrl| ctrl.devices.iter().cloned())
        .collect()
}

/// Mass storage устройства, готовые к использованию.
pub fn storage_devices() -> Vec<StorageInfo> {
    let mut result = Vec::new();
    for (index, ctrl) in XHCI.lock().iter().enumerate() {
        for dev in &ctrl.storage {
            result.push(StorageInfo {
                controller: index,
                slot: dev.slot,
                port: dev.port,
                vendor: dev.vendor,
                product: dev.product,
                max_transfer: dev.max_transfer,
            });
        }
    }
    result
}

/// Control transfer к устройству по слоту.
pub fn control(
    controller: usize,
    slot: u8,
    bm: u8,
    request: u8,
    value: u16,
    index: u16,
    data: Option<&mut [u8]>,
) -> Result<usize, UsbError> {
    let mut controllers = XHCI.lock();
    let Some(ctrl) = controllers.get_mut(controller) else {
        return Err(UsbError::NoDevice);
    };
    unsafe { slot_control(ctrl, slot, bm, request, value, index, data) }
}

/// Bulk transfer к устройству по слоту.
///
/// `dir_in` = true — чтение с устройства. Возвращает число фактически
/// переданных байт (может быть меньше `data.len()` при short packet).
pub fn bulk(
    controller: usize,
    slot: u8,
    dir_in: bool,
    data: &mut [u8],
) -> Result<usize, UsbError> {
    if data.is_empty() {
        return Ok(0);
    }

    let mut controllers = XHCI.lock();
    let Some(ctrl) = controllers.get_mut(controller) else {
        return Err(UsbError::NoDevice);
    };
    let Some(index) = ctrl.storage.iter().position(|d| d.slot == slot) else {
        return Err(UsbError::NoDevice);
    };

    let (dci, max_transfer) = {
        let dev = &mut ctrl.storage[index];
        (dev.endpoint(dir_in).dci, dev.max_transfer)
    };

    let mut done = 0usize;
    while done < data.len() {
        let chunk = (data.len() - done).min(max_transfer.max(512));
        let got = unsafe { bulk_chunk(ctrl, index, dir_in, dci, &mut data[done..done + chunk]) }?;
        done += got;
        // Короткий пакет означает конец transfer.
        if got < chunk {
            break;
        }
    }
    Ok(done)
}

/// Один bulk transfer (не больше `max_transfer`) через TRB Normal.
unsafe fn bulk_chunk(
    ctrl: &mut Controller,
    device_index: usize,
    dir_in: bool,
    dci: u8,
    data: &mut [u8],
) -> Result<usize, UsbError> {
    let Some(mut dma) = DmaBuffer::allocate(data.len()) else {
        return Err(UsbError::NoMemory);
    };
    if !dir_in {
        dma.as_mut_slice().copy_from_slice(data);
    }
    let phys = dma.physical_address() as u64;

    let slot = ctrl.storage[device_index].slot;
    let Some(mut ring) = ctrl.storage[device_index].endpoint(dir_in).ring.take() else {
        return Err(UsbError::NoDevice);
    };

    let mut trb_control = (TRB_NORMAL << 10) | (1 << 5); // ISP = 1
    if dir_in {
        trb_control |= 1 << 16;
    }
    ring.push(phys, data.len() as u32, trb_control);
    ctrl.doorbell(slot, dci as u32);

    let event = ctrl.wait_transfer_event(slot, dci, 20_000_000);

    // Кольцо возвращаем устройству в любом случае.
    ctrl.storage[device_index].endpoint(dir_in).ring = Some(ring);

    let Some((code, residual)) = event else {
        return Err(UsbError::Timeout);
    };
    if code == CC_STALL {
        return Err(UsbError::Stall);
    }
    if code != CC_SUCCESS && code != CC_SHORT_PACKET {
        return Err(UsbError::TransferFailed);
    }

    let received = data.len().saturating_sub(residual.min(data.len()));
    if dir_in && received > 0 {
        data[..received].copy_from_slice(&dma.as_slice()[..received]);
    }
    Ok(received)
}

/// Bulk-Only Mass Storage Reset + ClearFeature(HALT) на обоих endpoint'ах.
///
/// Вызывается после STALL/timeout: без сброса BOT-сессия остаётся в
/// рассогласованном состоянии (CSW от предыдущей команды).
pub fn resolve_stall(controller: usize, slot: u8) -> Result<(), UsbError> {
    let mut controllers = XHCI.lock();
    let Some(ctrl) = controllers.get_mut(controller) else {
        return Err(UsbError::NoDevice);
    };
    let Some(index) = ctrl.storage.iter().position(|d| d.slot == slot) else {
        return Err(UsbError::NoDevice);
    };
    let (interface, in_address, out_address) = {
        let dev = &ctrl.storage[index];
        (dev.interface, dev.in_address, dev.out_address)
    };

    unsafe {
        let _ = slot_control(ctrl, slot, 0x21, 0xFF, 0, interface as u16, None);
        let _ = slot_control(ctrl, slot, 0x02, 1, 0, in_address as u16, None);
        let _ = slot_control(ctrl, slot, 0x02, 1, 0, out_address as u16, None);
    }
    Ok(())
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
            storage: Vec::new(),
            slots: Vec::new(),
            devices: Vec::new(),
            scratchpad_pages: scratchpad_count,
            enumerated: false,
        };

        // Перечисление портов отложено до `enumerate_now()` (после timer::init):
        // на реальном железе оно занимает секунды и не должно задерживать boot.
        println!(
            "[xHCI] {:#x}: running, enumeration deferred (scratchpad {} page(s))",
            bar_phys, controller.scratchpad_pages
        );
        XHCI.lock().push(controller);
        true
    }
}

/// Перечисляет подключённые порты и настраивает найденные устройства.
unsafe fn enumerate_ports(ctrl: &mut Controller, controller_index: usize) {
    for port in 1..=ctrl.max_ports {
        let portsc = rd32(ctrl.port_reg(port));
        if portsc & PORTSC_CCS == 0 {
            continue;
        }
        // Уже перечисленный порт повторно не сбрасываем: reset отключил бы
        // работающее устройство (клавиатуру или накопитель).
        if ctrl.devices.iter().any(|d| d.port == port) {
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
                enumerate_device(ctrl, controller_index, port, speed);
            }
            None => println!("[xHCI] port {}: reset failed", port),
        }
    }
}

