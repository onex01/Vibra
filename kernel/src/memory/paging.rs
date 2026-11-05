// Базовый слой виртуальной памяти x86_64.
//
// На этом этапе Vibra ещё использует CR3 и таблицы, созданные Limine. Модуль
// умеет безопасно для ядра читать CR3 и обходить PML4/PDPT/PD/PT, что позволяет
// проверить реальные boot-маппинги до того, как мы начнём переключать CR3 на
// собственные таблицы. `sandbox_mapping_test` создаёт отдельную, неактивную
// PML4 для проверки map-логики; CR3 он не меняет.

use crate::memory::pmm;
use core::sync::atomic::{AtomicU64, Ordering};

pub(crate) const ENTRY_PRESENT: u64 = 1 << 0;
pub(crate) const ENTRY_WRITABLE: u64 = 1 << 1;
pub(crate) const ENTRY_USER: u64 = 1 << 2;
pub(crate) const ENTRY_PWT: u64 = 1 << 3;
pub(crate) const ENTRY_HUGE_PAGE: u64 = 1 << 7;
pub(crate) const ENTRY_NX: u64 = 1 << 63;
pub(crate) const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
const PAGE_4K_MASK: u64 = 0xfff;
const PAGE_2M_MASK: u64 = 0x1f_ffff;
const PAGE_1G_MASK: u64 = 0x3fff_ffff;
const PAGE_2M_ADDRESS_MASK: u64 = ADDRESS_MASK & !PAGE_2M_MASK;
const PAGE_1G_ADDRESS_MASK: u64 = ADDRESS_MASK & !PAGE_1G_MASK;

pub(crate) static HHDM_OFFSET: AtomicU64 = AtomicU64::new(0);

/// Поддерживает ли процессор NX/XD: `CPUID.80000001H:EDX[20]`.
///
/// Проверка обязательна перед записью `EFER.NXE`: по SDM `WRMSR` в
/// `IA32_EFER` с `NXE=1` на CPU без NX (или с выключенным в BIOS
/// «Execute Disable Bit») даёт `#GP(0)`. Именно так падало ядро на
/// ноутбуке сразу после `[VMM] Activating new address space...`.
pub fn cpu_has_nx() -> bool {
    // Extended leaf доступен только если максимальный extended leaf его включает.
    let max_ext = core::arch::x86_64::__cpuid(0x8000_0000).eax;
    if max_ext < 0x8000_0001 {
        return false;
    }
    let edx = core::arch::x86_64::__cpuid(0x8000_0001).edx;
    edx & (1 << 20) != 0
}

/// Фактически включённый `EFER.NXE` (бит 11).
///
/// Пока NX выключен, бит 63 (NX) в PTE/PDE — reserved bit: page walk даст
/// `#PF` с reserved-bit violation. Поэтому таблицы страниц обязаны узнавать
/// реальное состояние, а не полагаться на поддержку CPU.
pub fn nx_enabled() -> bool {
    const EFER_MSR: u32 = 0xC000_0080;
    let (lo, hi): (u32, u32);
    unsafe {
        core::arch::asm!(
            "rdmsr",
            in("ecx") EFER_MSR,
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack)
        );
    }
    (((hi as u64) << 32) | lo as u64) & (1 << 11) != 0
}

/// Биты защиты для PTE: `ENTRY_NX`, только если NX реально включён.
pub(crate) fn nx_flags(executable: bool) -> u64 {
    if executable || !nx_enabled() {
        0
    } else {
        ENTRY_NX
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageSize {
    Size4KiB,
    Size2MiB,
    Size1GiB,
}

impl PageSize {
    pub const fn bytes(self) -> usize {
        match self {
            PageSize::Size4KiB => 4 * 1024,
            PageSize::Size2MiB => 2 * 1024 * 1024,
            PageSize::Size1GiB => 1024 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Mapping {
    pub physical_address: u64,
    pub page_size: PageSize,
    pub flags: u64,
}

/// Результат временного теста построителя page tables.
#[derive(Clone, Copy, Debug)]
pub struct SandboxMapping {
    pub root_phys: u64,
    pub virtual_address: u64,
    pub physical_address: u64,
}

/// Инициализирует read-only walker текущего адресного пространства.
///
/// Безопасность: Limine по HHDM отображает всю физическую память, в том числе
/// активную PML4 и дочерние таблицы. Мы вызываем функцию единожды до включения
/// прерываний и только читаем эти отображения.
pub fn init(hhdm_offset: u64) {
    HHDM_OFFSET.store(hhdm_offset, Ordering::Relaxed);
    let root = current_root_phys();
    let entries = unsafe { present_entries(root) };
    crate::println!(
        "[PAGING] Limine CR3 root={:#x}; PML4 present entries={}",
        root,
        entries
    );
}

/// Физический адрес активной PML4; младшие 12 бит CR3 (PCID) отброшены.
/// Смещение HHDM (0, если `paging::init` ещё не вызывался).
pub fn hhdm_offset() -> u64 {
    HHDM_OFFSET.load(Ordering::Relaxed)
}

pub fn current_root_phys() -> u64 {
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack, preserves_flags));
    }
    cr3 & ADDRESS_MASK
}

/// Переводит виртуальный адрес через текущий CR3 в физический адрес.
/// Возвращает None, если адрес не отображён.
pub fn translate(virtual_address: u64) -> Option<Mapping> {
    unsafe { translate_from_root(current_root_phys(), virtual_address) }
}

/// Физический адрес для DMA через активные таблицы страниц.
///
/// Арифметика `virt - hhdm` верна только для HHDM-адресов (heap, PMM-фреймы).
/// Буфер может лежать в `.bss`, `.data` или на стеке — там разность даёт
/// фиктивный адрес, и устройство пишет в чужую физическую память. Именно так
/// терялось начало `/VIBRA.LOG`: DMA-источником был статический `CHUNK_BUF`.
pub fn virt_to_phys(virtual_address: u64) -> Option<u64> {
    translate(virtual_address).map(|mapping| mapping.physical_address)
}

/// Сколько раз `virt_to_phys` не смог перевести адрес (диагностика).
static TRANSLATE_FAILURES: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

pub fn translate_failures() -> u64 {
    TRANSLATE_FAILURES.load(Ordering::Relaxed)
}

/// DMA-адрес с явным отказом вместо тихой подстановки мусора.
///
/// Возвращает `None`, если адрес не отображён и не лежит в HHDM. Драйвер
/// обязан в этом случае вернуть ошибку, а не программировать PRD/PRP.
pub fn dma_address(virt: u64) -> Option<u64> {
    if let Some(phys) = virt_to_phys(virt) {
        return Some(phys);
    }
    // Fallback только для HHDM-диапазона: после смены CR3 таблицы Limine
    // могут не содержать промежуточных уровней для экзотических адресов.
    let hhdm = HHDM_OFFSET.load(Ordering::Relaxed);
    if hhdm != 0 && virt >= hhdm {
        return Some(virt - hhdm);
    }
    TRANSLATE_FAILURES.fetch_add(1, Ordering::Relaxed);
    None
}

/// Создаёт копию текущей PML4 и добавляет в свободный PML4-slot одно временное
/// 4-КиБ отображение. Новая PML4 никогда не становится активной: тестирует
/// построитель таблиц через `translate_from_root`, после чего освобождает все
/// пять фреймов (PML4, PDPT, PD, PT, test-page).
///
/// Это безопасный мост к следующему этапу: мы проверяем собственные page
/// tables без риска потерять доступ к ядру, HHDM или framebuffer при смене CR3.
pub fn sandbox_mapping_test() -> Result<SandboxMapping, &'static str> {
    let hhdm = HHDM_OFFSET.load(Ordering::Relaxed);
    if hhdm == 0 {
        return Err("paging is not initialized");
    }

    let root = allocate_zeroed_phys(hhdm).ok_or("cannot allocate sandbox PML4")?;
    let mut allocations = [root, 0, 0, 0, 0];

    // Замыкание удерживает ошибки внутри теста, чтобы общий cleanup ниже всегда
    // вернул уже выделенные фреймы PMM.
    let result = (|| unsafe {
        copy_table(root, current_root_phys());
        let slot = find_free_pml4_slot(root).ok_or("no free PML4 slot")?;

        allocations[1] = allocate_zeroed_phys(hhdm).ok_or("cannot allocate sandbox PDPT")?;
        allocations[2] = allocate_zeroed_phys(hhdm).ok_or("cannot allocate sandbox PD")?;
        allocations[3] = allocate_zeroed_phys(hhdm).ok_or("cannot allocate sandbox PT")?;
        allocations[4] = allocate_zeroed_phys(hhdm).ok_or("cannot allocate sandbox test page")?;

        let virtual_address = canonical_address(slot);
        write_entry(root, slot, allocations[1] | ENTRY_PRESENT | ENTRY_WRITABLE);
        write_entry(allocations[1], 0, allocations[2] | ENTRY_PRESENT | ENTRY_WRITABLE);
        write_entry(allocations[2], 0, allocations[3] | ENTRY_PRESENT | ENTRY_WRITABLE);
        write_entry(allocations[3], 0, allocations[4] | ENTRY_PRESENT | ENTRY_WRITABLE);

        match translate_from_root(root, virtual_address) {
            Some(mapping)
                if mapping.physical_address == allocations[4]
                    && mapping.page_size == PageSize::Size4KiB =>
            {
                Ok(SandboxMapping {
                    root_phys: root,
                    virtual_address,
                    physical_address: allocations[4],
                })
            }
            _ => Err("sandbox mapping translation mismatch"),
        }
    })();

    // Обратный порядок отражает владение таблицами. Они не были подключены к
    // CR3, поэтому после проверки их можно сразу вернуть PMM.
    for phys in allocations.into_iter().rev() {
        if phys != 0 {
            pmm::free_frame(phys as usize);
        }
    }
    result
}

/// Включён ли 5-level paging (CR4.LA57). Limine может включить его сам — тогда
/// корневая таблица это PML5, а не PML4, и обход начинается с битов 56:48.
pub fn la57_enabled() -> bool {
    let cr4: u64;
    unsafe {
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack, preserves_flags));
    }
    cr4 & (1 << 12) != 0
}

/// Та же операция для указанной корневой PML4. Нужна следующему этапу, где
/// будет построено новое адресное пространство, но CR3 ещё не переключён.
pub unsafe fn translate_from_root(root_phys: u64, virtual_address: u64) -> Option<Mapping> {
    // 5-level paging: корень — PML5, перед PML4 появляется ещё один уровень.
    let root_phys = if la57_enabled() {
        read_entry(root_phys, index(virtual_address, 48))? & ADDRESS_MASK
    } else {
        root_phys
    };

    let pml4e = read_entry(root_phys, index(virtual_address, 39))?;
    let pdpte = read_entry(pml4e & ADDRESS_MASK, index(virtual_address, 30))?;
    if pdpte & ENTRY_HUGE_PAGE != 0 {
        return Some(Mapping {
            physical_address: (pdpte & PAGE_1G_ADDRESS_MASK) | (virtual_address & PAGE_1G_MASK),
            page_size: PageSize::Size1GiB,
            flags: pdpte,
        });
    }

    let pde = read_entry(pdpte & ADDRESS_MASK, index(virtual_address, 21))?;
    if pde & ENTRY_HUGE_PAGE != 0 {
        return Some(Mapping {
            physical_address: (pde & PAGE_2M_ADDRESS_MASK) | (virtual_address & PAGE_2M_MASK),
            page_size: PageSize::Size2MiB,
            flags: pde,
        });
    }

    let pte = read_entry(pde & ADDRESS_MASK, index(virtual_address, 12))?;
    Some(Mapping {
        physical_address: (pte & ADDRESS_MASK) | (virtual_address & PAGE_4K_MASK),
        page_size: PageSize::Size4KiB,
        flags: pte,
    })
}

/// Проверяет, что перечисленные виртуальные адреса отображены в адресном
/// пространстве с корнем `root_phys` (CR3 при этом ещё не переключён).
///
/// Возвращает `(mapped, failed)`. Отчёт печатается построчно, чтобы на экране
/// ноутбука без serial было видно, какой именно регион теряется.
pub fn validate_space(root_phys: u64, regions: &[(&'static str, u64)]) -> (usize, usize) {
    crate::println!(
        "[PAGING] Validating {} regions against root {:#x} (LA57={})",
        regions.len(),
        root_phys,
        la57_enabled()
    );

    let mut mapped = 0usize;
    let mut failed = 0usize;

    for (name, virtual_address) in regions {
        match unsafe { translate_from_root(root_phys, *virtual_address) } {
            Some(mapping) => {
                mapped += 1;
                crate::println!(
                    "  [ OK ] {:<14} {:#018x} -> {:#012x} ({:?})",
                    name,
                    virtual_address,
                    mapping.physical_address,
                    mapping.page_size
                );
            }
            None => {
                failed += 1;
                crate::println!("  [FAIL] {:<14} {:#018x} NOT MAPPED", name, virtual_address);
            }
        }
    }

    (mapped, failed)
}

// ==========================================
// Write-combining для MMIO (фреймбуфер)
// ==========================================

/// Индекс записи PAT (биты 32..39), которую переводим в write-combining.
const PAT_WC_INDEX: u32 = 4;

/// MSR, задающий типы памяти для PAT-индексов.
const IA32_PAT: u32 = 0x277;

/// Бит PAT в 4-КиБ PTE.
const PTE_PAT: u64 = 1 << 7;
/// Page-level write-through (бит 3). Вместе с PCD/PAT образует индекс PAT.
const PTE_PWT: u64 = 1 << 3;
/// Page-level cache disable (бит 4).
const PTE_PCD: u64 = 1 << 4;
/// Бит PAT в 2-МиБ/1-ГиБ PDE/PDPTE (там он занимает место бита 12).
const HUGE_PAT: u64 = 1 << 12;

unsafe fn rdmsr(msr: u32) -> u64 {
    let (low, high): (u32, u32);
    core::arch::asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") low,
        out("edx") high,
        options(nostack, preserves_flags)
    );
    ((high as u64) << 32) | low as u64
}

unsafe fn wrmsr(msr: u32, value: u64) {
    core::arch::asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") value as u32,
        in("edx") (value >> 32) as u32,
        options(nostack, preserves_flags)
    );
}

/// Настроить PAT-запись `PAT_WC_INDEX` как write-combining (тип 0x01).
///
/// По умолчанию PAT[4] — это WB, поэтому одного бита PAT в PTE недостаточно:
/// нужно перепрограммировать MSR. Цикл настройки PAT поддерживают все CPU с
/// PAE/NX (то есть любая x86_64), но читаем и пишем аккуратно.
pub fn enable_pat_write_combining() -> bool {
    unsafe {
        let pat = rdmsr(IA32_PAT);
        let shift = PAT_WC_INDEX * 8;
        let current = (pat >> shift) & 0xFF;
        if current == 0x01 {
            return true;
        }
        let updated = (pat & !(0xFFu64 << shift)) | (0x01u64 << shift);
        wrmsr(IA32_PAT, updated);
        // Перечитать: если запись не принялась (или CPU игнорирует), лучше
        // знать об этом, чем молча получить UC вместо WC.
        ((rdmsr(IA32_PAT) >> shift) & 0xFF) == 0x01
    }
}

/// Пометить диапазон `[virt, virt + len)` как write-combining.
///
/// Возвращает число изменённых страниц (0 — не удалось). Затрагиваются только
/// уже существующие отображения: новые страницы здесь не создаются, потому
/// что функция вызывается для MMIO, который прошивка уже отобразила.
///
/// Работает и с 4-КиБ страницами, и с huge-page (2 МиБ/1 ГиБ): у них бит PAT
/// лежит в разных позициях, и перепутать их значит получить вместо WC
/// неизменяемое поведение или UC.
pub struct WcReport {
    pub changed: usize,
    pub already: usize,
    pub unmapped: usize,
    /// Флаги листа (PWT/PCD/PAT) до изменения — для диагностики.
    pub leaf_flags: u64,
}

pub fn set_write_combining(virt: u64, len: u64) -> WcReport {
    let mut report = WcReport { changed: 0, already: 0, unmapped: 0, leaf_flags: 0 };
    if len == 0 {
        return report;
    }
    // До `paging::init` HHDM неизвестен, и обход таблиц читал бы физические
    // адреса как виртуальные. Возвращаем пустой отчёт: вызывающий повторит
    // попытку, когда HHDM появится.
    if HHDM_OFFSET.load(Ordering::Relaxed) == 0 {
        return report;
    }
    let start = virt & !0xfff;
    let end = (virt + len + 0xfff) & !0xfff;
    let mut address = start;

    while address < end {
        unsafe {
            let Some(mapping) = translate(address) else {
                // Дыра в отображении: пропускаем страницу, а не выходим.
                report.unmapped += 1;
                address += 4096;
                continue;
            };
            let page_size = match mapping.page_size {
                PageSize::Size4KiB => 4096u64,
                PageSize::Size2MiB => 2 * 1024 * 1024,
                PageSize::Size1GiB => 1024 * 1024 * 1024,
            };

            // Находим запись, которую нужно изменить, и её бит PAT.
            let root = current_root_phys();
            let Some((table_phys, entry_index, pat_bit)) = find_entry_for(root, address) else {
                address += page_size;
                continue;
            };

            let existing = read_raw_entry(table_phys, entry_index);
            if report.leaf_flags == 0 {
                report.leaf_flags = existing;
            }
            if existing & pat_bit != 0 {
                // Запись уже помечена: второй раз не трогаем.
                report.already += 1;
                address = (address / page_size + 1) * page_size;
                continue;
            }

            let mut entry = existing;
            // PAT=1 при PCD=PWT=0 -> индекс 4 (настроен как WC выше).
            entry |= pat_bit;
            // PWT и PCD оба должны стать 0 — иначе индекс PAT будет 5/7,
            // а не 4, и вместо write-combining получится WT/UC.
            entry &= !PTE_PWT;
            entry &= !PTE_PCD;
            write_entry(table_phys, entry_index, entry);
            core::arch::asm!("invlpg [{}]", in(reg) address, options(nostack));
            report.changed += 1;

            address = (address / page_size + 1) * page_size;
        }
    }

    report
}

/// Найти таблицу, индекс и позицию бита PAT для конкретного адреса.
unsafe fn find_entry_for(root_phys: u64, address: u64) -> Option<(u64, usize, u64)> {
    let root_phys = if la57_enabled() {
        read_entry(root_phys, index(address, 48))? & ADDRESS_MASK
    } else {
        root_phys
    };

    let pml4e = read_entry(root_phys, index(address, 39))?;
    let pdpte = read_entry(pml4e & ADDRESS_MASK, index(address, 30))?;
    if pdpte & ENTRY_HUGE_PAGE != 0 {
        return Some((pml4e & ADDRESS_MASK, index(address, 30), HUGE_PAT));
    }

    let pde = read_entry(pdpte & ADDRESS_MASK, index(address, 21))?;
    if pde & ENTRY_HUGE_PAGE != 0 {
        return Some((pdpte & ADDRESS_MASK, index(address, 21), HUGE_PAT));
    }

    Some((pde & ADDRESS_MASK, index(address, 12), PTE_PAT))
}

#[inline]
const fn index(address: u64, shift: u8) -> usize {
    ((address >> shift) & 0x1ff) as usize
}

/// Возвращает entry таблицы только если она present.
///
/// `table_phys` взят из present entry предыдущего уровня либо из CR3; HHDM
/// делает этот физический адрес доступным ядру. Volatile-read не позволяет
/// компилятору кешировать содержимое таблицы страниц.
pub(crate) unsafe fn read_entry(table_phys: u64, entry_index: usize) -> Option<u64> {
    let hhdm = HHDM_OFFSET.load(Ordering::Relaxed);
    let table = (hhdm + table_phys) as *const u64;
    let entry = core::ptr::read_volatile(table.add(entry_index));
    if entry & ENTRY_PRESENT != 0 { Some(entry) } else { None }
}

pub(crate) unsafe fn write_entry(table_phys: u64, entry_index: usize, value: u64) {
    let hhdm = HHDM_OFFSET.load(Ordering::Relaxed);
    let table = (hhdm + table_phys) as *mut u64;
    core::ptr::write_volatile(table.add(entry_index), value);
}

unsafe fn copy_table(destination_phys: u64, source_phys: u64) {
    for entry in 0..512 {
        let value = read_raw_entry(source_phys, entry);
        write_entry(destination_phys, entry, value);
    }
}

unsafe fn find_free_pml4_slot(root_phys: u64) -> Option<usize> {
    for entry in 0..512 {
        if read_raw_entry(root_phys, entry) & ENTRY_PRESENT == 0 {
            return Some(entry);
        }
    }
    None
}

unsafe fn read_raw_entry(table_phys: u64, entry_index: usize) -> u64 {
    let hhdm = HHDM_OFFSET.load(Ordering::Relaxed);
    let table = (hhdm + table_phys) as *const u64;
    core::ptr::read_volatile(table.add(entry_index))
}

pub(crate) fn allocate_zeroed_phys(hhdm: u64) -> Option<u64> {
    pmm::alloc_frame_zeroed(hhdm).map(|virtual_address| virtual_address - hhdm)
}

const fn canonical_address(pml4_index: usize) -> u64 {
    let lower = (pml4_index as u64) << 39;
    if pml4_index & 0x100 != 0 {
        lower | 0xffff_0000_0000_0000
    } else {
        lower
    }
}

unsafe fn present_entries(root_phys: u64) -> usize {
    let mut count = 0;
    for entry in 0..512 {
        if read_entry(root_phys, entry).is_some() {
            count += 1;
        }
    }
    count
}
