// EHCI (USB 2.0) — хост-контроллер USB2.
//
// Зачем: на Haswell/Broadwell (ThinkPad T450) USB2-половина каждого
// USB3-разъёма принадлежит EHCI. Порты xHCI при этом читаются как пустые
// (`PORTSC=0x2a0`), и флешка не видна ни в одном порту, пока нет драйвера.
//
// Объём — только async schedule:
//   * периодический список нужен для interrupt-эндпоинтов, а BOT их не
//     использует (bulk-only), клавиатуру на EHCI не берём;
//   * split transactions не нужны: устройство high-speed на корневом порту.
//
// Транспорт тот же, что у xHCI: `storage.rs` делает BOT+SCSI и вызывает
// `bulk`/`resolve_stall` через `usb::UsbHost`. Здесь — только контроллер.
//
// DMA: все структуры и буферы обязаны лежать ниже 4 ГиБ (EHCI адресует
// 32-битную физику), поэтому `Dma::allocate` берёт память у кучи и
// проверяет адрес через `paging::dma_address`, отбрасывая высокие кадры.

use super::xhci::{
    find_storage_interface, parse_interfaces, InterfaceInfo, PortStatus, StorageInfo,
    UsbDeviceInfo, UsbError,
};
use crate::drivers::usb::UsbHost;
use crate::println;
use alloc::vec::Vec;
use core::alloc::Layout;
use core::sync::atomic::Ordering;
use spin::Mutex;

// --- Регистры (смещения от operational base) ---
const USBCMD: u64 = 0x00;
const USBSTS: u64 = 0x04;
const CTRLDSSEGMENT: u64 = 0x10;
const ASYNCLISTADDR: u64 = 0x18;
const CONFIGFLAG: u64 = 0x40;
const PORTSC: u64 = 0x44;

// USBCMD
const CMD_RS: u32 = 1 << 0;
const CMD_HCRESET: u32 = 1 << 1;
const CMD_PSE: u32 = 1 << 4;
const CMD_ASE: u32 = 1 << 5;

// USBSTS
const STS_USBINT: u32 = 1 << 0;
const STS_USBERRINT: u32 = 1 << 1;
const STS_PCD: u32 = 1 << 2;
const STS_FLR: u32 = 1 << 3;
const STS_HSE: u32 = 1 << 4;
const STS_IAA: u32 = 1 << 5;
const STS_HALT: u32 = 1 << 12;
const STS_ASS: u32 = 1 << 15;

// PORTSC
const PORT_CCS: u32 = 1 << 0;
const PORT_CSC: u32 = 1 << 1;
const PORT_PE: u32 = 1 << 2;
const PORT_PEC: u32 = 1 << 3;
const PORT_OCC: u32 = 1 << 5;
const PORT_PR: u32 = 1 << 8;
const PORT_PP: u32 = 1 << 12;
const PORT_SPEED_SHIFT: u32 = 26;
/// Биты, которые сбрасываются записью единицы.
const PORT_W1C: u32 = PORT_CSC | PORT_PEC | PORT_OCC;

// PID в qTD token (биты 8-9).
const PID_OUT: u32 = 0;
const PID_IN: u32 = 1;
const PID_SETUP: u32 = 2;

/// Бит Active в qTD token — **бит 7**, не 0.
const TD_ACTIVE: u32 = 1 << 7;
/// qTD завершён с halt (STALL).
const TD_HALTED: u32 = 1 << 1;
/// Ошибки qTD: data buffer error, babble, transaction error.
const TD_ERROR: u32 = (1 << 2) | (1 << 3) | (1 << 4);

/// Один qTD покрывает не больше 20 КиБ (пять 4К-страниц). Держим transfer
/// меньше: 16 КиБ, чтобы страниц было не больше четырёх.
const MAX_TRANSFER: usize = 16 * 1024;
const BULK_BUF: usize = 64 * 1024;
const CONTROL_DATA_MAX: usize = 4096;

/// Таймаут control-передачи. 2 с на реальном железе означает, что цикл
/// ввода стоит две секунды на каждой неудаче; рабочее устройство отвечает
/// на порядки быстрее.
const CONTROL_TIMEOUT_MS: u64 = 500;
/// Таймаут одной bulk-передачи: чтение сектора бывает медленным, но не
/// секундами.
const BULK_TIMEOUT_MS: u64 = 2_000;

#[inline]
unsafe fn rd32(addr: u64) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

#[inline]
unsafe fn wr32(addr: u64, value: u32) {
    core::ptr::write_volatile(addr as *mut u32, value);
}

/// Пауза. До инициализации таймера — прокрутка.
fn delay_ms(ms: u64) {
    if crate::timer::source_hz() > 0 {
        crate::timer::sleep_ms(ms);
    } else {
        for _ in 0..(ms as u32).saturating_mul(100_000) {
            core::hint::spin_loop();
        }
    }
}

// ==========================================
// DMA-память ниже 4 ГиБ
// ==========================================

struct Dma {
    ptr: *mut u8,
    phys: u32,
    layout: Layout,
}

impl Dma {
    fn allocate(size: usize, align: usize) -> Option<Dma> {
        let layout = Layout::from_size_align(size, align.max(32)).ok()?;
        for _ in 0..128 {
            let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) };
            if ptr.is_null() {
                return None;
            }
            if let Some(phys) = crate::memory::paging::dma_address(ptr as u64) {
                if phys != 0 && phys + size as u64 <= 0x1_0000_0000 {
                    return Some(Dma {
                        ptr,
                        phys: phys as u32,
                        layout,
                    });
                }
            }
            // Фрейм выше 4 ГиБ: EHCI его не увидит, возвращаем и пробуем ещё.
            unsafe { alloc::alloc::dealloc(ptr, layout) };
        }
        None
    }

    fn size(&self) -> usize {
        self.layout.size()
    }
}

impl Drop for Dma {
    fn drop(&mut self) {
        unsafe { alloc::alloc::dealloc(self.ptr, self.layout) };
    }
}

// SAFETY: память принадлежит контроллеру и используется под `EHCI`.
unsafe impl Send for Dma {}

// ==========================================
// Структуры расписания
// ==========================================

/// Queue Head.
///
/// Массив намеренно больше 32-битной раскладки (48 байт): при включённой
/// 64-битной адресации контроллер читает ещё `bufptr_hi[5]` (68 байт).
/// Старшие слова остаются нулями (память нулевая), то есть все адреса
/// лежат в младших 4 ГиБ.
#[repr(C, align(64))]
#[derive(Clone, Copy)]
struct Qh {
    words: [u32; 20],
}

impl Qh {
    fn configure(
        &mut self,
        address: u8,
        endpoint: u8,
        max_packet: u16,
        high_speed: bool,
        head: bool,
    ) {
        let eps = if high_speed { 2u32 } else { 0u32 };
        let mut word = ((max_packet as u32 & 0x7FF) << 16)
            | (eps << 12)
            | ((endpoint as u32 & 0x0F) << 8)
            | (address as u32 & 0x7F)
            | (8 << 28); // NAK reload
        if head {
            word |= 1 << 15;
        }
        // Control Endpoint Flag: для EP0 на full/low speed.
        if endpoint == 0 && !high_speed {
            word |= 1 << 27;
        }
        self.words[1] = word;
        self.words[2] = 1 << 30; // MULT = 1
        self.words[3] = 0;
        self.words[4] = 1; // overlay.next: terminate
        self.words[5] = 1; // overlay.alt next: terminate
        self.words[6] = 0;
        for index in 7..12 {
            self.words[index] = 0;
        }
    }

    /// Горизонтальная ссылка на следующий QH.
///
/// Биты 2:1 — тип: `01` = queue head, `00` = iTD. Ноль в бите 1 QEMU
/// (и железо) читает как изохронный дескриптор и останавливает контроллер
/// с «non queue head request in async schedule».
fn set_horizontal(&mut self, phys: u32) {
        self.words[0] = (phys & !0x1F) | 0x2;
    }

    fn terminate(&mut self) {
        self.words[0] = 1;
    }

    /// Начать transfer: overlay.next указывает на первый qTD.
///
/// Копировать сам qTD в overlay нельзя: QEMU (и железо) считают overlay
/// «своим» и на следующем проходе `ehci_verify_qh` видит расхождение,
/// сбрасывает очередь и отменяет уже отправленный пакет.
fn start(&mut self, first_qtd: u32) {
        self.words[4] = first_qtd;
        self.words[5] = 1;
        self.words[6] = 0;
    }

    fn stop(&mut self) {
        self.words[4] = 1;
        self.words[5] = 1;
        self.words[6] = 0;
    }
}

/// Transfer Descriptor.
///
/// Как и `Qh`: 32-битная раскладка — 32 байта, 64-битная — 52 байта со
/// `bufptr_hi[5]`. Пять лишних слов нужны, иначе контроллер читает за
/// концом структуры и берёт мусор как старшую половину адреса — transfer
/// уходит в никуда и завершается таймаутом.
#[repr(C, align(64))]
#[derive(Clone, Copy)]
struct Qtd {
    words: [u32; 13],
}

impl Qtd {
    fn build(
        next: u32,
        pid: u32,
        total: usize,
        dt: u32,
        ioc: bool,
        buffer_phys: u32,
        len: usize,
    ) -> Qtd {
        let mut words = [0u32; 13];
        words[0] = next;
        words[1] = 1; // alternate next: terminate
        words[2] = TD_ACTIVE
            | (pid << 8)
            | (3 << 10) // C_ERR = 3 попытки
            | (((total as u32) & 0x7FFF) << 16)
            | (dt << 31)
            | if ioc { 1 << 15 } else { 0 };

        let base = buffer_phys & !0xFFF;
        let offset = (buffer_phys & 0xFFF) as usize;
        let pages = ((offset + len + 4095) / 4096).clamp(1, 5);
        words[3] = buffer_phys;
        let mut index = 1usize;
        while index < pages {
            words[3 + index] = base + (index as u32) * 4096;
            index += 1;
        }
        // Остальные указатели дублируют последнюю страницу: контроллер их
        // не читает, но мусор в них затрудняет разбор дампов.
        while index < 5 {
            words[3 + index] = words[3 + pages - 1];
            index += 1;
        }
        Qtd { words }
    }
}

/// Ждать завершения qTD по времени, а не по числу итераций.
unsafe fn wait_qtd(qtd: *mut Qtd, timeout_ms: u64) -> Result<u32, UsbError> {
    let token_addr = qtd as u64 + 8;
    let start = crate::timer::uptime_ms();
    loop {
        let token = core::ptr::read_volatile(token_addr as *const u32);
        if token & TD_ACTIVE == 0 {
            return Ok(token);
        }
        if crate::timer::uptime_ms().saturating_sub(start) > timeout_ms {
            return Err(UsbError::Timeout);
        }
        core::hint::spin_loop();
    }
}

/// Токен qTD, прочитанный после завершения (для residual).
unsafe fn qtd_token(qtd: *mut Qtd) -> u32 {
    core::ptr::read_volatile((qtd as *const u8).add(8) as *const u32)
}

// ==========================================
// Контроллер
// ==========================================

struct DeviceState {
    port: u8,
    address: u8,
    /// Нормализованная скорость (как в xHCI): 1 full, 2 low, 3 high.
    speed: u32,
    high_speed: bool,
    ep0_max_packet: u16,
    vendor: u16,
    product: u16,
    class: u8,
    subclass: u8,
    protocol: u8,
    kind: &'static str,
}

struct StorageState {
    address: u8,
    port: u8,
    interface: u8,
    in_address: u8,
    out_address: u8,
    in_qh: Dma,
    out_qh: Dma,
    in_max_packet: u16,
    out_max_packet: u16,
    max_transfer: usize,
    high_speed: bool,
    vendor: u16,
    product: u16,
    speed: u32,
}

struct Controller {
    op: u64,
    mmio: u64,
    ports: u8,
    head: Dma,
    ctrl_qh: Dma,
    setup: Dma,
    data: Dma,
    root: Dma,
    next_address: u8,
    /// `HCCPARAMS[0]`: контроллер читает структуры в расширенной
    /// (64-битной) раскладке.
    addr64: bool,
    /// Сколько раз перечисление порта уже провалилось. Порт не трогаем
    /// повторно, пока не придёт новый CSC: иначе неудача повторяется из
    /// цикла ввода каждые полсекунды и превращается в «залипание» клавиатуры.
    port_failures: [u8; 16],
    /// Причина последней неудачи — для `diag usb`.
    port_reason: [&'static str; 16],
    devices: Vec<DeviceState>,
    storage: Vec<StorageState>,
}

// SAFETY: доступ строго под `EHCI`; ядро однопоточное до SMP.
unsafe impl Send for Controller {}

static EHCI: Mutex<Vec<Controller>> = Mutex::new(Vec::new());

impl Controller {
    fn port_reg(&self, port: u8) -> u64 {
        self.op + PORTSC + (port.saturating_sub(1) as u64) * 4
    }

    fn head_qh(&self) -> *mut Qh {
        self.head.ptr as *mut Qh
    }

    fn ctrl_qh(&self) -> *mut Qh {
        self.ctrl_qh.ptr as *mut Qh
    }

    /// Start/Run: RS=1, ASE=1. PSE не нужен.
    unsafe fn start(&mut self) {
        wr32(self.op + CONFIGFLAG, rd32(self.op + CONFIGFLAG) | 1);
        wr32(self.op + ASYNCLISTADDR, self.head.phys);
        let cmd = rd32(self.op + USBCMD) & !(CMD_PSE | CMD_HCRESET);
        wr32(self.op + USBCMD, cmd | CMD_RS | CMD_ASE);
        for _ in 0..2_000_000 {
            let status = rd32(self.op + USBSTS);
            if status & STS_HALT == 0 {
                break;
            }
            core::hint::spin_loop();
        }
    }

    unsafe fn reset(&mut self) -> bool {
        // Остановить, если запущен.
        let cmd = rd32(self.op + USBCMD);
        wr32(self.op + USBCMD, cmd & !CMD_RS);
        for _ in 0..2_000_000 {
            if rd32(self.op + USBSTS) & STS_HALT != 0 {
                break;
            }
            core::hint::spin_loop();
        }

        wr32(self.op + USBCMD, CMD_HCRESET);
        let mut ok = false;
        for _ in 0..4_000_000 {
            if rd32(self.op + USBCMD) & CMD_HCRESET == 0 {
                ok = true;
                break;
            }
            core::hint::spin_loop();
        }
        if !ok {
            return false;
        }
        for _ in 0..1_000_000 {
            if rd32(self.op + USBSTS) & STS_HALT != 0 {
                break;
            }
            core::hint::spin_loop();
        }
        wr32(
            self.op + USBSTS,
            STS_USBINT | STS_USBERRINT | STS_PCD | STS_FLR | STS_HSE | STS_IAA,
        );
        true
    }

    /// Отобрать порты у прошивки (EECP → USBLEGSUP).
    unsafe fn legacy_handoff(&mut self, eecp: u64) {
        if eecp == 0 {
            return;
        }
        let reg = self.mmio + eecp;
        let mut value = rd32(reg);
        if value & (1 << 16) != 0 {
            // BIOS owned: просим передать управление.
            wr32(reg, value | (1 << 24));
            for _ in 0..1_000_000 {
                value = rd32(reg);
                if value & (1 << 16) == 0 {
                    break;
                }
                core::hint::spin_loop();
            }
            value = rd32(reg);
        }
        // OS owned = 1, SMI enable = 0.
        wr32(reg, (value | (1 << 24)) & !1);
    }

    unsafe fn power_ports(&mut self) {
        for port in 1..=self.ports {
            // Пишем прочитанное значение (чтобы не сбросить PE/PP), плюс
            // W1C-биты: они очищают изменения, а не выставляются.
            let value = rd32(self.port_reg(port));
            wr32(self.port_reg(port), value | PORT_PP | PORT_W1C);
        }
        delay_ms(20);
    }

    /// Сбросить изменения порта, не трогая питание и enable.
    unsafe fn clear_port_changes(&self, port: u8) {
        let reg = self.port_reg(port);
        let value = rd32(reg);
        wr32(reg, value | PORT_PP | PORT_W1C);
    }

    /// Сброс порта. Возвращает нормализованную скорость.
    unsafe fn reset_port(&mut self, port: u8) -> Option<u32> {
        let reg = self.port_reg(port);
        let mut value = rd32(reg);
        if value & PORT_PP == 0 {
            wr32(reg, value | PORT_PP | PORT_W1C);
            delay_ms(20);
            value = rd32(reg);
        }
        if value & PORT_CCS == 0 {
            return None;
        }
        // Port Reset. На реальном железе PR снимает контроллер сам; QEMU
        // (и часть эмуляторов) завершает reset по записи PR=0, поэтому
        // после паузы снимаем бит явно, если он ещё стоит.
        wr32(reg, value | PORT_PP | PORT_PR);
        delay_ms(50);
        let after_delay = rd32(reg);
        if after_delay & PORT_PR != 0 {
            wr32(reg, (after_delay & !PORT_PR) | PORT_PP | PORT_W1C);
        }
        let start = crate::timer::uptime_ms();
        loop {
            let value = rd32(reg);
            if value & PORT_PR == 0 {
                break;
            }
            if crate::timer::source_hz() != 0
                && crate::timer::uptime_ms().saturating_sub(start) > 500
            {
                return None;
            }
            core::hint::spin_loop();
        }
        // Сбросить изменения и проверить, что порт включён. Значение читаем
        // заново: PE выставляет сам контроллер после reset.
        self.clear_port_changes(port);
        let value = rd32(reg);
        if value & PORT_PE == 0 {
            return None;
        }
        let raw = (value >> PORT_SPEED_SHIFT) & 0x3;
        // EHCI: 0 = full, 1 = low, 2 = high. QEMU вообще не заполняет эти
        // биты, но PE=1 на EHCI означает именно high-speed (full/low-speed
        // уходят в companion). Переводим в кодировку xHCI, чтобы
        // `speed_name` и `diag usb` говорили на одном языке.
        Some(match raw {
            0 if value & PORT_PE != 0 => 3,
            0 => 1,
            1 => 2,
            _ => 3,
        })
    }

    fn alloc_address(&mut self) -> u8 {
        self.next_address = if self.next_address == 0 || self.next_address >= 127 {
            1
        } else {
            self.next_address + 1
        };
        self.next_address
    }

    /// Один control transfer на EP0.
    #[allow(clippy::too_many_arguments)]
    unsafe fn control_raw(
        &mut self,
        address: u8,
        max_packet: u16,
        high_speed: bool,
        bm: u8,
        request: u8,
        value: u16,
        index: u16,
        data: Option<&mut [u8]>,
    ) -> Result<usize, UsbError> {
        let len = data.as_ref().map(|buffer| buffer.len()).unwrap_or(0);
        if len > CONTROL_DATA_MAX {
            return Err(UsbError::Unsupported);
        }
        let dir_in = bm & 0x80 != 0;

        let setup = [
            bm,
            request,
            value as u8,
            (value >> 8) as u8,
            index as u8,
            (index >> 8) as u8,
            len as u8,
            (len >> 8) as u8,
        ];
        core::ptr::copy_nonoverlapping(setup.as_ptr(), self.setup.ptr, setup.len());
        if len > 0 && !dir_in {
            if let Some(buffer) = data.as_ref() {
                core::ptr::copy_nonoverlapping(buffer.as_ptr(), self.data.ptr, len);
            }
        }

        let qtd_dma = Dma::allocate(
            3 * core::mem::size_of::<Qtd>(),
            core::mem::align_of::<Qtd>(),
        )
        .ok_or(UsbError::NoMemory)?;
        let qtds = qtd_dma.ptr as *mut Qtd;
        let base = qtd_dma.phys;
        // Шаг между qTD — размер структуры, а не былое «32 байта»:
        // в 64-битной раскладке он другой, и жёсткие смещения ломали цепочку.
        let stride = core::mem::size_of::<Qtd>() as u32;
        let (second, third) = (base + stride, base + 2 * stride);

        // 0: SETUP, 1: DATA (если есть), 2: STATUS.
        let q0 = Qtd::build(
            if len > 0 { second } else { third },
            PID_SETUP,
            8,
            0,
            false,
            self.setup.phys,
            8,
        );
        let q1 = Qtd::build(
            third,
            if dir_in { PID_IN } else { PID_OUT },
            len,
            1,
            false,
            self.data.phys,
            len,
        );
        // Status stage идёт в противоположном направлении к data stage.
        let status_pid = if len > 0 && dir_in { PID_OUT } else { PID_IN };
        let q2 = Qtd::build(1, status_pid, 0, 1, true, self.data.phys, 0);

        core::ptr::write_volatile(qtds, q0);
        core::ptr::write_volatile(qtds.add(1), q1);
        core::ptr::write_volatile(qtds.add(2), q2);

        let head_phys = self.head.phys;
        let qh_phys = self.ctrl_qh.phys;
        (*self.ctrl_qh()).configure(address, 0, max_packet, high_speed, false);
        (*self.ctrl_qh()).start(base);
        (*self.ctrl_qh()).set_horizontal(head_phys);
        (*self.head_qh()).set_horizontal(qh_phys);
        core::arch::asm!("sfence", options(nostack, preserves_flags));

        let result = wait_qtd(qtds.add(2), CONTROL_TIMEOUT_MS);

        // Снять QH со списка до освобождения qTD.
        (*self.head_qh()).set_horizontal(head_phys);
        (*self.ctrl_qh()).terminate();
        (*self.ctrl_qh()).stop();
        core::arch::asm!("sfence", options(nostack, preserves_flags));

        let _ = result?;
        // Проверяем оба этапа, а не только status: ошибка data stage при
        // успешном status иначе выглядела бы как успех.
        for qtd in [qtds, qtds.add(1)] {
            let token = qtd_token(qtd);
            if token & TD_HALTED != 0 {
                return Err(UsbError::Stall);
            }
            if token & TD_ERROR != 0 {
                return Err(UsbError::TransferFailed);
            }
        }

        if len > 0 && dir_in {
            if let Some(buffer) = data {
                let residual = ((qtd_token(qtds.add(1)) >> 16) & 0x7FFF) as usize;
                let got = len.saturating_sub(residual).min(buffer.len());
                core::sync::atomic::compiler_fence(Ordering::SeqCst);
                core::ptr::copy_nonoverlapping(self.data.ptr, buffer.as_mut_ptr(), got);
                return Ok(got);
            }
        }
        Ok(len)
    }

    unsafe fn bulk_inner(
        &mut self,
        address: u8,
        dir_in: bool,
        data: &mut [u8],
        toggle: u32,
    ) -> Result<usize, UsbError> {
        if data.is_empty() {
            return Ok(0);
        }
        let Some(index) = self.storage.iter().position(|d| d.address == address) else {
            return Err(UsbError::NoDevice);
        };
        let (qh_phys, max_packet, max_transfer, high_speed, endpoint) = {
            let storage = &self.storage[index];
            let (qh, max_packet, address_byte) = if dir_in {
                (&storage.in_qh, storage.in_max_packet, storage.in_address)
            } else {
                (&storage.out_qh, storage.out_max_packet, storage.out_address)
            };
            (
                qh.phys,
                max_packet,
                storage.max_transfer.min(self.root.size()),
                storage.high_speed,
                address_byte & 0x0F,
            )
        };
        let qh = qh_phys as *mut Qh;

        let mut done = 0usize;
        let mut dt = toggle & 1;
        while done < data.len() {
            let chunk = (data.len() - done).min(max_transfer);
            if !dir_in {
                core::ptr::copy_nonoverlapping(data.as_ptr().add(done), self.root.ptr, chunk);
            }

            let qtd_dma = Dma::allocate(
                core::mem::size_of::<Qtd>(),
                core::mem::align_of::<Qtd>(),
            )
            .ok_or(UsbError::NoMemory)?;
            let qtd = qtd_dma.ptr as *mut Qtd;
            let qtd_value = Qtd::build(
                1,
                if dir_in { PID_IN } else { PID_OUT },
                chunk,
                dt,
                true,
                self.root.phys,
                chunk,
            );
            core::ptr::write_volatile(qtd, qtd_value);

            let head_phys = self.head.phys;
            (*qh).configure(address, endpoint, max_packet, high_speed, false);
            (*qh).start(qtd_dma.phys);
            (*qh).set_horizontal(head_phys);
            (*self.head_qh()).set_horizontal(qh_phys);
            core::arch::asm!("sfence", options(nostack, preserves_flags));

            let result = wait_qtd(qtd, BULK_TIMEOUT_MS);

            (*self.head_qh()).set_horizontal(head_phys);
            (*qh).terminate();
            (*qh).stop();
            core::arch::asm!("sfence", options(nostack, preserves_flags));

            let token = result?;
            if token & TD_HALTED != 0 {
                return Err(UsbError::Stall);
            }
            if token & TD_ERROR != 0 {
                return Err(UsbError::TransferFailed);
            }
            let residual = ((token >> 16) & 0x7FFF) as usize;
            let transferred = chunk.saturating_sub(residual);
            if dir_in && transferred > 0 {
                core::sync::atomic::compiler_fence(Ordering::SeqCst);
                core::ptr::copy_nonoverlapping(
                    self.root.ptr,
                    data.as_mut_ptr().add(done),
                    transferred,
                );
            }
            if !dir_in && transferred != chunk {
                return Err(UsbError::TransferFailed);
            }
            done += transferred;

            // Data toggle для следующего chunk: считаем пакеты, включая
            // короткий последний.
            let packets = (transferred + max_packet as usize - 1) / max_packet as usize;
            dt ^= (packets & 1) as u32;
            if transferred < chunk {
                break;
            }
        }
        Ok(done)
    }

    /// Сбросить порт и перечислить устройство на нём.
    ///
    /// Возвращает причину отказа, чтобы `diag usb` и лог говорили, на каком
    /// шаге сорвалось, а не просто «не получилось».
    unsafe fn enumerate_port(&mut self, port: u8) -> Result<(), &'static str> {
        let Some(speed) = self.reset_port(port) else {
            return Err("port reset failed");
        };
        let high_speed = speed == 3;
        println!(
            "[EHCI] port {}: connected, {} speed",
            port,
            if high_speed { "high" } else { "full/low" }
        );

        // Адрес 0: EP0 ещё не адресован.
        let mut ep0_max_packet = if high_speed { 64u16 } else { 8 };
        let mut short = [0u8; 8];
        if self
            .control_raw(0, ep0_max_packet, high_speed, 0x80, 6, 0x0100, 0, Some(&mut short))
            .is_err()
        {
            return Err("GET_DESCRIPTOR(8) at address 0 failed");
        }
        if ep0_max_packet == 8 {
            ep0_max_packet = match short[7] {
                3..=6 => 1u16 << short[7],
                9 => 512,
                _ => 8,
            };
        }

        let address = self.alloc_address();
        if self
            .control_raw(0, ep0_max_packet, high_speed, 0x00, 5, address as u16, 0, None)
            .is_err()
        {
            return Err("SET_ADDRESS failed");
        }
        delay_ms(2);

        let mut descriptor = [0u8; 18];
        if self
            .control_raw(
                address,
                ep0_max_packet,
                high_speed,
                0x80,
                6,
                0x0100,
                0,
                Some(&mut descriptor),
            )
            .is_err()
        {
            return Err("GET_DESCRIPTOR(device) failed");
        }
        let vendor = u16::from_le_bytes([descriptor[8], descriptor[9]]);
        let product = u16::from_le_bytes([descriptor[10], descriptor[11]]);
        if let Some(parsed) = crate::drivers::usb::parse_device_descriptor(&descriptor) {
            crate::drivers::usb::register_device_from_descriptor(&parsed);
        }

        let mut header = [0u8; 9];
        if self
            .control_raw(
                address,
                ep0_max_packet,
                high_speed,
                0x80,
                6,
                0x0200,
                0,
                Some(&mut header),
            )
            .is_err()
        {
            return Err("GET_DESCRIPTOR(config header) failed");
        }
        let total = u16::from_le_bytes([header[2], header[3]]) as usize;
        if total < 9 || total > CONTROL_DATA_MAX {
            return Err("config descriptor length is invalid");
        }
        let mut config = alloc::vec![0u8; total];
        if self
            .control_raw(
                address,
                ep0_max_packet,
                high_speed,
                0x80,
                6,
                0x0200,
                0,
                Some(&mut config),
            )
            .is_err()
        {
            return Err("GET_DESCRIPTOR(config) failed");
        }
        let config_value = config[5];
        let mut interfaces = Vec::new();
        parse_interfaces(&config, &mut interfaces);

        if self
            .control_raw(
                address,
                ep0_max_packet,
                high_speed,
                0x00,
                9,
                config_value as u16,
                0,
                None,
            )
            .is_err()
        {
            return Err("SET_CONFIGURATION failed");
        }

        // Класс 09 — хаб. Перечислить его самого мы можем (и это уже
        // полезно: видно, что порт занят хабом, а не пуст), но устройства
        // за ним потребуют hub class requests — это rc.25.1.
        let mut kind = "unsupported";
        let mut class = descriptor[4];
        let mut subclass = descriptor[5];
        let mut protocol = descriptor[6];
        if class == 0x09 {
            kind = "hub";
        } else if let Some(interface) = find_storage_interface(&interfaces) {
            class = interface.class;
            subclass = interface.subclass;
            protocol = interface.protocol;
            if self.setup_storage(address, port, speed, high_speed, vendor, product, &interface) {
                kind = "storage";
            }
        }

        println!(
            "[EHCI] addr {}: {:04x}:{:04x} class {:02x}/{:02x}/{:02x} -> {}",
            address, vendor, product, class, subclass, protocol, kind
        );
        if kind == "hub" {
            println!(
                "[EHCI] port {}: devices behind this hub are not reachable yet \
                 (class 09 needs hub requests)",
                port
            );
        }
        self.devices.push(DeviceState {
            port,
            address,
            speed,
            high_speed,
            ep0_max_packet,
            vendor,
            product,
            class,
            subclass,
            protocol,
            kind,
        });
        Ok(())
    }

    unsafe fn setup_storage(
        &mut self,
        address: u8,
        port: u8,
        speed: u32,
        high_speed: bool,
        vendor: u16,
        product: u16,
        interface: &InterfaceInfo,
    ) -> bool {
        let (Some(in_ep), Some(out_ep)) = (interface.in_ep, interface.out_ep) else {
            println!("[EHCI] addr {}: storage without bulk endpoints", address);
            return false;
        };
        let Some(in_qh) =
            Dma::allocate(core::mem::size_of::<Qh>(), core::mem::align_of::<Qh>())
        else {
            return false;
        };
        let Some(out_qh) =
            Dma::allocate(core::mem::size_of::<Qh>(), core::mem::align_of::<Qh>())
        else {
            return false;
        };
        let max_packet = if high_speed {
            512
        } else {
            in_ep.max_packet.clamp(8, 64)
        };
        (*(in_qh.ptr as *mut Qh)).configure(
            address,
            in_ep.address & 0x0F,
            max_packet,
            high_speed,
            false,
        );
        (*(out_qh.ptr as *mut Qh)).configure(
            address,
            out_ep.address & 0x0F,
            max_packet,
            high_speed,
            false,
        );
        let max_transfer = MAX_TRANSFER.min(self.root.size());
        self.storage.push(StorageState {
            address,
            port,
            interface: interface.number,
            in_address: in_ep.address,
            out_address: out_ep.address,
            in_qh,
            out_qh,
            in_max_packet: max_packet,
            out_max_packet: max_packet,
            max_transfer,
            high_speed,
            vendor,
            product,
            speed,
        });
        println!(
            "[EHCI] addr {}: storage ep in={:#04x} out={:#04x} mps={}",
            address, in_ep.address, out_ep.address, max_packet
        );
        true
    }

    fn remove_port(&mut self, port: u8) {
        self.storage.retain(|entry| entry.port != port);
        self.devices.retain(|entry| entry.port != port);
    }
}

// ==========================================
// Публичный API
// ==========================================

/// Инициализировать контроллер по BAR0. Перечисление — отдельно
/// (`enumerate_now`), после инициализации таймера.
pub fn init_controller(bar_phys: u64, mmio_virt: u64) -> bool {
    unsafe {
        let cap0 = rd32(mmio_virt);
        let cap_len = (cap0 & 0xFF) as u8;
        let version = (cap0 >> 16) as u16;
        if (version >> 8) != 1 {
            println!(
                "[EHCI] {:#x}: interface version {}.{} is not EHCI 1.x",
                bar_phys,
                version >> 8,
                version & 0xFF
            );
            return false;
        }
        let hcs_params = rd32(mmio_virt + 0x04);
        let hcc_params = rd32(mmio_virt + 0x08);
        let ports = (hcs_params & 0xF) as u8;
        if ports == 0 {
            println!("[EHCI] {:#x}: no ports", bar_phys);
            return false;
        }
        // HCCPARAMS[0] — 64-битная адресация. От неё зависит раскладка
        // QH/qTD: при включённом бите контроллер читает дополнительные
        // `bufptr_hi[5]` и берёт оттуда старшую половину адреса.
        let addr64 = hcc_params & 1 != 0;
        println!(
            "[EHCI] {:#x}: v{}.{} ports={} PPC={} 64bit={} EECP={:#x}",
            bar_phys,
            version >> 8,
            version & 0xFF,
            ports,
            (hcs_params >> 4) & 1,
            if addr64 { "yes" } else { "no" },
            ((hcc_params >> 8) & 0xFF) as u64 * 4
        );

        let Some(head) =
            Dma::allocate(core::mem::size_of::<Qh>(), core::mem::align_of::<Qh>())
        else {
            println!("[EHCI] {:#x}: no low memory for async head", bar_phys);
            return false;
        };
        let Some(ctrl_qh) =
            Dma::allocate(core::mem::size_of::<Qh>(), core::mem::align_of::<Qh>())
        else {
            return false;
        };
        let Some(setup) = Dma::allocate(64, 64) else {
            return false;
        };
        let Some(data) = Dma::allocate(CONTROL_DATA_MAX, 4096) else {
            return false;
        };
        let Some(root) = Dma::allocate(BULK_BUF, 4096) else {
            return false;
        };

        let mut controller = Controller {
            op: mmio_virt + cap_len as u64,
            mmio: mmio_virt,
            ports,
            head,
            ctrl_qh,
            setup,
            data,
            root,
            next_address: 0,
            addr64,
            port_failures: [0; 16],
            port_reason: [""; 16],
            devices: Vec::new(),
            storage: Vec::new(),
        };

        if !controller.reset() {
            println!("[EHCI] {:#x}: reset timeout", bar_phys);
            return false;
        }

        if addr64 {
            // Все DMA-буферы ниже 4 ГиБ, поэтому сегмент нулевой. Без этой
            // записи контроллер может подставить в адрес мусор из регистра.
            wr32(controller.op + CTRLDSSEGMENT, 0);
        }

        let eecp = ((hcc_params >> 8) & 0xFF) as u64 * 4;
        controller.legacy_handoff(eecp);

        // Голова async-списка: H=1 и петля на себя.
        let head_phys = controller.head.phys;
        (*controller.head_qh()).configure(0, 0, 64, true, true);
        // Halted в overlay: голова списка не должна искать qTD, только
        // передавать управление следующему QH.
        (*controller.head_qh()).words[6] = 0x2;
        (*controller.head_qh()).set_horizontal(head_phys);
        (*controller.ctrl_qh()).configure(0, 0, 8, false, false);
        (*controller.ctrl_qh()).terminate();

        controller.start();
        controller.power_ports();

        let status = rd32(controller.op + USBSTS);
        println!(
            "[EHCI] {:#x}: started (ASS={}, HCHalted={})",
            bar_phys,
            (status & STS_ASS != 0) as u8,
            (status & STS_HALT != 0) as u8
        );

        EHCI.lock().push(controller);
        true
    }
}

impl Controller {
    /// Разобрать состояние одного порта.
    ///
    /// `force` игнорирует счётчик неудач (явный запрос из `diag usb`).
    /// Возвращает `(подключено, отключено)` для этого порта.
    unsafe fn service_port(&mut self, port: u8, force: bool) -> (usize, usize) {
        let value = rd32(self.port_reg(port));
        let connected = value & PORT_CCS != 0;
        let csc = value & PORT_CSC != 0;
        let known = self.devices.iter().any(|entry| entry.port == port);

        if !csc && connected == known {
            return (0, 0);
        }
        self.clear_port_changes(port);

        // Новое физическое событие: порт переподключили, значит попытку
        // перечисления надо разрешить заново.
        if csc {
            self.port_failures[port as usize] = 0;
            self.port_reason[port as usize] = "";
        }

        if connected && !known {
            if self.port_failures[port as usize] > 0 && !force {
                // Уже пробовали и сорвались: не повторяем на каждом опросе,
                // иначе блокирующее перечисление превращается в залипание
                // клавиатуры. Ждём нового CSC (вынул/вставил).
                return (0, 0);
            }
            println!("[EHCI] hotplug: port {} connected", port);
            return match self.enumerate_port(port) {
                Ok(()) => {
                    self.port_failures[port as usize] = 0;
                    self.port_reason[port as usize] = "";
                    (1, 0)
                }
                Err(reason) => {
                    self.port_failures[port as usize] =
                        self.port_failures[port as usize].saturating_add(1);
                    self.port_reason[port as usize] = reason;
                    println!(
                        "[EHCI] port {}: enumeration failed ({}) — not retrying until replug",
                        port, reason
                    );
                    (0, 0)
                }
            };
        }

        if !connected && known {
            self.remove_port(port);
            println!("[EHCI] hotplug: port {} disconnected", port);
            return (0, 1);
        }

        (0, 0)
    }
}

/// Перечислить устройства на всех портах. Вызывается после `timer::init()`.
pub fn enumerate_now() {
    let mut controllers = EHCI.lock();
    for controller in controllers.iter_mut() {
        for port in 1..=controller.ports {
            unsafe {
                controller.service_port(port, false);
            }
        }
    }
}

/// Обработать hotplug. Возвращает `(подключено, отключено)`.
///
/// Неудачная попытка запоминается по порту и больше не повторяется до
/// следующего события подключения: цикл ввода вызывает эту функцию каждые
/// 500 мс, а перечисление блокирующее.
pub fn poll_hotplug() -> (usize, usize) {
    let mut controllers = match EHCI.try_lock() {
        Some(guard) => guard,
        None => return (0, 0),
    };
    let mut added = 0usize;
    let mut removed = 0usize;
    for controller in controllers.iter_mut() {
        for port in 1..=controller.ports {
            let (plus, minus) = unsafe { controller.service_port(port, false) };
            added += plus;
            removed += minus;
        }
    }
    (added, removed)
}

/// Принудительно повторить перечисление портов, где оно сорвалось.
pub fn retry_failed() -> usize {
    let mut controllers = EHCI.lock();
    let mut attempted = 0usize;
    for controller in controllers.iter_mut() {
        for port in 1..=controller.ports {
            if controller.port_failures[port as usize] == 0 {
                continue;
            }
            controller.port_failures[port as usize] = 0;
            attempted += 1;
            unsafe {
                controller.service_port(port, true);
            }
        }
    }
    attempted
}

/// Состояние портов для `diag usb`: причина последней неудачи.
pub fn port_failures() -> Vec<(u8, u8, &'static str)> {
    let controllers = match EHCI.try_lock() {
        Some(guard) => guard,
        None => return Vec::new(),
    };
    let mut result = Vec::new();
    for controller in controllers.iter() {
        for port in 1..=controller.ports {
            let failures = controller.port_failures[port as usize];
            if failures == 0 {
                continue;
            }
            result.push((port, failures, controller.port_reason[port as usize]));
        }
    }
    result
}

/// 64-битная адресация контроллеров (для `diag usb`).
pub fn addressing() -> Vec<(bool, usize)> {
    let controllers = match EHCI.try_lock() {
        Some(guard) => guard,
        None => return Vec::new(),
    };
    controllers
        .iter()
        .map(|controller| (controller.addr64, controller.devices.len()))
        .collect()
}

/// Снимок корневых портов для `diag usb`.
pub fn ports() -> Vec<PortStatus> {
    let controllers = match EHCI.try_lock() {
        Some(guard) => guard,
        None => return Vec::new(),
    };
    let mut result = Vec::new();
    for (index, controller) in controllers.iter().enumerate() {
        for port in 1..=controller.ports {
            let portsc = unsafe { rd32(controller.port_reg(port)) };
            let raw_speed = (portsc >> PORT_SPEED_SHIFT) & 0x3;
            let speed = match raw_speed {
                0 if portsc & PORT_PE != 0 => 3,
                0 => 1,
                1 => 2,
                _ => 3,
            };
            let device = controller
                .devices
                .iter()
                .find(|entry| entry.port == port)
                .map(|entry| (entry.vendor, entry.product, entry.kind));
            result.push(PortStatus {
                controller: index,
                port,
                portsc,
                speed,
                connected: portsc & PORT_CCS != 0,
                enabled: portsc & PORT_PE != 0,
                powered: portsc & PORT_PP != 0,
                device,
            });
        }
    }
    result
}

pub fn devices() -> Vec<UsbDeviceInfo> {
    EHCI
        .lock()
        .iter()
        .enumerate()
        .flat_map(|(index, controller)| {
            controller.devices.iter().map(move |device| UsbDeviceInfo {
                controller: index,
                slot: device.address,
                port: device.port,
                speed: device.speed,
                vendor: device.vendor,
                product: device.product,
                class: device.class,
                subclass: device.subclass,
                protocol: device.protocol,
                kind: device.kind,
            })
        })
        .collect()
}

pub fn storage_devices() -> Vec<StorageInfo> {
    let mut result = Vec::new();
    for (index, controller) in EHCI.lock().iter().enumerate() {
        for storage in &controller.storage {
            result.push(StorageInfo {
                host: UsbHost::Ehci,
                controller: index,
                slot: storage.address,
                port: storage.port,
                vendor: storage.vendor,
                product: storage.product,
                max_transfer: storage.max_transfer,
                speed: storage.speed,
                max_burst: 0,
            });
        }
    }
    result
}

pub fn controller_count() -> usize {
    EHCI.lock().len()
}

/// Control transfer к устройству по адресу.
pub fn control(
    controller: usize,
    address: u8,
    bm: u8,
    request: u8,
    value: u16,
    index: u16,
    data: Option<&mut [u8]>,
) -> Result<usize, UsbError> {
    let mut controllers = EHCI.lock();
    let Some(ctrl) = controllers.get_mut(controller) else {
        return Err(UsbError::NoDevice);
    };
    let Some((max_packet, high_speed)) = ctrl
        .devices
        .iter()
        .find(|entry| entry.address == address)
        .map(|entry| (entry.ep0_max_packet, entry.high_speed))
    else {
        return Err(UsbError::NoDevice);
    };
    unsafe { ctrl.control_raw(address, max_packet, high_speed, bm, request, value, index, data) }
}

/// Bulk transfer. `toggle` — начальный data toggle (BOT: CBW=0, data/CSW=1).
pub fn bulk(
    controller: usize,
    address: u8,
    dir_in: bool,
    data: &mut [u8],
    toggle: u32,
) -> Result<usize, UsbError> {
    let mut controllers = EHCI.lock();
    let Some(ctrl) = controllers.get_mut(controller) else {
        return Err(UsbError::NoDevice);
    };
    unsafe { ctrl.bulk_inner(address, dir_in, data, toggle) }
}

/// BOT Bulk-Only Reset + ClearFeature(HALT) для обоих bulk-эндпоинтов.
pub fn resolve_stall(controller: usize, address: u8) -> Result<(), UsbError> {
    let (interface, in_address, out_address, ep0_max_packet, high_speed) = {
        let controllers = EHCI.lock();
        let Some(ctrl) = controllers.get(controller) else {
            return Err(UsbError::NoDevice);
        };
        let Some(storage) = ctrl.storage.iter().find(|entry| entry.address == address) else {
            return Err(UsbError::NoDevice);
        };
        let Some(device) = ctrl.devices.iter().find(|entry| entry.address == address) else {
            return Err(UsbError::NoDevice);
        };
        (
            storage.interface,
            storage.in_address,
            storage.out_address,
            device.ep0_max_packet,
            device.high_speed,
        )
    };

    let mut controllers = EHCI.lock();
    let Some(ctrl) = controllers.get_mut(controller) else {
        return Err(UsbError::NoDevice);
    };
    unsafe {
        let _ = ctrl.control_raw(
            address,
            ep0_max_packet,
            high_speed,
            0x21,
            0xFF,
            0,
            interface as u16,
            None,
        );
        let _ = ctrl.control_raw(
            address,
            ep0_max_packet,
            high_speed,
            0x02,
            1,
            0,
            in_address as u16,
            None,
        );
        let _ = ctrl.control_raw(
            address,
            ep0_max_packet,
            high_speed,
            0x02,
            1,
            0,
            out_address as u16,
            None,
        );
    }
    Ok(())
}