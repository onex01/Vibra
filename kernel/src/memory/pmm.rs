// Physical Memory Manager (PMM).
//
// Битмап: 1 бит на 4 КБ-фрейм. 0 = свободен, 1 = занят.
// Инициализируется константой [0xFF; ...] (всё занято), init() размечает
// только MEMMAP_USABLE.
//
// ВАЖНО: BOOTLOADER_RECLAIMABLE намеренно НЕ помечается свободным —
// там живёт Limine-стек BSP, который нужен вплоть до Шага 4 (свои page
// tables + свой стек будут на этапе планировщика).
//
// Внешний API (init/alloc_frame/free_frame) не менялся; добавлены
// alloc_frame_zeroed, alloc_contiguous, stats — задел под Шаги 3-4.
//
// Все операции берут единый лок через interrupts::without_interrupts(),
// чтобы ISR никогда не отобрал лок у держателя (страховка от дедлоков
// под будущий вытесняющий планировщик).

use crate::println;
use crate::interrupts::without_interrupts;
use limine::memmap::{Entry, MEMMAP_USABLE};
use spin::Mutex;

pub const FRAME_SIZE: usize = 4096;
const MAX_FRAMES: usize = 16 * 1024 * 1024; // 64 ГБ (16M фреймов)
const BITMAP_SIZE: usize = MAX_FRAMES / 8;   // 2 МБ

// Состояние, защищённое локом.
struct PmmInner {
    free_frames: usize,
    total_frames: usize,
    // next-fit: начинать поиск свободного фрейма отсюда, а не с нуля.
    next_hint: usize,
}

impl PmmInner {
    const fn new() -> Self {
        PmmInner { free_frames: 0, total_frames: 0, next_hint: 0 }
    }
}

// Битмап живёт отдельным статическим массивом (4 ГБ / 8 = 128 КБ в .bss).
// Инициализируем «всё занято» — init() разметит USABLE.
static BITMAP: Mutex<[u8; BITMAP_SIZE]> = Mutex::new([0xFF; BITMAP_SIZE]);
static PMM: Mutex<PmmInner> = Mutex::new(PmmInner::new());

pub fn init(memory_map: &[&Entry]) {
    without_interrupts(|| {
        let mut bitmap = BITMAP.lock();
        let mut pmm = PMM.lock();

        // Сбрасываем в «всё занято» на случай повторного вызова.
        for b in bitmap.iter_mut() { *b = 0xFF; }
        pmm.free_frames = 0;
        pmm.total_frames = 0;

        for entry in memory_map {
            // Только USABLE. BOOTLOADER_RECLAIMABLE не трогаем (см. комментарий выше).
            if entry.type_ == MEMMAP_USABLE {
                let base = entry.base as usize;
                let length = entry.length as usize;

                let start_frame = base / FRAME_SIZE;
                let frame_count = length / FRAME_SIZE;

                for i in 0..frame_count {
                    let frame_index = start_frame + i;
                    if frame_index < MAX_FRAMES {
                        // set_frame_free безопасна только когда мы держим лок.
                        unsafe { set_frame_free(&mut bitmap, frame_index); }
                        pmm.free_frames += 1;
                        pmm.total_frames += 1;
                    }
                }
            }
        }

        let free_ram_mb = (pmm.free_frames * FRAME_SIZE) / (1024 * 1024);
        println!(
            "[MEM] PMM initialized. Free RAM: {} MB ({} frames free / {} total)",
            free_ram_mb, pmm.free_frames, pmm.total_frames
        );
    });
}

pub fn alloc_frame() -> Option<usize> {
    without_interrupts(|| {
        let mut bitmap = BITMAP.lock();
        let mut pmm = PMM.lock();

        if pmm.free_frames == 0 {
            return None;
        }

        // Next-fit: один полный оборот по битмапу начиная с next_hint.
        let start = pmm.next_hint;
        for offset in 0..MAX_FRAMES {
            let frame_index = (start + offset) % MAX_FRAMES;
            // Безопасно: bitmap под локом, frame_index в диапазоне.
            if unsafe { is_frame_free(&bitmap, frame_index) } {
                unsafe { set_frame_used(&mut bitmap, frame_index); }
                pmm.free_frames -= 1;
                pmm.next_hint = frame_index + 1;
                return Some(frame_index * FRAME_SIZE);
            }
        }
        None
    })
}

pub fn free_frame(addr: usize) {
    if addr % FRAME_SIZE != 0 {
        println!("[MEM] ERROR: Attempt to free unaligned address {:#x}!", addr);
        return;
    }

    let frame_index = addr / FRAME_SIZE;
    if frame_index >= MAX_FRAMES {
        return;
    }

    without_interrupts(|| {
        let mut bitmap = BITMAP.lock();
        let mut pmm = PMM.lock();

        // Безопасно: bitmap под локом, frame_index проверен выше.
        if unsafe { !is_frame_free(&bitmap, frame_index) } {
            unsafe { set_frame_free(&mut bitmap, frame_index); }
            pmm.free_frames += 1;
        } else {
            println!("[MEM] WARNING: Double free of frame {}", frame_index);
        }
    });
}

/// Освобождает диапазон, ранее полученный через `alloc_contiguous(count)`.
/// Операция атомарна относительно PMM: сначала проверяется весь диапазон и
/// только затем все фреймы возвращаются в bitmap. Поэтому ошибочный вызов не
/// оставит диапазон освобождённым наполовину.
pub fn free_contiguous(addr: usize, count: usize) -> bool {
    if count == 0 || addr % FRAME_SIZE != 0 {
        println!(
            "[MEM] ERROR: invalid contiguous free addr={:#x}, count={}",
            addr,
            count
        );
        return false;
    }

    let start = addr / FRAME_SIZE;
    let end = match start.checked_add(count) {
        Some(end) if end <= MAX_FRAMES => end,
        _ => {
            println!("[MEM] ERROR: contiguous free range is out of bounds");
            return false;
        }
    };

    without_interrupts(|| {
        let mut bitmap = BITMAP.lock();
        let mut pmm = PMM.lock();

        for frame in start..end {
            if unsafe { is_frame_free(&bitmap, frame) } {
                println!("[MEM] WARNING: contiguous free includes free frame {}", frame);
                return false;
            }
        }

        for frame in start..end {
            unsafe { set_frame_free(&mut bitmap, frame); }
        }
        pmm.free_frames += count;
        pmm.next_hint = pmm.next_hint.min(start);
        true
    })
}

// Выделяет один фрейм и возвращает его виртуальный адрес через HHDM,
// заполненный нулями. Предназначен для page tables Шага 4: им нужен
// гарантированно чистый 4 КБ-блок. Пока не вызывается (задел).
#[allow(dead_code)]
pub fn alloc_frame_zeroed(hhdm: u64) -> Option<u64> {
    let phys = alloc_frame()?;
    let virt = hhdm as usize + phys;
    unsafe {
        let ptr = virt as *mut u8;
        // 4096 байт = 64 * 64 байта; пишем u64 для скорости.
        let words = virt as *mut u64;
        for i in 0..(FRAME_SIZE / 8) {
            core::ptr::write_volatile(words.add(i), 0);
        }
        core::ptr::read_volatile(ptr); // барьер от реордеринга
    }
    Some(virt as u64)
}

// Выделяет count подряд идущих фреймов, возвращает физический адрес первого.
// Используется heap-регионом Шага 3 (4 МБ = 1024 фрейма).
pub fn alloc_contiguous(count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }

    without_interrupts(|| {
        let mut bitmap = BITMAP.lock();
        let mut pmm = PMM.lock();

        if pmm.free_frames < count {
            return None;
        }

        // Next-fit поиск: начиная с next_hint искать последовательность
        // из count свободных фреймов. Если дошли до конца, продолжить
        // с начала (круговой обход).
        let mut checked = 0usize;
        let mut start = pmm.next_hint;
        let max_check = MAX_FRAMES * 2; //avoid infinite loop

        while checked < max_check {
            // Найти начало следующей свободной последовательности
            while start < MAX_FRAMES {
                if unsafe { is_frame_free(&bitmap, start) } {
                    break;
                }
                start += 1;
            }
            if start >= MAX_FRAMES {
                start = 0;
                continue;
            }

            // Подсчитать длину свободной последовательности
            let mut end = start;
            while end < MAX_FRAMES && end - start < count {
                if unsafe { is_frame_free(&bitmap, end) } {
                    end += 1;
                } else {
                    break;
                }
            }

            let length = end - start;
            if length >= count {
                // Нашли достаточно! Выделить.
                for frame in start..(start + count) {
                    unsafe { set_frame_used(&mut bitmap, frame); }
                }
                pmm.free_frames -= count;
                pmm.next_hint = start + count;
                return Some(start * FRAME_SIZE);
            }

            // Пропустить эту последовательность и продолжить с неё
            start = end;
            checked += 1;
            if start >= MAX_FRAMES {
                start = 0;
            }
        }

        None
    })
}

// (used_frames, total_frames)
pub fn stats() -> (usize, usize) {
    without_interrupts(|| {
        let pmm = PMM.lock();
        (pmm.total_frames - pmm.free_frames, pmm.total_frames)
    })
}

// ============ Внутренние помощники (только под локом BITMAP) ============
//
// Принимают ссылку на битмап явно, чтобы связь с локом была видна в сигнатуре,
// а не через static mut (избегаем static_mut_refs lint).

#[inline]
unsafe fn set_frame_used(bitmap: &mut [u8; BITMAP_SIZE], index: usize) {
    bitmap[index / 8] |= 1 << (index % 8);
}

#[inline]
unsafe fn set_frame_free(bitmap: &mut [u8; BITMAP_SIZE], index: usize) {
    bitmap[index / 8] &= !(1 << (index % 8));
}

#[inline]
unsafe fn is_frame_free(bitmap: &[u8; BITMAP_SIZE], index: usize) -> bool {
    (bitmap[index / 8] & (1 << (index % 8))) == 0
}
