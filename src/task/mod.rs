// Task Scheduler — вытесняющий планировщик (kernel threads).
//
// Фаза 2: round-robin планировщик с квантом 1 тик (10мс).
// Задачи: kshell (текущий поток), idle, demo задачи.
//
// Архитектура:
// - Task Control Block (TCB) хранит контекст регистров, стек, состояние
// - Scheduler хранит очередь задач и выбирает следующую по round-robin
// - Переключение через context switch (сохранение/восстановление регистров)
//
// Статус: заглушка. Полная реализация — когда VMM будет готов.

use alloc::string::String;
use spin::Mutex;

/// Состояние задачи
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TaskState {
    Ready,
    Running,
    Sleeping,
    Zombie,
    Blocked,
}

/// Приоритет задачи
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

/// Task Control Block (TCB)
pub struct Task {
    pub id: u32,
    pub name: String,
    pub state: TaskState,
    pub priority: Priority,
    pub stack_ptr: u64,
    pub stack_base: u64,
    pub stack_size: usize,
    pub time_slices: u64,
}

/// Планировщик
pub struct Scheduler {
    tasks: alloc::vec::Vec<Task>,
    current_task: Option<usize>,
    tick_count: u64,
    quantum: u64,  // квант в тиках
    next_id: u32,
}

static SCHEDULER: Mutex<Option<Scheduler>> = Mutex::new(None);

/// Инициализация планировщика
pub fn init() {
    let scheduler = Scheduler {
        tasks: alloc::vec::Vec::new(),
        current_task: None,
        tick_count: 0,
        quantum: 1, // 1 тик = 10мс
        next_id: 1,
    };
    *SCHEDULER.lock() = Some(scheduler);
    crate::println!("[SCHED] Task scheduler initialized (round-robin, quantum=1 tick)");
}

/// Создать задачу (пока заглушка — не запускает)
pub fn spawn(name: &str, _entry: fn(), priority: Priority) -> Option<u32> {
    let mut sched_guard = SCHEDULER.lock();
    if let Some(ref mut sched) = *sched_guard {
        let id = sched.next_id;
        sched.next_id += 1;

        let task = Task {
            id,
            name: String::from(name),
            state: TaskState::Ready,
            priority,
            stack_ptr: 0, // будет выделен при реальной реализации
            stack_base: 0,
            stack_size: 0,
            time_slices: 0,
        };

        sched.tasks.push(task);
        crate::println!("[SCHED] Task '{}' (id={}) created, priority={:?}", name, id, priority);
        Some(id)
    } else {
        None
    }
}

/// Количество задач
pub fn task_count() -> usize {
    let sched_guard = SCHEDULER.lock();
    if let Some(ref sched) = *sched_guard {
        sched.tasks.len()
    } else {
        0
    }
}

/// Получить информацию о задачах
pub fn list_tasks() -> alloc::vec::Vec<(u32, &'static str, TaskState)> {
    // Возвращаем статические данные (упрощённо)
    alloc::vec::Vec::new()
}

/// Обработчик тика таймера (вызывается из ISR таймера)
pub fn timer_tick() {
    if let Some(ref mut sched) = *SCHEDULER.lock() {
        sched.tick_count += 1;
        // TODO: context switch каждые quantum тиков
    }
}

/// Текущий ID задачи
pub fn current_task_id() -> Option<u32> {
    let sched_guard = SCHEDULER.lock();
    if let Some(ref sched) = *sched_guard {
        sched.current_task.and_then(|idx| sched.tasks.get(idx)).map(|t| t.id)
    } else {
        None
    }
}
