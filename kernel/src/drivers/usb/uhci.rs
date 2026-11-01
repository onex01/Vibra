// UHCI (USB 1.1) — хост-контроллер Intel PIIX/ICH.
//
// Зачем: на чипсетах с EHCI/UHCI (в т.ч. Intel 8-series, AMD SB) клавиатуры
// и мыши low-speed подключены к UHCI-компаньону, а не к xHCI. Драйвер xHCI
// такие устройства не видит вообще, поэтому на десктопе «не работает
// никакая клавиатура».
//
// Реализовано минимально и без прерываний:
//   * control-передачи (SETUP/DATA/STATUS) для адресации и настроек;
//   * interrupt IN — опрос boot-клавиатуры;
//   * bulk IN/OUT — обмен с mass storage (Bulk-Only Transport).
//
// UHCI работает только с 32-битными физическими адресами, поэтому все
// структуры и буферы размещаются в памяти ниже 4 ГиБ (`alloc_low`).

use crate::println;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use spin::Mutex;

// --- Регистры (смещения от I/O base) ---
const REG_CMD: u16 = 0x00;
const REG_STATUS: u16 = 0x02;
const REG_INTR: u16 = 0x04;
const REG_FRNUM: u16 = 0x06;
const REG_FLBASE: u16 = 0x08;
const REG_SOFMOD: u16 = 0x0C;
const REG_PORTSC1: u16 = 0x10;

// USBCMD
const CMD_RS: u16 = 0x0001;
const CMD_HCRESET: u16 = 0x0002;
const CMD_GRESET: u16 = 0x0004;
const CMD_CF: u16 = 0x0040;   // Configure Flag: без него контроллер не работает
const CMD_MAXP: u16 = 0x0080; // 64-байтные пакеты (иначе 32)

// USBSTS
const STS_HCHALTED: u16 = 0x0020;
const STS_ERRINT: u16 = 0x0002;
const STS_HSE: u16 = 0x0004;

// PORTSC
const PORT_CCS: u16 = 0x0001;
const PORT_CSC: u16 = 0x0002;
const PORT_PE: u16 = 0x0004;
const PORT_PEC: u16 = 0x0008;
const PORT_LS: u16 = 0x0100;
const PORT_RD: u16 = 0x0200;
const PORT_SUSP: u16 = 0x1000;
const PORT_RWC: u16 = PORT_CSC | PORT_PEC;

// --- Пакетные идентификаторы TD ---
const PID_SETUP: u32 = 0x2D;
const PID_IN: u32 = 0x69;
const PID_OUT: u32 = 0xE1;

// --- Биты TD status ---
const TD_ACTIVE: u32 = 1 << 23;
const TD_STALLED: u32 = 1 << 22;
const TD_BABBLE: u32 = 1 << 20;
const TD_DBUFERR: u32 = 1 << 19;
const TD_CRC: u32 = 1 << 18;
const TD_BITSTUFF: u32 = 1 << 17;
const TD_ERROR_MASK: u32 = TD_STALLED | TD_BABBLE | TD_DBUFERR | TD_CRC | TD_BITSTUFF;

/// Лимит ожидания завершения TD (итераций опроса).
const TD_SPIN: u32 = 3_000_000;

/// UHCI обрабатывает ровно одну запись списка кадров — ту, на которую
/// указывает FRNUM. Поэтому все 1024 записи указывают на один и тот же
/// control-QH, а клавиатурные QH прицеплены к нему через `QH.head`.

/// Пауза в миллисекундах. До инициализации таймера — прокрутка.
fn delay_ms(ms: u64) {
    if crate::timer::source_hz() > 0 {
        crate::timer::sleep_ms(ms);
    } else {
        for _ in 0..(ms as u32).saturating_mul(100_000) {
            core::hint::spin_loop();
        }
    }
}

#[inline]
unsafe fn inw(port: u16) -> u16 {
    let value: u16;
    core::arch::asm!("in ax, dx", out("ax") value, in("dx") port,
        options(nostack, preserves_flags));
    value
}

#[inline]
unsafe fn outw(port: u16, value: u16) {
    core::arch::asm!("out dx, ax", in("dx") port, in("ax") value,
        options(nostack, preserves_flags));
}

#[inline]
unsafe fn inl(port: u16) -> u32 {
    let value: u32;
    core::arch::asm!("in eax, dx", out("eax") value, in("dx") port,
        options(nostack, preserves_flags));
    value
}

#[inline]
unsafe fn outl(port: u16, value: u32) {
    core::arch::asm!("out dx, eax", in("dx") port, in("eax") value,
        options(nostack, preserves_flags));
}

/// Transfer Descriptor — 32 байта, выравнивание 16.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct Td {
    link: u32,
    status: u32,
    token: u32,
    buffers: [u32; 5],
}


/// Поле Max Length в токене TD: UHCI хранит `len - 1`.
///
/// QEMU и реальные контроллеры вычисляют длину как `(field >> 21) + 1`.
/// Пока здесь стояло `len`, SETUP уходил 9 байтами вместо 8, а IN-передачи
/// читали на байт больше — устройство отвечало ошибкой.
fn max_len_field(len: usize) -> u32 {
    if len == 0 {
        0
    } else {
        ((len - 1) as u32) << 21
    }
}

impl Td {
    const fn empty() -> Self {
        Td { link: 1, status: 0, token: 0, buffers: [0; 5] }
    }

    fn is_active(&self) -> bool {
        self.status & TD_ACTIVE != 0
    }

    /// Фактическая длина. UHCI хранит `len - 1` в битах 0-10.
    fn actual_len(&self) -> u32 {
        (self.status & 0x7FF) + 1
    }

    fn failed(&self) -> bool {
        self.status & TD_ERROR_MASK != 0
    }
}

/// Queue Head — 16 байт, выравнивание 16.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct Qh {
    head: u32,
    element: u32,
    _reserved: [u32; 2],
}

impl Qh {
    const fn empty() -> Self {
        Qh { head: 1, element: 1, _reserved: [0; 2] }
    }
}

/// Контроллер UHCI.
struct Uhci {
    io: u16,
    /// Список кадров: 1024 dword, 4096-выровнен.
    frame: *mut u32,
    frame_phys: u32,
    /// QH для control/bulk (кадр 0).
    qh: *mut Qh,
    qh_phys: u32,
    /// Пул TD для последовательных передач.
    tds: *mut Td,
    td_phys: [u32; TD_COUNT],
    /// Общий staging-буфер.
    buf: *mut u8,
    buf_phys: u32,
    buf_len: usize,
}

const TD_COUNT: usize = 8;

// SAFETY: контроллер и его DMA-структуры используются только под
// `CONTROLLERS`/`KEYBOARDS` (спин-локи), ядро однопоточное до SMP.
unsafe impl Send for Uhci {}

/// Найденное устройство на UHCI.
pub struct UhciDevice {
    pub controller: usize,
    pub port: u8,
    pub address: u8,
    pub low_speed: bool,
    pub max_packet: u8,
    pub vendor: u16,
    pub product: u16,
}

struct KeyboardSlot {
    device: UhciDevice,
    #[allow(dead_code)]
    interface: u8,
    #[allow(dead_code)]
    endpoint: u8,
    #[allow(dead_code)]
    toggle: bool,
    qh: *mut Qh,
    qh_phys: u32,
    td: *mut Td,
    td_phys: u32,
    report: *mut u8,
    report_phys: u32,
    report_len: usize,
    last_report: [u8; 8],
}

unsafe impl Send for KeyboardSlot {}

static CONTROLLERS: Mutex<Vec<Uhci>> = Mutex::new(Vec::new());
static DEVICES: Mutex<Vec<UhciDevice>> = Mutex::new(Vec::new());
static KEYBOARDS: Mutex<Vec<KeyboardSlot>> = Mutex::new(Vec::new());
static READY: AtomicBool = AtomicBool::new(false);

// ==========================================
// Память ниже 4 ГиБ
// ==========================================

fn hhdm() -> u64 {
    crate::memory::paging::HHDM_OFFSET.load(Ordering::Relaxed)
}

/// Выделить выровненный буфер, физический адрес которого помещается в u32.
///
/// UHCI адресует только 32-битную физику. PMM на машине с 32 ГиБ может
/// выдать кадр выше 4 ГиБ, поэтому такие кадры сразу возвращаем в PMM.
fn alloc_low(size: usize, align: usize) -> Option<(*mut u8, u32)> {
    let layout = core::alloc::Layout::from_size_align(size, align.max(16)).ok()?;
    for _ in 0..64 {
        let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) };
        if ptr.is_null() {
            return None;
        }
        let phys = (ptr as u64).wrapping_sub(hhdm());
        if phys != 0 && phys < 0xFFFF_F000 {
            return Some((ptr, phys as u32));
        }
        // Слишком высокий адрес: освобождаем и пробуем следующий кадр.
        unsafe { alloc::alloc::dealloc(ptr, layout) };
    }
    None
}

// ==========================================
// Инициализация
// ==========================================

/// Инициализировать контроллер по I/O BAR.
pub fn init(io_base: u16) -> bool {
    if io_base == 0 {
        return false;
    }

    let frame_size = 4096;
    let Some((frame, frame_phys)) = alloc_low(frame_size, 4096) else {
        println!("[UHCI] no low memory for frame list");
        return false;
    };
    let frame = frame as *mut u32;

    let Some((qh, qh_phys)) = alloc_low(core::mem::size_of::<Qh>(), 16) else {
        return false;
    };
    let qh = qh as *mut Qh;

    let Some((buf, buf_phys)) = alloc_low(4096, 16) else {
        return false;
    };

    unsafe {
        qh.write_volatile(Qh::empty());
    }

    let mut td_phys = [0u32; TD_COUNT];
    let Some((tds, tds_phys)) = alloc_low(core::mem::size_of::<Td>() * TD_COUNT, 16) else {
        return false;
    };
    let tds = tds as *mut Td;
    for index in 0..TD_COUNT {
        td_phys[index] = tds_phys + (index * core::mem::size_of::<Td>()) as u32;
    }

    let mut controller = Uhci {
        io: io_base,
        frame,
        frame_phys,
        qh,
        qh_phys,
        tds,
        td_phys,
        buf,
        buf_phys,
        buf_len: 4096,
    };

    if !controller.reset() {
        println!("[UHCI] {:#x}: reset failed", io_base);
        return false;
    }

    println!(
        "[UHCI] {:#06x}: frame list phys={:#x} qh={:#x} buf={:#x}",
        io_base, frame_phys, qh_phys, buf_phys
    );
    CONTROLLERS.lock().push(controller);
    READY.store(true, Ordering::Relaxed);
    true
}

impl Uhci {
    /// Сброс контроллера и запуск.
    fn reset(&mut self) -> bool {
        unsafe {
            // GRESET сбрасывает всю шину, затем HCRESET — контроллер.
            outw(self.io + REG_CMD, CMD_GRESET);
            for _ in 0..10_000 {
                core::hint::spin_loop();
            }
            outw(self.io + REG_CMD, 0);

            outw(self.io + REG_CMD, CMD_HCRESET);
            for _ in 0..1_000_000 {
                if inw(self.io + REG_CMD) & CMD_HCRESET == 0 {
                    break;
                }
                core::hint::spin_loop();
            }

            // CF=1 обязателен: без Configure Flag контроллер не обрабатывает
            // список кадров, и любая передача завершается ошибкой шины.
            outw(self.io + REG_CMD, CMD_CF | CMD_MAXP);

            // Адрес списка кадров и начало с кадра 0.
            outl(self.io + REG_FLBASE, self.frame_phys);
            outw(self.io + REG_FRNUM, 0);
            outw(self.io + REG_INTR, 0);
            outw(self.io + REG_SOFMOD, 64);

            // Снимаем накопленные статусы.
            outw(self.io + REG_STATUS, 0xFFFF);

            // Все записи списка кадров указывают на control-QH: контроллер
            // обрабатывает только запись по FRNUM, а FRNUM меняется каждый
            // кадр — с одной записью передача ждала бы до 1024 мс.
            for index in 0..1024 {
                self.frame.add(index).write_volatile(self.qh_phys | 2);
            }

            outw(self.io + REG_CMD, CMD_CF | CMD_MAXP | CMD_RS);
            for _ in 0..100_000 {
                if inw(self.io + REG_STATUS) & STS_HCHALTED == 0 {
                    return true;
                }
                core::hint::spin_loop();
            }
        }
        false
    }

    fn port_count(&self) -> u8 {
        // UHCI всегда имеет два корневых порта.
        2
    }

    fn portsc(&self, port: u8) -> u16 {
        unsafe { inw(self.io + REG_PORTSC1 + (port as u16 - 1) * 2) }
    }

    fn port_write(&self, port: u8, value: u16) {
        unsafe { outw(self.io + REG_PORTSC1 + (port as u16 - 1) * 2, value) }
    }

    /// Сбросить порт по последовательности из спецификации UHCI 2.1 §2.2.1.
    ///
    /// PORTSC — обычный RW-регистр: запись обнуляет все биты, которых нет в
    /// значении. Поэтому «очистить изменения» отдельной записью нельзя —
    /// это выключает порт (PE). Все нужные биты пишутся одним значением.
    fn port_reset(&self, port: u8) -> Option<bool> {
        if self.portsc(port) & PORT_CCS == 0 {
            return None;
        }

        // 1. Снять накопленные изменения при выключенном порте.
        self.port_write(port, PORT_RWC);

        // 2. Reset: держим RD не меньше 50 мс.
        self.port_write(port, PORT_RD);
        delay_ms(50);
        self.port_write(port, 0);
        delay_ms(10);

        // 3. Включить порт и снять изменения ОДНОЙ записью.
        self.port_write(port, PORT_PE | PORT_RWC);
        delay_ms(10);

        let mut status = self.portsc(port);
        for _ in 0..20 {
            if status & PORT_PE != 0 {
                break;
            }
            delay_ms(5);
            status = self.portsc(port);
        }

        if status & PORT_PE == 0 {
            println!(
                "[UHCI] port {}: not enabled after reset (PORTSC={:#06x})",
                port, status
            );
            return None;
        }

        Some(status & PORT_LS != 0)
    }

    /// Выполнить цепочку TD через QH в кадре 0 и дождаться завершения.
    fn run_chain(&mut self, first_td: usize, count: usize) -> Result<usize, ()> {
        unsafe {
            // Связываем TD в цепочку.
            for index in 0..count {
                let td = self.tds.add(first_td + index);
                let link = if index + 1 < count {
                    self.td_phys[first_td + index + 1]
                } else {
                    1
                };
                (*td).link = link;
            }

            let qh = self.qh;
            (*qh).element = self.td_phys[first_td];
            // `head` не трогаем: он держит цепочку клавиатурных QH.
            // QH уже в кадровом списке (все записи указывают на него),
            // достаточно выставить элемент.
            let _ = &self.frame_phys;

            let last = self.tds.add(first_td + count - 1);
            let mut completed = false;
            for _ in 0..TD_SPIN {
                if !(*last).is_active() {
                    completed = true;
                    break;
                }
                core::hint::spin_loop();
            }

            // Снимаем элемент: иначе контроллер будет перезапускать
            // завершённые TD каждый кадр.
            (*qh).element = 1;

            if !completed {
                return Err(());
            }
            if (*last).failed() {
                return Err(());
            }
            Ok((*last).actual_len() as usize)
        }
    }

    /// Control-передача: SETUP → DATA (опционально) → STATUS.
    fn control(
        &mut self,
        address: u8,
        low_speed: bool,
        max_packet: u8,
        setup: &[u8; 8],
        data: Option<&mut [u8]>,
        dir_in: bool,
    ) -> Result<usize, ()> {
        let td_size = core::mem::size_of::<Td>();
        let _ = td_size;

        // Setup-пакет кладём в staging-буфер.
        unsafe {
            core::ptr::copy_nonoverlapping(setup.as_ptr(), self.buf, 8);
        }
        let setup_phys = self.buf_phys;

        // Данные — следом за setup.
        let (data_phys, data_len) = match &data {
            Some(slice) if !slice.is_empty() => {
                let len = slice.len().min(self.buf_len - 16);
                unsafe {
                    if !dir_in {
                        core::ptr::copy_nonoverlapping(
                            slice.as_ptr(),
                            self.buf.add(16),
                            len,
                        );
                    }
                }
                (self.buf_phys + 16, len)
            }
            _ => (0, 0),
        };

        let token_base = ((address as u32) & 0x7F) << 8;
        let _ = (low_speed, max_packet);

        unsafe {
            // TD 0: SETUP
            let td0 = self.tds;
            *td0 = Td::empty();
            // SETUP всегда 8 байт, независимо от max packet устройства.
            (*td0).token = PID_SETUP | token_base | max_len_field(8);
            (*td0).status = TD_ACTIVE;
            (*td0).buffers[0] = setup_phys;

            let mut used = 1usize;

            // TD 1: DATA (если есть)
            if data_len > 0 {
                let td1 = self.tds.add(1);
                *td1 = Td::empty();
                let pid = if dir_in { PID_IN } else { PID_OUT };
                (*td1).token = pid | token_base | (1 << 19) | max_len_field(data_len);
                (*td1).status = TD_ACTIVE;
                (*td1).buffers[0] = data_phys;
                used = 2;
            }

            // Последний TD: STATUS (нулевая длина, направление противоположно)
            let status_td = self.tds.add(used);
            *status_td = Td::empty();
            let status_pid = if data_len > 0 && dir_in { PID_OUT } else { PID_IN };
            // Status-стадия — пакет нулевой длины.
            (*status_td).token = status_pid | token_base | (1 << 19) | max_len_field(0);
            (*status_td).status = TD_ACTIVE;
            (*status_td).buffers[0] = 0;
            used += 1;

            let result = self.run_chain(0, used);
            if result.is_err() {
                let mut index = 0usize;
                while index < used {
                    let td = self.tds.add(index);
                    crate::println!(
                        "[UHCI] control TD{} status={:#010x} token={:#010x}",
                        index, (*td).status, (*td).token
                    );
                    index += 1;
                }
                return Err(());
            }

            // Для IN-передачи данные лежат в staging-буфере.
            if data_len > 0 && dir_in {
                if let Some(slice) = data {
                    let td1 = self.tds.add(1);
                    let got = ((*td1).actual_len() as usize).min(data_len);
                    let got = got.min(slice.len());
                    core::ptr::copy_nonoverlapping(self.buf.add(16), slice.as_mut_ptr(), got);
                    return Ok(got);
                }
            }
            Ok(data_len)
        }
    }

    /// Bulk-передача одним TD (максимум 64 КиБ на пакетный лимит UHCI).
    fn bulk(
        &mut self,
        address: u8,
        endpoint: u8,
        _max_packet: u8,
        dir_in: bool,
        toggle: &mut bool,
        data: &mut [u8],
    ) -> Result<usize, ()> {
        if data.is_empty() {
            return Ok(0);
        }
        let len = data.len().min(self.buf_len);
        let pid = if dir_in { PID_IN } else { PID_OUT };

        unsafe {
            if !dir_in {
                core::ptr::copy_nonoverlapping(data.as_ptr(), self.buf, len);
            }
            let td = self.tds;
            *td = Td::empty();
            (*td).token = pid
                | ((address as u32) & 0x7F) << 8
                | ((endpoint as u32) & 0x0F) << 15
                | if *toggle { 1 << 19 } else { 0 }
                | max_len_field(len);
            (*td).status = TD_ACTIVE;
            (*td).buffers[0] = self.buf_phys;
        }

        self.run_chain(0, 1)?;

        unsafe {
            let td = self.tds;
            // Контроллер обновляет DT в самом TD — забираем его.
            *toggle = (*td).token & (1 << 19) != 0;
            let received = ((*td).actual_len() as usize).min(len);
            if dir_in && received > 0 {
                core::ptr::copy_nonoverlapping(self.buf, data.as_mut_ptr(), received);
            }
            Ok(if dir_in { received } else { len })
        }
    }
}

// ==========================================
// Перечисление
// ==========================================

fn descriptor_lengths(buf: &[u8]) -> Option<(u8, u8, u8)> {
    if buf.len() < 18 || buf[1] != 0x01 {
        return None;
    }
    Some((buf[4], buf[5], buf[6])) // class, subclass, protocol
}

/// Опросить все порты всех контроллеров и настроить найденные устройства.
pub fn enumerate() -> usize {
    let mut controllers = CONTROLLERS.lock();
    let mut found = Vec::new();

    for controller_index in 0..controllers.len() {
        let ports = controllers[controller_index].port_count();
        for port in 1..=ports {
            let status = controllers[controller_index].portsc(port);
            if status & PORT_CCS == 0 {
                continue;
            }
            if DEVICES
                .lock()
                .iter()
                .any(|d| d.controller == controller_index && d.port == port)
            {
                continue;
            }

            let Some(low_speed) = controllers[controller_index].port_reset(port) else {
                println!("[UHCI] port {}: reset failed", port);
                continue;
            };

            match configure_device(&mut controllers[controller_index], controller_index, port, low_speed)
            {
                Some(device) => {
                    println!(
                        "[UHCI] port {}: {} device {:04x}:{:04x} addr {} ({})",
                        port,
                        if low_speed { "low-speed" } else { "full-speed" },
                        device.vendor,
                        device.product,
                        device.address,
                        if device.max_packet == 8 { "8 B" } else { "64 B" }
                    );
                    found.push(device);
                }
                None => println!("[UHCI] port {}: enumeration failed", port),
            }
        }
    }

    if found.is_empty() {
        return 0;
    }
    let count = found.len();
    DEVICES.lock().extend(found);
    count
}

fn configure_device(
    controller: &mut Uhci,
    controller_index: usize,
    port: u8,
    low_speed: bool,
) -> Option<UhciDevice> {
    // Адрес 0: GET_DESCRIPTOR(device) с максимальным пакетом 8.
    let mut descriptor = [0u8; 18];
    let setup = [0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 18, 0];
    controller
        .control(0, low_speed, 8, &setup, Some(&mut descriptor), true)
        .ok()?;
    let (class, _subclass, _protocol) = descriptor_lengths(&descriptor)?;
    let vendor = u16::from_le_bytes([descriptor[8], descriptor[9]]);
    let product = u16::from_le_bytes([descriptor[10], descriptor[11]]);
    let max_packet = descriptor[7].max(8);

    // SET_ADDRESS
    let address = 1 + (DEVICES.lock().len() as u8 % 100);
    let setup = [0x00, 0x05, address, 0, 0, 0, 0, 0];
    controller.control(0, low_speed, 8, &setup, None, false).ok()?;
    // Спецификация: не меньше 2 мс после SET_ADDRESS.
    for _ in 0..200_000 {
        core::hint::spin_loop();
    }

    // GET_DESCRIPTOR(device) уже с новым адресом.
    let mut descriptor = [0u8; 18];
    let setup = [0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 18, 0];
    controller
        .control(address, low_speed, max_packet, &setup, Some(&mut descriptor), true)
        .ok()?;

    // Конфигурация: читаем заголовок, затем целиком.
    let mut config_head = [0u8; 9];
    let setup = [0x80, 0x06, 0x00, 0x02, 0x00, 0x00, 9, 0];
    controller
        .control(address, low_speed, max_packet, &setup, Some(&mut config_head), true)
        .ok()?;
    let total = u16::from_le_bytes([config_head[2], config_head[3]]) as usize;
    if !(9..=512).contains(&total) {
        return None;
    }
    let mut config = alloc::vec![0u8; total];
    let setup = [0x80, 0x06, 0x00, 0x02, 0x00, 0x00, total as u8, (total >> 8) as u8];
    controller
        .control(address, low_speed, max_packet, &setup, Some(&mut config), true)
        .ok()?;

    let config_value = config[5];
    let setup = [0x00, 0x09, config_value, 0, 0, 0, 0, 0];
    controller.control(address, low_speed, max_packet, &setup, None, false).ok()?;

    let device = UhciDevice {
        controller: controller_index,
        port,
        address,
        low_speed,
        max_packet,
        vendor,
        product,
    };

    // HID boot keyboard?
    if class == 0 {
        if let Some((interface, endpoint)) = find_hid_keyboard(&config) {
            if setup_keyboard(controller, &device, interface, endpoint, &config).is_some() {
                println!("[UHCI] keyboard ready on port {} (interface {}, ep {:#x})", port, interface, endpoint);
            }
        }
    }

    Some(device)
}

/// Ищем интерфейс HID boot keyboard и его interrupt IN endpoint.
fn find_hid_keyboard(config: &[u8]) -> Option<(u8, u8)> {
    let mut offset = 0usize;
    let mut current_interface: Option<(u8, bool)> = None;
    while offset + 2 <= config.len() {
        let length = config[offset] as usize;
        let kind = config[offset + 1];
        if length < 2 || offset + length > config.len() {
            break;
        }
        match kind {
            0x04 if length >= 9 => {
                let number = config[offset + 2];
                let class = config[offset + 5];
                let subclass = config[offset + 6];
                let protocol = config[offset + 7];
                current_interface = Some((number, class == 3 && subclass == 1 && protocol == 1));
            }
            0x05 if length >= 7 => {
                if let Some((number, is_keyboard)) = current_interface {
                    if is_keyboard {
                        let address = config[offset + 2];
                        let attributes = config[offset + 3];
                        let is_interrupt_in = attributes & 0x03 == 0x03 && address & 0x80 != 0;
                        if is_interrupt_in {
                            return Some((number, address & 0x0F));
                        }
                    }
                }
            }
            _ => {}
        }
        offset += length;
    }
    None
}

fn setup_keyboard(
    controller: &mut Uhci,
    device: &UhciDevice,
    interface: u8,
    endpoint: u8,
    config: &[u8],
) -> Option<()> {
    let _ = device.max_packet;
    // SET_PROTOCOL(boot) и SET_IDLE(0) для интерфейса.
    let setup = [0x21, 0x0B, 0x00, 0x00, interface, 0, 0, 0];
    let _ = controller.control(device.address, device.low_speed, device.max_packet, &setup, None, false);
    let setup = [0x21, 0x0A, 0x00, 0x00, interface, 0, 0, 0];
    let _ = controller.control(device.address, device.low_speed, device.max_packet, &setup, None, false);

    let (report, report_phys) = alloc_low(8, 16)?;
    let (qh, qh_phys) = alloc_low(core::mem::size_of::<Qh>(), 16)?;
    let (td, td_phys) = alloc_low(core::mem::size_of::<Td>(), 16)?;

    let max_packet = endpoint_max_packet(config, endpoint).unwrap_or(8);

    unsafe {
        let qh = qh as *mut Qh;
        qh.write_volatile(Qh::empty());
        let td = td as *mut Td;
        *td = Td::empty();
        (*td).token = PID_IN
            | ((device.address as u32) & 0x7F) << 8
            | ((endpoint as u32) & 0x0F) << 15
            | max_len_field(max_packet as usize);
        (*td).status = TD_ACTIVE;
        (*td).buffers[0] = report_phys;
        (*qh).element = td_phys;
    }

    KEYBOARDS.lock().push(KeyboardSlot {
        device: UhciDevice {
            controller: device.controller,
            port: device.port,
            address: device.address,
            low_speed: device.low_speed,
            max_packet: device.max_packet,
            vendor: device.vendor,
            product: device.product,
        },
        interface,
        endpoint,
        toggle: false,
        qh: qh as *mut Qh,
        qh_phys,
        td: td as *mut Td,
        td_phys,
        report,
        report_phys,
        report_len: max_packet as usize,
        last_report: [0; 8],
    });

    // Прицепляем QH клавиатуры к цепочке: ctrl_qh.head -> kb_qh -> ...
    // Так контроллер опрашивает клавиатуру в каждом кадре.
    unsafe {
        let new_qh = qh as *mut Qh;
        (*new_qh).head = (*controller.qh).head;
        (*controller.qh).head = qh_phys | 2;
    }
    Some(())
}

fn endpoint_max_packet(config: &[u8], endpoint: u8) -> Option<u8> {
    let mut offset = 0usize;
    while offset + 2 <= config.len() {
        let length = config[offset] as usize;
        let kind = config[offset + 1];
        if length < 2 || offset + length > config.len() {
            break;
        }
        if kind == 0x05 && length >= 7 && config[offset + 2] & 0x0F == endpoint {
            let mps = u16::from_le_bytes([config[offset + 4], config[offset + 5]]);
            if mps > 0 && mps <= 64 {
                return Some(mps as u8);
            }
        }
        offset += length;
    }
    None
}

// ==========================================
// Опрос клавиатуры
// ==========================================

/// Опросить HID-клавиатуры и передать отчёты в общий HID-слой.
pub fn poll_keyboard() {
    if !READY.load(Ordering::Relaxed) {
        return;
    }

    let mut keyboards = match KEYBOARDS.try_lock() {
        Some(guard) => guard,
        None => return,
    };

    for slot in keyboards.iter_mut() {
        unsafe {
            let td = slot.td;
            if (*td).is_active() {
                continue;
            }
            if (*td).failed() {
                // STALL/timeout: перезапускаем TD, иначе клавиатура замолчит.
                (*td).status = TD_ACTIVE;
                (*slot.qh).element = slot.td_phys;
                continue;
            }

            let len = slot.report_len.min(8);
            let mut report = [0u8; 8];
            core::ptr::copy_nonoverlapping(slot.report, report.as_mut_ptr(), len);
            if report != slot.last_report {
                slot.last_report = report;
                crate::drivers::usb::hid::handle_boot_report(&report);
            }

            // Перезапуск TD для следующего отчёта. Контроллер после
            // завершения сам обнуляет QH.element (TD.link = Terminate),
            // поэтому ссылку надо восстановить.
            (*td).status = TD_ACTIVE;
            (*slot.qh).element = slot.td_phys;
        }
    }
}

pub fn keyboard_count() -> usize {
    KEYBOARDS.lock().len()
}

pub fn controller_count() -> usize {
    CONTROLLERS.lock().len()
}

pub fn devices() -> Vec<UhciDevice> {
    DEVICES.lock().iter().map(|d| UhciDevice {
        controller: d.controller,
        port: d.port,
        address: d.address,
        low_speed: d.low_speed,
        max_packet: d.max_packet,
        vendor: d.vendor,
        product: d.product,
    }).collect()
}
