use core::alloc::{GlobalAlloc, Layout};
use core::ptr;
use spin::Mutex;

// Выделяем 1 МБ под кучу статически (безопасно до настройки виртуальной памяти)
static mut HEAP_MEMORY: [u8; 1024 * 1024] = [0; 1024 * 1024];

pub struct BumpAllocator {
    heap_start: usize,
    heap_end: usize,
    next: usize,
}

impl BumpAllocator {
    pub const fn new() -> Self {
        BumpAllocator {
            heap_start: 0,
            heap_end: 0,
            next: 0,
        }
    }

    pub unsafe fn init(&mut self, heap_start: usize, heap_size: usize) {
        self.heap_start = heap_start;
        self.heap_end = heap_start + heap_size;
        self.next = heap_start;
    }
}

// ✅ РЕШЕНИЕ: Локальная обёртка, чтобы обойти Orphan Rule
pub struct KernelAllocator(pub Mutex<BumpAllocator>);

unsafe impl GlobalAlloc for KernelAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut allocator = self.0.lock();
        
        let align = layout.align();
        let size = layout.size();
        let next_aligned = (allocator.next + align - 1) & !(align - 1);

        if next_aligned + size > allocator.heap_end {
            crate::println!("[HEAP] Out of memory! Requested: {} bytes", size);
            return ptr::null_mut();
        }

        allocator.next = next_aligned + size;
        next_aligned as *mut u8
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Bump allocator не поддерживает освобождение (free).
        // На следующем этапе апгрейда мы заменим его на Linked List Allocator.
    }
}

// Регистрируем наш локальный тип как глобальный аллокатор
#[global_allocator]
static ALLOCATOR: KernelAllocator = KernelAllocator(Mutex::new(BumpAllocator::new()));

/// Инициализация кучи
pub unsafe fn init() {
    let heap_start = core::ptr::addr_of_mut!(HEAP_MEMORY) as usize;
    let heap_size = HEAP_MEMORY.len();
    
    let mut allocator = ALLOCATOR.0.lock();
    allocator.init(heap_start, heap_size);
    
    crate::println!("[MEM] Heap initialized at 0x{:x} ({} bytes)", heap_start, heap_size);
}