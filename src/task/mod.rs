// Preemptive Task Scheduler — round-robin с контекстным переключением.
//
// Каждая задача имеет kernel stack (8KB, heap-allocated) + TCB.
// Timer (vector 32) → naked stub → tick_and_switch(ctx) -> new_rsp.
// Yield → INT 0x81 → softirq_naked_stub → softirq_handler(ctx) -> new_rsp.
// Формат контекста — единый: 15 GP + iretq frame (5 слов) = 160 байт.

pub mod ctx_switch;
pub mod user;

use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;
use core::sync::atomic::{AtomicBool, Ordering};
use core::alloc::Layout;

const KERNEL_STACK_SIZE: usize = 8 * 1024; // 8 KiB

/// Состояние задачи
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TaskState {
    Ready,
    Running,
    Sleeping,
    Zombie,
    Blocked,
}

impl TaskState {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskState::Ready => "Ready",
            TaskState::Running => "Running",
            TaskState::Sleeping => "Sleep",
            TaskState::Zombie => "Zombie",
            TaskState::Blocked => "Block",
        }
    }
}

/// Приоритет
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

/// Выделить kernel stack через alloc_zeroed (без large temp на стеке).
/// Возвращает (ptr, top_addr).
fn alloc_kstack() -> Option<(*mut u8, u64)> {
    let layout = Layout::from_size_align(KERNEL_STACK_SIZE, 16).ok()?;
    unsafe {
        let ptr = alloc::alloc::alloc_zeroed(layout);
        if ptr.is_null() {
            None
        } else {
            let top = ptr as u64 + KERNEL_STACK_SIZE as u64;
            Some((ptr, top))
        }
    }
}

/// Task Control Block
pub struct Task {
    pub id: u32,
    pub name: String,
    pub state: TaskState,
    pub priority: Priority,
    pub time_slices: u64,
    pub wake_time: Option<u64>,
    pub entry: Option<fn()>,
    /// Указатель на сохранённый контекст (rsp)
    saved_rsp: u64,
    /// Kernel stack pointer + layout для dealloc
    kstack_ptr: *mut u8,
    kstack_layout: Layout,
    /// Верхушка kernel stack (для TSS.rsp0 при ring3→ring0 переходе)
    kstack_top: Option<u64>,
}

// SAFETY: kernel stack pointer used only by scheduler, single-threaded kernel
unsafe impl Send for Task {}
unsafe impl Sync for Task {}

impl Drop for Task {
    fn drop(&mut self) {
        if !self.kstack_ptr.is_null() {
            unsafe { alloc::alloc::dealloc(self.kstack_ptr, self.kstack_layout); }
        }
    }
}

/// Планировщик
pub struct Scheduler {
    tasks: Vec<Task>,
    current: Option<usize>,
    tick_count: u64,
    quantum: u64,
    next_id: u32,
    switches: u64,
}

static SCHEDULER: Mutex<Option<Scheduler>> = Mutex::new(None);
static SCHED_READY: AtomicBool = AtomicBool::new(false);

/// ===== Вызывается из naked stub =====

/// Обработчик тика: вызывается из timer_naked_stub.
/// Возвращает old_rsp (если контекст не меняется) или new_rsp.
#[no_mangle]
pub extern "sysv64" fn tick_and_switch(ctx_ptr: u64) -> u64 {
    let mut guard = match SCHEDULER.try_lock() {
        Some(g) => g,
        None => return ctx_ptr,
    };
    let sched = match *guard {
        Some(ref mut s) => s,
        None => return ctx_ptr,
    };

    sched.tick_count += 1;
    crate::interrupts::idt::TICKS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);

    // EOI: PIC или LAPIC — зависит от того, кто управляет IRQ0
    // Если APIC полностью активен → LAPIC EOI, иначе → PIC EOI
    if crate::interrupts::apic::is_active() {
        crate::interrupts::apic::eoi();
    } else {
        unsafe { crate::interrupts::pic::eoi(0); }
    }

    // Пробуждаем спящие
    let tc = sched.tasks.len();
    for i in 0..tc {
        if sched.tasks[i].state == TaskState::Sleeping {
            if let Some(wt) = sched.tasks[i].wake_time {
                if sched.tick_count >= wt {
                    sched.tasks[i].state = TaskState::Ready;
                    sched.tasks[i].wake_time = None;
                }
            }
        }
    }

    // Квант
    if sched.tick_count % sched.quantum != 0 || tc <= 1 {
        return ctx_ptr;
    }

    let cur = match sched.current {
        Some(i) => i,
        None => return ctx_ptr,
    };

    sched.tasks[cur].saved_rsp = ctx_ptr;
    sched.tasks[cur].state = TaskState::Ready;
    sched.tasks[cur].time_slices += 1;

    // Round-robin
    let mut next = (cur + 1) % tc;
    let start = next;
    loop {
        if sched.tasks[next].state == TaskState::Ready {
            break;
        }
        next = (next + 1) % tc;
        if next == start {
            sched.tasks[cur].state = TaskState::Running;
            sched.current = Some(cur);
            return ctx_ptr;
        }
    }

    sched.tasks[next].state = TaskState::Running;
    sched.current = Some(next);
    sched.switches += 1;

    // Обновляем TSS.rsp0 — стек ядра для ring 3 задач
    if let Some(kstack_top) = sched.tasks[next].kstack_top {
        crate::gdt::set_kernel_stack(kstack_top);
        crate::syscall::update_kernel_stack(kstack_top);
    }

    // Сохраняем user RSP в PerCpu для syscall_entry
    // user RSP в iretq frame: saved_rsp + 144 (offset от r15)
    let next_rsp = sched.tasks[next].saved_rsp;
    let user_rsp = unsafe { core::ptr::read_volatile((next_rsp + 144) as *const u64) };
    crate::syscall::save_user_rsp(user_rsp);

    sched.tasks[next].saved_rsp
}

/// Soft IRQ handler (yield)
#[no_mangle]
pub extern "sysv64" fn softirq_handler(ctx_ptr: u64) -> u64 {
    let mut guard = match SCHEDULER.try_lock() {
        Some(g) => g,
        None => return ctx_ptr,
    };
    let sched = match *guard {
        Some(ref mut s) => s,
        None => return ctx_ptr,
    };

    let cur = match sched.current {
        Some(i) => i,
        None => return ctx_ptr,
    };
    let tc = sched.tasks.len();
    if tc <= 1 {
        return ctx_ptr;
    }

    sched.tasks[cur].saved_rsp = ctx_ptr;
    sched.tasks[cur].state = TaskState::Ready;

    let mut next = (cur + 1) % tc;
    let start = next;
    loop {
        if sched.tasks[next].state == TaskState::Ready {
            break;
        }
        next = (next + 1) % tc;
        if next == start {
            sched.tasks[cur].state = TaskState::Running;
            return ctx_ptr;
        }
    }

    sched.tasks[next].state = TaskState::Running;
    sched.current = Some(next);
    sched.switches += 1;

    if let Some(kstack_top) = sched.tasks[next].kstack_top {
        crate::gdt::set_kernel_stack(kstack_top);
    }

    sched.tasks[next].saved_rsp
}

/// ===== API =====

pub fn init() {
    let layout = match Layout::from_size_align(KERNEL_STACK_SIZE, 16) {
        Ok(l) => l,
        Err(_) => { crate::println!("[SCHED] FATAL: bad layout"); return; }
    };

    // kstack0 не используется для kshell (текущий поток на Limine стеке),
    // но TCB требует поле — выделяем最小 стек, он не будет переключаться.
    let (ptr0, top0) = match alloc_kstack() {
        Some(v) => v,
        None => { crate::println!("[SCHED] FATAL: no memory for kstack0"); return; }
    };

    let mut scheduler = Scheduler {
        tasks: Vec::new(),
        current: None,
        tick_count: 0,
        quantum: 4,
        next_id: 0,
        switches: 0,
    };

    // Задача 0 = kshell (текущий поток, Limine stack)
    scheduler.tasks.push(Task {
        id: 0,
        name: String::from("kshell"),
        state: TaskState::Running,
        priority: Priority::High,
        time_slices: 0,
        wake_time: None,
        entry: None,
        saved_rsp: 0,
        kstack_ptr: ptr0,
        kstack_layout: layout,
        kstack_top: Some(top0),
    });
    scheduler.current = Some(0);

    // Задача 1 = idle
    let (ptr1, top1) = match alloc_kstack() {
        Some(v) => v,
        None => { crate::println!("[SCHED] FATAL: no memory for kstack1"); return; }
    };
    let rsp1 = unsafe { ctx_switch::prepare_task_stack(top1, idle_task_entry as u64) };

    scheduler.tasks.push(Task {
        id: 1,
        name: String::from("idle"),
        state: TaskState::Ready,
        priority: Priority::Low,
        time_slices: 0,
        wake_time: None,
        entry: Some(idle_task_entry as fn()),
        saved_rsp: rsp1,
        kstack_ptr: ptr1,
        kstack_layout: layout,
        kstack_top: Some(top1),
    });

    scheduler.next_id = 2;
    *SCHEDULER.lock() = Some(scheduler);
    SCHED_READY.store(true, Ordering::SeqCst);

    crate::println!("[SCHED] Preemptive scheduler ready (quantum=4, stack={}KB)", KERNEL_STACK_SIZE / 1024);
}

/// Создать задачу
pub fn spawn(name: &str, entry: fn(), priority: Priority) -> Option<u32> {
    let layout = Layout::from_size_align(KERNEL_STACK_SIZE, 16).ok()?;

    let mut guard = SCHEDULER.lock();
    let sched = match *guard {
        Some(ref mut s) => s,
        None => return None,
    };

    let (ptr, top) = alloc_kstack()?;
    let rsp = unsafe { ctx_switch::prepare_task_stack(top, entry as u64) };
    let id = sched.next_id;
    sched.next_id += 1;

    sched.tasks.push(Task {
        id,
        name: String::from(name),
        state: TaskState::Ready,
        priority,
        time_slices: 0,
        wake_time: None,
        entry: Some(entry),
        saved_rsp: rsp,
        kstack_ptr: ptr,
        kstack_layout: layout,
        kstack_top: Some(top),
    });

    crate::println!("[SCHED] Task '{}' (id={}) spawned", name, id);
    Some(id)
}

pub fn exit_task(id: u32) {
    let mut guard = SCHEDULER.lock();
    if let Some(ref mut sched) = *guard {
        for task in &mut sched.tasks {
            if task.id == id {
                task.state = TaskState::Zombie;
                crate::println!("[SCHED] Task '{}' (id={}) exited", task.name, id);
                break;
            }
        }
    }
}

pub fn sleep_task(id: u32, ticks: u64) {
    let mut guard = SCHEDULER.lock();
    if let Some(ref mut sched) = *guard {
        for task in &mut sched.tasks {
            if task.id == id {
                task.state = TaskState::Sleeping;
                task.wake_time = Some(sched.tick_count + ticks);
                break;
            }
        }
    }
}

pub fn wake_task(id: u32) {
    let mut guard = SCHEDULER.lock();
    if let Some(ref mut sched) = *guard {
        for task in &mut sched.tasks {
            if task.id == id && task.state == TaskState::Sleeping {
                task.state = TaskState::Ready;
                task.wake_time = None;
                break;
            }
        }
    }
}

pub fn yield_now() {
    unsafe { core::arch::asm!("int 0x81", options(nostack)); }
}

pub fn is_ready() -> bool {
    SCHED_READY.load(Ordering::SeqCst)
}

pub fn task_count() -> usize {
    let guard = SCHEDULER.lock();
    match *guard {
        Some(ref s) => s.tasks.len(),
        None => 0,
    }
}

pub fn list_tasks() -> Vec<(u32, String, &'static str)> {
    let guard = SCHEDULER.lock();
    let mut result = Vec::new();
    if let Some(ref sched) = *guard {
        for task in &sched.tasks {
            result.push((task.id, task.name.clone(), task.state.as_str()));
        }
    }
    result
}

pub fn stats() -> (u64, u64, usize) {
    let guard = SCHEDULER.lock();
    match *guard {
        Some(ref s) => (s.tick_count, s.switches, s.tasks.len()),
        None => (0, 0, 0),
    }
}

pub fn current_task_id() -> Option<u32> {
    let guard = SCHEDULER.lock();
    match *guard {
        Some(ref sched) => sched.current.and_then(|i| sched.tasks.get(i)).map(|t| t.id),
        None => None,
    }
}

fn idle_task_entry() {
    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }
}

/// Получить верхушку kernel stack текущей задачи (для TSS.rsp0 / syscall)
pub fn get_kstack_top() -> Option<u64> {
    let guard = SCHEDULER.lock();
    match *guard {
        Some(ref sched) => sched.current
            .and_then(|i| sched.tasks.get(i))
            .and_then(|t| t.kstack_top),
        None => None,
    }
}
