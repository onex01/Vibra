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

// Узел свободного списка. Overlay: живёт в первых байтах свободного блока.
//
// Размер и магия упакованы в одно слово, чтобы FreeNode оставался 16 байт:
// иначе MIN_BLOCK пришлось бы поднять до 32, и блоки с выравниванием 256
// нельзя было бы использовать (маленький prefix/suffix отбрасывался —
// `mount ... fat32` падал с OOM при 3% занятой кучи).
const SIZE_MAGIC_BITS: usize = 0x5649_0000_0000_0000; // "VI" в старших 16 битах
const SIZE_MASK: usize = 0x0000_FFFF_FFFF_FFFF;

// Минимальный размер блока: FreeNode должен помещаться в любой свободный блок.
// size_of::<FreeNode>() = 16 (usize + Option<NonNull>), выравнивание 16.
const MIN_BLOCK: usize = align_up(core::mem::size_of::<FreeNode>(), 16);
// Все размеры/смещения выравниваются под это.
const ALIGN_GRANULE: usize = 16;

struct FreeNode {
    /// Старшие 16 бит — магия, младшие 48 — полный размер блока.
    size_magic: usize,
    next: Option<NonNull<FreeNode>>,
}

impl FreeNode {
    unsafe fn size(ptr: NonNull<FreeNode>) -> usize {
        (*ptr.as_ptr()).size_magic & SIZE_MASK
    }

    unsafe fn set_size(ptr: NonNull<FreeNode>, size: usize) {
        (*ptr.as_ptr()).size_magic = SIZE_MAGIC_BITS | (size & SIZE_MASK);
    }

    unsafe fn next(ptr: NonNull<FreeNode>) -> Option<NonNull<FreeNode>> {
        (*ptr.as_ptr()).next
    }

    unsafe fn set_next(ptr: NonNull<FreeNode>, next: Option<NonNull<FreeNode>>) {
        (*ptr.as_ptr()).next = next;
    }
}

/// Сколько отдельных регионов PMM разрешено добавить в кучу.
///
/// Куча растёт по требованию: при нехватке места аллокатор берёт у PMM ещё
/// один непрерывный кусок и вставляет его в free-list. Без этого реальная
/// машина с большим framebuffer, USB-стеком и VIBRALOG упиралась в
/// фиксированные 4 МиБ и падала в `handle_alloc_error`.
const MAX_REGIONS: usize = 16;
/// Верхний предел роста кучи, чтобы не съесть всю RAM.
const MAX_HEAP_BYTES: usize = 64 * 1024 * 1024;

struct FreeList {
    head: Option<NonNull<FreeNode>>,
    /// (начало, конец) каждого региона кучи.
    regions: [(usize, usize); MAX_REGIONS],
    region_count: usize,
    /// Суммарный размер всех регионов.
    total_bytes: usize,
}

impl FreeList {
    const fn empty() -> Self {
        FreeList {
            head: None,
            regions: [(0, 0); MAX_REGIONS],
            region_count: 0,
            total_bytes: 0,
        }
    }

    // Инициализация первым свободным блоком на весь регион.
    unsafe fn init(&mut self, start: usize, size: usize) {
        *self = FreeList::empty();
        self.add_region(start, size);
    }

    /// Вставить новый регион PMM в free-list.
    ///
    /// Возвращает false, если регионов уже максимум или размер слишком мал.
    ///
    /// PMM почти всегда отдаёт следующий кусок вплотную к предыдущему, и
    /// free-list сливает их в один блок. Поэтому границы регионов тоже нужно
    /// сливать: иначе `node_ok` видел блок, выходящий за конец своего региона,
    /// объявлял его испорченным и обрезал список (куча «росла», но памяти
    /// не появлялось).
    unsafe fn add_region(&mut self, start: usize, size: usize) -> bool {
        if size < MIN_BLOCK || self.region_count >= MAX_REGIONS {
            return false;
        }
        if self.total_bytes.saturating_add(size) > MAX_HEAP_BYTES {
            return false;
        }
        let end = match start.checked_add(size) {
            Some(end) => end,
            None => return false,
        };

        // Слить новый интервал со всеми соприкасающимися регионами.
        let mut merged_start = start;
        let mut merged_end = end;
        let mut index = 0;
        while index < self.region_count {
            let (region_start, region_end) = self.regions[index];
            if region_end == merged_start || region_start == merged_end {
                merged_start = merged_start.min(region_start);
                merged_end = merged_end.max(region_end);
                for shift in index..self.region_count - 1 {
                    self.regions[shift] = self.regions[shift + 1];
                }
                self.region_count -= 1;
                index = 0;
                continue;
            }
            index += 1;
        }

        if self.region_count >= MAX_REGIONS {
            return false;
        }
        self.regions[self.region_count] = (merged_start, merged_end);
        self.region_count += 1;
        self.total_bytes = self.total_bytes.saturating_add(size);

        self.insert_block(start, size);
        true
    }

    /// Вставить свободный блок [start, start+size) с коалесценцией соседей.
    unsafe fn insert_block(&mut self, start: usize, size: usize) {
        let end = start + size;

        let mut prev: Option<NonNull<FreeNode>> = None;
        let mut cur = self.head;
        while let Some(node_ptr) = cur {
            if !self.node_ok(node_ptr) {
                self.truncate_before(prev);
                cur = None;
                break;
            }
            if node_ptr.as_ptr() as usize >= start {
                break;
            }
            prev = cur;
            cur = FreeNode::next(node_ptr);
        }

        // Слияние с правым соседом.
        let mut merge_right = false;
        let mut right_next = None;
        let mut merged_end = end;
        if let Some(node_ptr) = cur {
            if node_ptr.as_ptr() as usize == end {
                merge_right = true;
                right_next = FreeNode::next(node_ptr);
                merged_end = end + FreeNode::size(node_ptr);
            }
        }

        // Слияние с левым соседом.
        if let Some(prev_ptr) = prev {
            let p_addr = prev_ptr.as_ptr() as usize;
            let p_end = p_addr + FreeNode::size(prev_ptr);
            if p_end == start {
                FreeNode::set_size(prev_ptr, merged_end - p_addr);
                if merge_right {
                    FreeNode::set_next(prev_ptr, right_next);
                }
                return;
            }
        }

        let node = NonNull::new_unchecked(start as *mut FreeNode);
        FreeNode::set_size(node, merged_end - start);
        FreeNode::set_next(node, if merge_right { right_next } else { cur });
        if let Some(prev_ptr) = prev {
            FreeNode::set_next(prev_ptr, Some(node));
        } else {
            self.head = Some(node);
        }
    }

    /// Границы региона, содержащего `addr`.
    fn region_of(&self, addr: usize) -> Option<(usize, usize)> {
        for index in 0..self.region_count {
            let (start, end) = self.regions[index];
            if addr >= start && addr < end {
                return Some((start, end));
            }
        }
        None
    }

    // Полный размер кучи (для stats).
    fn total(&self) -> usize {
        self.total_bytes
    }

    // Общий диапазон кучи: (минимум начала, максимум конца).
    fn region(&self) -> (usize, usize) {
        let mut start = usize::MAX;
        let mut end = 0usize;
        for index in 0..self.region_count {
            let (s, e) = self.regions[index];
            start = start.min(s);
            end = end.max(e);
        }
        if start == usize::MAX {
            (0, 0)
        } else {
            (start, end)
        }
    }

    /// Проверить, что узел лежит в одном регионе и его заголовок не испорчен.
    fn node_ok(&self, ptr: NonNull<FreeNode>) -> bool {
        let addr = ptr.as_ptr() as usize;
        let Some((region_start, region_end)) = self.region_of(addr) else {
            return false;
        };
        if addr < region_start {
            return false;
        }
        if addr
            .checked_add(core::mem::size_of::<FreeNode>())
            .map_or(true, |end| end > region_end)
        {
            return false;
        }
        let word = unsafe { (*ptr.as_ptr()).size_magic };
        if word & !SIZE_MASK != SIZE_MAGIC_BITS {
            return false;
        }
        let size = word & SIZE_MASK;
        size >= MIN_BLOCK
            && addr.checked_add(size).map_or(false, |end| end <= region_end)
    }

    /// Обрезать список перед битым узлом. Возвращает false, если головной
    /// узел уже битый (куча недоступна).
    fn truncate_before(&mut self, prev: Option<NonNull<FreeNode>>) -> bool {
        match prev {
            Some(p) => {
                unsafe { FreeNode::set_next(p, None) };
                true
            }
            None => {
                self.head = None;
                false
            }
        }
    }

    // Сумма свободных байт — обход списка с проверкой заголовков.
    fn free_bytes(&self) -> usize {
        let mut sum = 0usize;
        let mut cur = self.head;
        while let Some(node_ptr) = cur {
            if !self.node_ok(node_ptr) {
                break;
            }
            unsafe {
                sum = sum.saturating_add(FreeNode::size(node_ptr));
                cur = FreeNode::next(node_ptr);
            }
        }
        sum
    }

    /// Диагностика для `diag heap-check`: (блоков, свободных байт, первый
    /// битый адрес).
    fn check(&self) -> (usize, usize, Option<usize>) {
        let mut blocks = 0usize;
        let mut bytes = 0usize;
        let mut cur = self.head;
        while let Some(node_ptr) = cur {
            if !self.node_ok(node_ptr) {
                return (blocks, bytes, Some(node_ptr.as_ptr() as usize));
            }
            unsafe {
                blocks += 1;
                bytes = bytes.saturating_add(FreeNode::size(node_ptr));
                cur = FreeNode::next(node_ptr);
            }
        }
        (blocks, bytes, None)
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
        while let Some(node_ptr) = cur {
            if !self.node_ok(node_ptr) {
                // Список испорчен. Не падаем с overflow, а отрезаем хвост:
                // ядро продолжит работу с оставшейся частью кучи, а
                // сообщение укажет адрес повреждения.
                crate::println!(
                    "[HEAP] CORRUPT free node {:#x} (alloc size {}); truncating list",
                    node_ptr.as_ptr() as usize,
                    size
                );
                self.truncate_before(prev);
                return core::ptr::null_mut();
            }

            let block_start = node_ptr.as_ptr() as usize;
            let block_size = FreeNode::size(node_ptr);
            let block_end = block_start + block_size; // границы проверены node_ok
            let alloc_start = align_up(block_start, align);
            let alloc_end = match alloc_start.checked_add(size) {
                Some(end) => end,
                None => return core::ptr::null_mut(),
            };

            if alloc_end <= block_end {
                let prefix = alloc_start - block_start;
                let suffix = block_end - alloc_end;
                let next = FreeNode::next(node_ptr);

                // Хвост уходит обратно в список, если в нём помещается узел.
                let right = if suffix >= MIN_BLOCK {
                    let new_node = NonNull::new_unchecked(alloc_end as *mut FreeNode);
                    FreeNode::set_size(new_node, suffix);
                    FreeNode::set_next(new_node, next);
                    Some(new_node)
                } else {
                    next
                };

                if prefix >= MIN_BLOCK {
                    // Левый отступ остаётся отдельным свободным блоком.
                    FreeNode::set_size(node_ptr, prefix);
                    FreeNode::set_next(node_ptr, right);
                } else if let Some(p) = prev {
                    FreeNode::set_next(p, right);
                } else {
                    self.head = right;
                }
                // prefix/suffix меньше MIN_BLOCK (0..15 байт) поглощаются
                // выделением: вернуть их в список нельзя — узел не поместится.

                return alloc_start as *mut u8;
            }

            prev = cur;
            cur = FreeNode::next(node_ptr);
        }
        core::ptr::null_mut()
    }

    // Освободить блок: вставка в address-order + коалесценция с соседями.
    unsafe fn deallocate(&mut self, ptr: *mut u8, layout: Layout) {
        let size = align_up(layout.size().max(1), ALIGN_GRANULE);
        let start = ptr as usize;
        let end = match start.checked_add(size) {
            Some(end) => end,
            None => return,
        };
        match self.region_of(start) {
            Some((region_start, region_end)) if start >= region_start && end <= region_end => {}
            _ => {
                crate::println!(
                    "[HEAP] dealloc outside heap: ptr={:#x} size={}",
                    start, size
                );
                return;
            }
        }

        // Список отсортирован по адресу. Найдём позицию вставки.
        let mut prev: Option<NonNull<FreeNode>> = None;
        let mut cur = self.head;
        while let Some(node_ptr) = cur {
            if !self.node_ok(node_ptr) {
                self.truncate_before(prev);
                cur = None;
                break;
            }
            let n_addr = node_ptr.as_ptr() as usize;
            if n_addr >= end {
                break;
            }
            prev = cur;
            cur = FreeNode::next(node_ptr);
        }

        // Коалесценция с правым соседом (cur), если он примыкает вплотную.
        let mut merge_right = false;
        let mut right_next = None;
        let merged_end = if let Some(node_ptr) = cur {
            let node_size = FreeNode::size(node_ptr);
            if node_ptr.as_ptr() as usize == end {
                merge_right = true;
                right_next = FreeNode::next(node_ptr);
                end.saturating_add(node_size)
            } else {
                end
            }
        } else {
            end
        };

        // Коалесценция с левым соседом (prev), если он примыкает вплотную.
        if let Some(prev_ptr) = prev {
            let p_addr = prev_ptr.as_ptr() as usize;
            let p_size = FreeNode::size(prev_ptr);
            if p_addr.saturating_add(p_size) == start {
                FreeNode::set_size(prev_ptr, merged_end - p_addr);
                if merge_right {
                    FreeNode::set_next(prev_ptr, right_next);
                }
                return;
            }
        }

        // Новый узел на [start, merged_end); правый сосед «впитан».
        let new_node = NonNull::new_unchecked(start as *mut FreeNode);
        FreeNode::set_size(new_node, merged_end - start);
        FreeNode::set_next(new_node, if merge_right { right_next } else { cur });
        if let Some(prev_ptr) = prev {
            FreeNode::set_next(prev_ptr, Some(new_node));
        } else {
            self.head = Some(new_node);
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

    // (start, end) виртуальных адресов региона кучи.
    pub fn region(&self) -> (usize, usize) {
        without_interrupts(|| self.0.lock().region())
    }

    /// Проверка целостности free-list: (блоков, свободных байт, битый адрес).
    pub fn check(&self) -> (usize, usize, Option<usize>) {
        without_interrupts(|| self.0.lock().check())
    }

    /// Добавить регион PMM в кучу. Возвращает false, если лимит исчерпан.
    unsafe fn add_region(&self, start: usize, size: usize) -> bool {
        without_interrupts(|| self.0.lock().add_region(start, size))
    }

    /// Сколько регионов уже подключено (для диагностики).
    pub fn region_count(&self) -> usize {
        without_interrupts(|| self.0.lock().region_count)
    }
}

unsafe impl GlobalAlloc for LockedHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // size == 0 -> dangling (по контракту GlobalAlloc).
        if layout.size() == 0 {
            return core::ptr::NonNull::dangling().as_ptr();
        }
        let ptr = without_interrupts(|| self.0.lock().allocate(layout));
        if !ptr.is_null() {
            return ptr;
        }

        // Не хватило места — растём у PMM, пока хватает лимита. Одна попытка
        // даёт 4 МиБ, поэтому крупный буфер может потребовать нескольких.
        for _ in 0..MAX_REGIONS {
            if !grow() {
                break;
            }
            let retry = without_interrupts(|| self.0.lock().allocate(layout));
            if !retry.is_null() {
                return retry;
            }
        }

        let (used, total) = self.stats();
        println!(
            "[HEAP] Out of memory! size={} align={} (used {}/{} bytes, {} regions)",
            layout.size(),
            layout.align(),
            used,
            total,
            self.region_count()
        );
        core::ptr::null_mut()
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

/// Начальный размер кучи: 8 МиБ. Дальше растёт по требованию.
const INITIAL_FRAMES: usize = 2048;
/// Порция роста: 4 МиБ за один раз.
const GROW_FRAMES: usize = 1024;

fn hhdm() -> u64 {
    crate::memory::paging::HHDM_OFFSET.load(core::sync::atomic::Ordering::Relaxed)
}

/// Добавить куче ещё один регион из PMM.
///
/// Вызывается из `alloc` при нехватке места. Не использует кучу (только PMM),
/// поэтому не может рекурсивно упасть в тот же OOM.
fn grow() -> bool {
    let (phys, frames) = match pmm::alloc_contiguous(GROW_FRAMES) {
        Some(p) => (p, GROW_FRAMES),
        None => match pmm::alloc_contiguous(GROW_FRAMES / 2) {
            Some(p) => (p, GROW_FRAMES / 2),
            None => return false,
        },
    };
    let size = frames * pmm::FRAME_SIZE;
    let start = hhdm() as usize + phys;

    let added = unsafe { ALLOCATOR.add_region(start, size) };
    if !added {
        // Лимит регионов/размера: физику надо вернуть, иначе она потеряется.
        pmm::free_contiguous(phys, frames);
        return false;
    }

    let (used, total) = ALLOCATOR.stats();
    println!(
        "[HEAP] grew by {} KB: {} KB total, {} regions (used {} KB)",
        size / 1024,
        total / 1024,
        ALLOCATOR.region_count(),
        used / 1024
    );
    true
}

// Инициализация кучи: 8 МБ из PMM через HHDM, с ростом по требованию.
pub unsafe fn init(hhdm_offset: u64) {
    let (phys, frames) = match pmm::alloc_contiguous(INITIAL_FRAMES) {
        Some(p) => (p, INITIAL_FRAMES),
        None => match pmm::alloc_contiguous(INITIAL_FRAMES / 2) {
            Some(p) => {
                println!("[HEAP] WARNING: requested 8 MB, got 4 MB");
                (p, INITIAL_FRAMES / 2)
            }
            None => match pmm::alloc_contiguous(INITIAL_FRAMES / 4) {
                Some(p) => {
                    println!("[HEAP] WARNING: requested 8 MB, got 2 MB");
                    (p, INITIAL_FRAMES / 4)
                }
                None => panic!("[HEAP] FATAL: cannot allocate heap region from PMM"),
            },
        },
    };
    let size = frames * pmm::FRAME_SIZE;
    let start = hhdm_offset as usize + phys;

    ALLOCATOR.init(start, size);
    println!(
        "[HEAP] Free-list allocator at phys={:#x} virt={:#x} ({} KB, max {} MB)",
        phys,
        start,
        size / 1024,
        MAX_HEAP_BYTES / (1024 * 1024)
    );
}

/// Сколько регионов подключено (для `diag heap-check`).
pub fn region_count() -> usize {
    ALLOCATOR.region_count()
}

/// Статистика кучи: (used, total) в байтах.
pub fn stats() -> (usize, usize) {
    ALLOCATOR.stats()
}

/// Проверка целостности кучи: (блоков, свободных байт, первый битый адрес).
pub fn check() -> (usize, usize, Option<usize>) {
    ALLOCATOR.check()
}

/// Регион кучи: (виртуальное начало, виртуальный конец).
/// VMM проверяет, что этот диапазон отображён в новом адресном пространстве.
pub fn region() -> (usize, usize) {
    ALLOCATOR.region()
}

/// Стресс-тест аллокатора: `iters` циклов alloc/write/check/drop.
///
/// Возвращает `(iters, ok, drift_bytes)`. На загрузке выполняется короткий
/// прогон, полный (10 000) — по команде `diag heap-test`: раньше он занимал
/// заметную долю boot-времени на каждой машине.
pub fn stress(iters: usize) -> (usize, bool, i64) {
    use alloc::{vec, vec::Vec};

    let (used_before, _) = stats();

    let mut patterns = [0u8; 256];
    let mut seed: u8 = 0x5A;
    for i in 0..256 {
        seed ^= (i as u8).wrapping_mul(0x9B);
        patterns[i] = seed;
    }

    for i in 0..iters {
        // Ctrl+C должен прерывать и полный heap-стресс.
        if i % 512 == 0 {
            crate::poll_input();
            if crate::is_cancelled() {
                crate::reset_cancel();
                println!("[HEAP] stress: cancelled at iter {}", i);
                return (i, true, 0);
            }
        }

        let n = 8 + (i % 248);
        let mut v: Vec<u8> = Vec::with_capacity(n);
        let mut idx = i;
        while v.len() < n {
            v.push(patterns[idx % 256]);
            idx = idx.wrapping_add(7);
        }
        let sample = v[n / 2];
        let expected = patterns[(i + (n / 2) * 7) % 256];
        if sample != expected {
            println!("[HEAP] stress: CORRUPTION at iter {} (got {:#x} want {:#x})", i, sample, expected);
            return (i, false, 0);
        }
        drop(v);

        if i % 2000 == 0 && i != 0 {
            let held_len = 8 + ((i / 2000) * 37) % 248;
            let _h = vec![0xCDu8; held_len];
            let _ = _h;
        }
    }

    let (used_after, _) = stats();
    let drift = used_after as i64 - used_before as i64;
    (iters, drift == 0, drift)
}
