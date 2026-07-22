// Heap-аллокатор ядра: собственный free-list с коалесценцией соседей.
//
// Бэкенд — PMM + HHDM: регион кучи берётся у физического менеджера памяти
// (pmm::alloc_contiguous) и отображается в виртуальный адрес через
// Higher Half Direct Map (virt = hhdm + phys). Limine уже маппит всю
// физику через HHDM, поэтому дополнительной настройки страниц не нужно
// (свои page tables — отдельный Шаг 4). Это убирает 1 МБ-статику из .bss,
// которая кормила старый BumpAllocator.
//
// Алгоритм: address-ordered singly-linked list свободных блоков. Узел
// FreeNode живёт ПРЯМО в свободном блоке (overlay), поэтому минимальный
// размер блока = size_of::<FreeNode>() с округлением до 16. При освобождении
// блок вставляется по адресу и коалесцируется с левым и правым соседом,
// если они примыкают вплотную — так список не фрагментируется.
//
// Форма API совместима с linked_list_allocator::Heap (init(start, size)),
// поэтому при необходимости крейт вставляется без правки вызывающего кода.

use crate::println;
use crate::interrupts::without_interrupts;
use crate::memory::pmm;
use core::alloc::{GlobalAlloc, Layout};
use core::ptr::NonNull;
use spin::Mutex;

// Минимальный размер блока: FreeNode должен помещаться в любой свободный блок.
// На x86_64 size_of::<FreeNode>() = 24 (usize size + usize-поле Option<NonNull>),
// округляем до 16-байтной границы — получаем 32.
const MIN_BLOCK: usize = align_up(core::mem::size_of::<FreeNode>(), 16);
// Все размеры/смещения выравниваются под это.
const ALIGN_GRANULE: usize = 16;

// Узел свободного списка. Overlay: живёт в первых байтах свободного блока.
// Поле size — полный размер блока (включая заголовок).
struct FreeNode {
    size: usize,
    next: Option<NonNull<FreeNode>>,
}

struct FreeList {
    head: Option<NonNull<FreeNode>>,
    start: usize,
    end: usize,
}

impl FreeList {
    const fn empty() -> Self {
        FreeList { head: None, start: 0, end: 0 }
    }

    // Инициализация одним свободным блоком на весь регион.
    unsafe fn init(&mut self, start: usize, size: usize) {
        self.start = start;
        self.end = start + size;
        if size >= MIN_BLOCK {
            let node = start as *mut FreeNode;
            core::ptr::addr_of_mut!((*node).size).write(size);
            core::ptr::addr_of_mut!((*node).next).write(None);
            self.head = NonNull::new(node);
        } else {
            self.head = None;
        }
    }

    // Полный размер региона (для stats).
    fn total(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    // Сумма свободных байт — обход списка.
    fn free_bytes(&self) -> usize {
        let mut sum = 0usize;
        let mut cur = self.head;
        while let Some(node_ptr) = cur {
            // Безопасно: узлы в нашем регионе, под локом.
            let node = unsafe { node_ptr.as_ref() };
            sum += node.size;
            cur = node.next;
        }
        sum
    }

    // Выделить блок под layout. Возвращает пользовательский указатель
    // (сразу за FreeNode/выравниванием) или None при нехватке места.
    unsafe fn allocate(&mut self, layout: Layout) -> *mut u8 {
        let size = align_up(layout.size().max(1), ALIGN_GRANULE);
        let align = layout.align().max(ALIGN_GRANULE);

        // Address-ordered first-fit: ищем первый блок, из которого можно
        // выделить выровненный регион размером `size`.
        let mut prev: Option<NonNull<FreeNode>> = None;
        let mut cur = self.head;
        while let Some(mut node_ptr) = cur {
            let node = node_ptr.as_mut();
            let block_start = node_ptr.as_ptr() as usize;
            let block_end = block_start + node.size;

            // Выровненный пользовательский адрес внутри блока. Левый
            // префикс и правый хвост обязаны либо стать FreeNode, либо
            // отсутствовать: иначе `dealloc(ptr, layout)` не сможет узнать
            // об этих байтах и они будут потеряны навсегда.
            let alloc_start = align_up(block_start, align);
            let alloc_end = alloc_start.checked_add(size);
            if let Some(ae) = alloc_end {
                if ae <= block_end {
                    let prefix = alloc_start - block_start;
                    let suffix = block_end - ae;
                    if (prefix != 0 && prefix < MIN_BLOCK)
                        || (suffix != 0 && suffix < MIN_BLOCK)
                    {
                        // Этот блок технически помещает запрос, но его
                        // неразмещаемые остатки нельзя корректно вернуть при
                        // освобождении. Ищем следующий подходящий блок.
                        prev = cur;
                        cur = node.next;
                        continue;
                    }

                    let next = node.next;
                    let right = if suffix >= MIN_BLOCK {
                        // Хвост уходит обратно в список как новый узел.
                        let new_node = ae as *mut FreeNode;
                        core::ptr::addr_of_mut!((*new_node).size).write(suffix);
                        core::ptr::addr_of_mut!((*new_node).next).write(next);
                        NonNull::new(new_node)
                    } else {
                        next
                    };

                    if prefix >= MIN_BLOCK {
                        // Сохраняем левый выровненный отступ в уже
                        // существующем узле — он остаётся перед выдаваемым
                        // пользователю адресом.
                        node.size = prefix;
                        node.next = right;
                    } else if let Some(p) = prev {
                        (*p.as_ptr()).next = right;
                    } else {
                        self.head = right;
                    }

                    return alloc_start as *mut u8;
                }
            }
            prev = cur;
            cur = node.next;
        }
        core::ptr::null_mut()
    }

    // Освободить блок: вставка в address-order + коалесценция с соседями.
    unsafe fn deallocate(&mut self, ptr: *mut u8, layout: Layout) {
        let size = align_up(layout.size().max(1), ALIGN_GRANULE);
        let start = ptr as usize;
        let end = start + size;

        // Список отсортирован по адресу. Найдём позицию вставки.
        let mut prev: Option<NonNull<FreeNode>> = None;
        let mut cur = self.head;
        while let Some(node_ptr) = cur {
            let n_addr = node_ptr.as_ptr() as usize;
            if n_addr >= end {
                break;
            }
            // Безопасно: узел в нашем регионе, под локом.
            let next = unsafe { node_ptr.as_ref().next };
            prev = cur;
            cur = next;
        }

        // Коалесценция с правым соседом (cur), если он примыкает вплотную.
        let merged_end = if let Some(node_ptr) = cur {
            let node = node_ptr.as_ref();
            let n_addr = node_ptr.as_ptr() as usize;
            if n_addr == end {
                // Сольёмся: расширим освобождаемый блок, выкинем соседа.
                end + node.size
            } else {
                // Не слился — правый сосед останется как есть.
                end
            }
        } else {
            end
        };

        // Коалесценция с левым соседом (prev), если он примыкает вплотную.
        if let Some(prev_ptr) = prev {
            let p = prev_ptr.as_ref();
            let p_addr = prev_ptr.as_ptr() as usize;
            let p_end = p_addr + p.size;
            if p_end == start {
                // Левый сосед поглощает освобождаемый блок (+правого, если был).
                core::ptr::addr_of_mut!((*prev_ptr.as_ptr()).size).write(merged_end - p_addr);
                // Если правый сосед слился — выкинуть его из списка.
                if let Some(node_ptr) = cur {
                    let node = node_ptr.as_ref();
                    let n_addr = node_ptr.as_ptr() as usize;
                    if n_addr == end {
                        (*prev_ptr.as_ptr()).next = node.next;
                    }
                }
                return;
            }
        }

        // Никакой коалесценции слева — создаём новый узел на [start, merged_end).
        // Если правый сосед слился, он «впитан» в новый узел и выбывает из списка.
        let new_node = start as *mut FreeNode;
        core::ptr::addr_of_mut!((*new_node).size).write(merged_end - start);
        if let Some(prev_ptr) = prev {
            // next нового узла = cur, но если cur слился — его next
            let next_after = if let Some(node_ptr) = cur {
                let node = node_ptr.as_ref();
                let n_addr = node_ptr.as_ptr() as usize;
                if n_addr == end { node.next } else { cur }
            } else {
                None
            };
            core::ptr::addr_of_mut!((*new_node).next).write(next_after);
            (*prev_ptr.as_ptr()).next = NonNull::new(new_node);
        } else {
            let next_after = if let Some(node_ptr) = cur {
                let node = node_ptr.as_ref();
                let n_addr = node_ptr.as_ptr() as usize;
                if n_addr == end { node.next } else { cur }
            } else {
                None
            };
            core::ptr::addr_of_mut!((*new_node).next).write(next_after);
            self.head = NonNull::new(new_node);
        }
    }
}

// Округление вверх до granule.
const fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

pub struct LockedHeap(Mutex<FreeList>);

impl LockedHeap {
    pub const fn empty() -> Self {
        LockedHeap(Mutex::new(FreeList::empty()))
    }

    // Инициализация уже созданного ALLOCATOR'а под локом.
    unsafe fn init(&self, start: usize, size: usize) {
        self.0.lock().init(start, size);
    }

    // (used, total) байт.
    pub fn stats(&self) -> (usize, usize) {
        without_interrupts(|| {
            let fl = self.0.lock();
            let total = fl.total();
            (total - fl.free_bytes(), total)
        })
    }
}

unsafe impl GlobalAlloc for LockedHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // size == 0 -> dangling (по контракту GlobalAlloc).
        if layout.size() == 0 {
            return core::ptr::NonNull::dangling().as_ptr();
        }
        let ptr = without_interrupts(|| self.0.lock().allocate(layout));
        if ptr.is_null() {
            let (used, total) = self.stats();
            println!(
                "[HEAP] Out of memory! size={} align={} (used {}/{} bytes)",
                layout.size(), layout.align(), used, total
            );
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if layout.size() == 0 || ptr == core::ptr::NonNull::dangling().as_ptr() {
            return;
        }
        without_interrupts(|| self.0.lock().deallocate(ptr, layout));
    }
}

// FreeList хранит NonNull<FreeNode> (не Send по умолчанию, т.к. сырой указатель).
// Доступ сериализуется через Mutex, и ядро однопоточное до появления
// планировщика — поэтому разделять структуру между «потоками» безопасно.
// Гарантия сохраняется: все обращения идут под heap-собственным локом
// внутри without_interrupts().
unsafe impl Send for LockedHeap {}
unsafe impl Sync for LockedHeap {}

// Регистрация глобального аллокатора.
#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

// Инициализация кучи: регион 4 МБ (1024 фрейма) из PMM, отображённый через HHDM.
pub unsafe fn init(hhdm: u64) {
    let (phys, frames) = match pmm::alloc_contiguous(1024) {
        Some(p) => (p, 1024usize),
        None => {
            // Запасной вариант: попросим меньше.
            match pmm::alloc_contiguous(512) {
                Some(p) => {
                    println!("[HEAP] WARNING: requested 4 MB, got 2 MB");
                    (p, 512usize)
                }
                None => panic!("[HEAP] FATAL: cannot allocate heap region from PMM"),
            }
        }
    };
    let size = frames * pmm::FRAME_SIZE;
    let start = hhdm as usize + phys;

    ALLOCATOR.init(start, size);
    println!(
        "[HEAP] Free-list allocator at phys={:#x} virt={:#x} ({} KB)",
        phys, start, size / 1024
    );
}

/// Статистика кучи: (used, total) в байтах.
pub fn stats() -> (usize, usize) {
    ALLOCATOR.stats()
}
