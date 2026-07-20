// Task Scheduler — cooperative round-robin планировщик kernel threads.
//
// Фаза 2: кооперативный планировщик с yield points.
// Задачи добровольно уступают управление через yield().
// Когда появится timer-based preemption, будет добавлен context switch.

use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;
use core::sync::atomic::{AtomicU64, Ordering};

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
    pub time_slices: u64,
    pub wake_time: Option<u64>,
    pub entry: Option<fn()>,
}

/// Планировщик
pub struct Scheduler {
    tasks: Vec<Task>,
    current_task: Option<usize>,
    tick_count: u64,
    quantum: u64,
    next_id: u32,
    context_switches: u64,
}

static SCHEDULER: Mutex<Option<Scheduler>> = Mutex::new(None);

/// Инициализация планировщика
pub fn init() {
    let mut scheduler = Scheduler {
        tasks: Vec::new(),
        current_task: None,
        tick_count: 0,
        quantum: 10, // 10 тиков = 100мс между переключениями
        next_id: 0,
        context_switches: 0,
    };

    // Задача 0 = kshell (текущий поток)
    scheduler.tasks.push(Task {
        id: 0,
        name: String::from("kshell"),
        state: TaskState::Running,
        priority: Priority::High,
        time_slices: 0,
        wake_time: None,
        entry: None,
    });
    scheduler.current_task = Some(0);

    // Задача 1 = idle
    scheduler.tasks.push(Task {
        id: 1,
        name: String::from("idle"),
        state: TaskState::Ready,
        priority: Priority::Low,
        time_slices: 0,
        wake_time: None,
        entry: None,
    });

    scheduler.next_id = 2;

    *SCHEDULER.lock() = Some(scheduler);
    crate::println!("[SCHED] Task scheduler initialized (cooperative, quantum=10 ticks)");
}

/// Создать задачу с функцией入口
pub fn spawn(name: &str, entry: fn(), priority: Priority) -> Option<u32> {
    let mut sched_guard = SCHEDULER.lock();
    if let Some(ref mut sched) = *sched_guard {
        let id = sched.next_id;
        sched.next_id += 1;

        let task = Task {
            id,
            name: String::from(name),
            state: TaskState::Ready,
            priority,
            time_slices: 0,
            wake_time: None,
            entry: Some(entry),
        };

        sched.tasks.push(task);
        crate::println!("[SCHED] Task '{}' (id={}) spawned, priority={:?}", name, id, priority);
        Some(id)
    } else {
        None
    }
}

/// Завершить задачу
pub fn exit_task(id: u32) {
    let mut sched_guard = SCHEDULER.lock();
    if let Some(ref mut sched) = *sched_guard {
        for task in &mut sched.tasks {
            if task.id == id {
                task.state = TaskState::Zombie;
                crate::println!("[SCHED] Task '{}' (id={}) exited", task.name, id);
                break;
            }
        }
    }
}

/// Поставить задачу в сон на N тиков
pub fn sleep_task(id: u32, ticks: u64) {
    let mut sched_guard = SCHEDULER.lock();
    if let Some(ref mut sched) = *sched_guard {
        for task in &mut sched.tasks {
            if task.id == id {
                task.state = TaskState::Sleeping;
                task.wake_time = Some(sched.tick_count + ticks);
                break;
            }
        }
    }
}

/// Разбудить задачу
pub fn wake_task(id: u32) {
    let mut sched_guard = SCHEDULER.lock();
    if let Some(ref mut sched) = *sched_guard {
        for task in &mut sched.tasks {
            if task.id == id && task.state == TaskState::Sleeping {
                task.state = TaskState::Ready;
                task.wake_time = None;
                break;
            }
        }
    }
}

/// Yield — уступить управление другой задаче
pub fn yield_now() {
    let mut sched_guard = SCHEDULER.lock();
    if let Some(ref mut sched) = *sched_guard {
        let current = sched.current_task.unwrap_or(0);
        sched.tasks[current].state = TaskState::Ready;
        sched.tasks[current].time_slices = 0;

        // Выбираем следующую задачу
        let next = (current + 1) % sched.tasks.len();
        sched.tasks[next].state = TaskState::Running;
        sched.current_task = Some(next);
        sched.context_switches += 1;
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

/// Список задач
pub fn list_tasks() -> Vec<(u32, String, &'static str)> {
    let sched_guard = SCHEDULER.lock();
    let mut result = Vec::new();
    if let Some(ref sched) = *sched_guard {
        for task in &sched.tasks {
            result.push((task.id, task.name.clone(), task.state.as_str()));
        }
    }
    result
}

/// Обработчик тика таймера
pub fn timer_tick() {
    let mut sched_guard = SCHEDULER.lock();
    if let Some(ref mut sched) = *sched_guard {
        sched.tick_count += 1;

        // Проверяем sleeping задачи
        for task in &mut sched.tasks {
            if task.state == TaskState::Sleeping {
                if let Some(wake_time) = task.wake_time {
                    if sched.tick_count >= wake_time {
                        task.state = TaskState::Ready;
                        task.wake_time = None;
                    }
                }
            }
        }

        // Удаляем zombie задачи
        sched.tasks.retain(|t| t.state != TaskState::Zombie);

        // Round-robin: каждый quantum тиков переключаем
        if sched.tick_count % sched.quantum == 0 && sched.tasks.len() > 1 {
            let current = sched.current_task.unwrap_or(0);
            sched.tasks[current].state = TaskState::Ready;
            sched.tasks[current].time_slices += 1;

            let next = (current + 1) % sched.tasks.len();
            sched.tasks[next].state = TaskState::Running;
            sched.current_task = Some(next);
            sched.context_switches += 1;
        }
    }
}

/// Статистика
pub fn stats() -> (u64, u64, usize) {
    let sched_guard = SCHEDULER.lock();
    if let Some(ref sched) = *sched_guard {
        (sched.tick_count, sched.context_switches, sched.tasks.len())
    } else {
        (0, 0, 0)
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
