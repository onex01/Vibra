// GDT + TSS для Vibra OS.
//
// Раскладка (фиксирована с прицелом на будущий ring 3 / sysret):
//   0x00  null
//   0x08  kernel code 64-bit
//   0x10  kernel data
//   0x18  user data   (резерв, пока не используется)
//   0x20  user code   (резерв, пока не используется)
//   0x28  TSS (16-байтовый системный дескриптор — занимает слоты 0x28 и 0x30)

use crate::println;
use core::arch::asm;

pub const KERNEL_CS: u16 = 0x08;
pub const KERNEL_DS: u16 = 0x10;
pub const USER_DS: u16 = 0x1B;   // index 3 (0x18) | RPL 3
pub const USER_CS: u16 = 0x23;   // index 4 (0x20) | RPL 3
const TSS_SELECTOR: u16 = 0x28;

// Индексы IST в TSS (1-based в записи IDT!)
pub const DOUBLE_FAULT_IST_INDEX: u16 = 1;
pub const NMI_IST_INDEX: u16 = 2;

const IST_STACK_SIZE: usize = 5 * 4096; // 20 КБ

#[repr(align(16))]
struct IstStack([u8; IST_STACK_SIZE]);

static mut DF_STACK: IstStack = IstStack([0; IST_STACK_SIZE]);
static mut NMI_STACK: IstStack = IstStack([0; IST_STACK_SIZE]);

// Task State Segment (long mode). Хранит стеки для смены привилегий (rsp)
// и Interrupt Stack Table (ist) — гарантированно валидные стеки для критичных
// исключений вроде Double Fault.
#[repr(C, packed)]
struct Tss {
    _reserved0: u32,
    rsp: [u64; 3],
    _reserved1: u64,
    ist: [u64; 7],
    _reserved2: u64,
    _reserved3: u16,
    iomap_base: u16,
}

static mut TSS: Tss = Tss {
    _reserved0: 0,
    rsp: [0; 3],
    _reserved1: 0,
    ist: [0; 7],
    _reserved2: 0,
    _reserved3: 0,
    iomap_base: core::mem::size_of::<Tss>() as u16, // без IO bitmap
};

// GDT: 5 обычных дескрипторов + 2 слота под 16-байтовый TSS-дескриптор
static mut GDT: [u64; 7] = [0; 7];

#[repr(C, packed)]
struct GdtPointer {
    limit: u16,
    base: u64,
}

// Обычный сегментный дескриптор (в long mode базы/лимиты кода и данных игнорируются)
const fn segment_descriptor(executable: bool, user: bool, long_mode: bool) -> u64 {
    let mut desc: u64 = 0;
    desc |= 1 << 47; // Present
    desc |= 1 << 44; // S = code/data (не системный)
    desc |= 1 << 41; // RW (код: читаемый; данные: записываемые)
    if executable { desc |= 1 << 43; }
    if long_mode { desc |= 1 << 53; } // L-бит (только для кода)
    if user { desc |= 0b11 << 45; }   // DPL = 3
    desc
}

pub fn init() {
    unsafe {
        println!("[GDT] Setting up GDT + TSS...");

        // Заполняем IST-стеки в TSS (стек растёт вниз — кладём верхушку)
        let df_top = core::ptr::addr_of!(DF_STACK) as u64 + IST_STACK_SIZE as u64;
        let nmi_top = core::ptr::addr_of!(NMI_STACK) as u64 + IST_STACK_SIZE as u64;
        TSS.ist[(DOUBLE_FAULT_IST_INDEX - 1) as usize] = df_top;
        TSS.ist[(NMI_IST_INDEX - 1) as usize] = nmi_top;

        // Дескрипторы сегментов
        GDT[0] = 0;                                            // null
        GDT[1] = segment_descriptor(true, false, true);        // 0x08 kernel code
        GDT[2] = segment_descriptor(false, false, false);      // 0x10 kernel data
        GDT[3] = segment_descriptor(false, true, false);       // 0x18 user data (резерв)
        GDT[4] = segment_descriptor(true, true, true);         // 0x20 user code (резерв)

        // TSS-дескриптор: 16 байт, тип 0x9 = available 64-bit TSS
        let tss_base = core::ptr::addr_of!(TSS) as u64;
        let tss_limit = (core::mem::size_of::<Tss>() - 1) as u64;
        let mut low: u64 = 0;
        low |= tss_limit & 0xFFFF;                     // limit[15:0]
        low |= (tss_base & 0xFFFFFF) << 16;            // base[23:0]
        low |= 0x9 << 40;                              // type = available 64-bit TSS
        low |= 1 << 47;                                // Present
        low |= ((tss_limit >> 16) & 0xF) << 48;        // limit[19:16]
        low |= ((tss_base >> 24) & 0xFF) << 56;        // base[31:24]
        GDT[5] = low;
        GDT[6] = tss_base >> 32;                       // base[63:32] + reserved

        let gdt_ptr = GdtPointer {
            limit: (core::mem::size_of::<[u64; 7]>() - 1) as u16,
            base: core::ptr::addr_of!(GDT) as u64,
        };

        asm!("lgdt [{}]", in(reg) &gdt_ptr, options(readonly, nostack));

        // Перезагрузка CS требует far transfer: push селектор + адрес, retfq
        asm!(
            "push {sel}",
            "lea {tmp}, [rip + 2f]",
            "push {tmp}",
            "retfq",
            "2:",
            sel = in(reg) KERNEL_CS as u64,
            tmp = out(reg) _,
        );

        // Data-сегменты — обычными mov
        asm!(
            "mov ds, {0:x}",
            "mov es, {0:x}",
            "mov ss, {0:x}",
            "mov fs, {0:x}",
            "mov gs, {0:x}",
            in(reg) KERNEL_DS,
            options(nostack, preserves_flags),
        );

        // Загружаем Task Register
        asm!("ltr {0:x}", in(reg) TSS_SELECTOR, options(nostack, preserves_flags));

        println!("[GDT] GDT loaded, TSS active (IST1=DF stack, IST2=NMI stack)");
    }
}

/// Установить kernel stack для ring 0 (TSS.rsp[0]).
/// Вызывается при переключении задач: чтобы при syscall/irq из ring 3
/// CPU нашёл валидный стек ядра.
pub fn set_kernel_stack(stack_top: u64) {
    unsafe {
        TSS.rsp[0] = stack_top;
    }
}
