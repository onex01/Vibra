// Базовый слой виртуальной памяти x86_64.
//
// На этом этапе Vibra ещё использует CR3 и таблицы, созданные Limine. Модуль
// умеет безопасно для ядра читать CR3 и обходить PML4/PDPT/PD/PT, что позволяет
// проверить реальные boot-маппинги до того, как мы начнём переключать CR3 на
// собственные таблицы. `sandbox_mapping_test` создаёт отдельную, неактивную
// PML4 для проверки map-логики; CR3 он не меняет.

use crate::memory::pmm;
use core::sync::atomic::{AtomicU64, Ordering};

const ENTRY_PRESENT: u64 = 1 << 0;
const ENTRY_WRITABLE: u64 = 1 << 1;
const ENTRY_HUGE_PAGE: u64 = 1 << 7;
const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
const PAGE_4K_MASK: u64 = 0xfff;
const PAGE_2M_MASK: u64 = 0x1f_ffff;
const PAGE_1G_MASK: u64 = 0x3fff_ffff;
const PAGE_2M_ADDRESS_MASK: u64 = ADDRESS_MASK & !PAGE_2M_MASK;
const PAGE_1G_ADDRESS_MASK: u64 = ADDRESS_MASK & !PAGE_1G_MASK;

static HHDM_OFFSET: AtomicU64 = AtomicU64::new(0);

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

/// Та же операция для указанной корневой PML4. Нужна следующему этапу, где
/// будет построено новое адресное пространство, но CR3 ещё не переключён.
pub unsafe fn translate_from_root(root_phys: u64, virtual_address: u64) -> Option<Mapping> {
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

#[inline]
const fn index(address: u64, shift: u8) -> usize {
    ((address >> shift) & 0x1ff) as usize
}

/// Возвращает entry таблицы только если она present.
///
/// `table_phys` взят из present entry предыдущего уровня либо из CR3; HHDM
/// делает этот физический адрес доступным ядру. Volatile-read не позволяет
/// компилятору кешировать содержимое таблицы страниц.
unsafe fn read_entry(table_phys: u64, entry_index: usize) -> Option<u64> {
    let hhdm = HHDM_OFFSET.load(Ordering::Relaxed);
    let table = (hhdm + table_phys) as *const u64;
    let entry = core::ptr::read_volatile(table.add(entry_index));
    if entry & ENTRY_PRESENT != 0 { Some(entry) } else { None }
}

unsafe fn write_entry(table_phys: u64, entry_index: usize, value: u64) {
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

fn allocate_zeroed_phys(hhdm: u64) -> Option<u64> {
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
