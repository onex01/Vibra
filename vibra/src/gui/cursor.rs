// Курсор мыши — 12x12 стрелка с XOR-отрисовкой.
//
// XOR-подход гарантирует видимость курсора на любом фоне:
// при повторном XOR изображение исчезает (автоматическое стирание).
// Курсор рисуется поверх всех окон поверхностью.
// Оптимизация: сохраняем фон при первом рисовании для быстрого стирания.

use spin::Mutex;
use vibra_kernel::framebuffer::Console;

const CURSOR_SIZE: usize = 12;

/// Bitmap курсора: 12x12 стрелка. 0 = прозрачный пиксель.
static CURSOR_BITMAP: [u32; CURSOR_SIZE * CURSOR_SIZE] = {
    const W: u32 = 0x00FFFFFF;
    const T: u32 = 0;
    [
        //  0  1  2  3  4  5  6  7  8  9 10 11
        W, T, T, T, T, T, T, T, T, T, T, T, // 0
        W, W, T, T, T, T, T, T, T, T, T, T, // 1
        W, W, W, T, T, T, T, T, T, T, T, T, // 2
        W, W, W, W, T, T, T, T, T, T, T, T, // 3
        W, W, W, W, W, T, T, T, T, T, T, T, // 4
        W, W, W, W, W, W, T, T, T, T, T, T, // 5
        W, W, W, W, W, W, W, T, T, T, T, T, // 6
        W, W, W, W, W, W, W, W, T, T, T, T, // 7
        W, W, T, T, T, W, W, W, W, T, T, T, // 8
        W, T, T, T, T, T, W, W, W, W, T, T, // 9
        T, T, T, T, T, T, T, W, W, W, T, T, // 10
        T, T, T, T, T, T, T, T, W, T, T, T, // 11
    ]
};

/// XOR-маска для видимости на любом фоне
const XOR_COLOR: u32 = 0x00FFFFFF;

/// Состояние курсора с сохранённым фоном
struct CursorState {
    x: i32,
    y: i32,
    drawn: bool,
    /// Сохранённые пиксели фона под курсором (для оптимизированного стирания)
    saved_bg: [u32; CURSOR_SIZE * CURSOR_SIZE],
    /// Позиция сохранённого фона
    saved_x: i32,
    saved_y: i32,
}

static STATE: Mutex<CursorState> = Mutex::new(CursorState {
    x: 100,
    y: 100,
    drawn: false,
    saved_bg: [0; CURSOR_SIZE * CURSOR_SIZE],
    saved_x: -1,
    saved_y: -1,
});

/// Инициализация курсора (устанавливает начальную позицию)
pub fn init() {
    let mut state = STATE.lock();
    state.x = 100;
    state.y = 100;
    state.drawn = false;
    state.saved_x = -1;
    state.saved_y = -1;
    vibra_kernel::println!("[GUI] Курсор инициализирован");
}

/// Сохраняет фон под курсором для быстрого восстановления
fn save_background(console: &Console, state: &mut CursorState) {
    let cx = state.x;
    let cy = state.y;
    for dy in 0..CURSOR_SIZE {
        let screen_y = cy + dy as i32;
        if screen_y < 0 || screen_y >= console.fb_height() as i32 {
            continue;
        }
        let sy = screen_y as usize;
        for dx in 0..CURSOR_SIZE {
            let screen_x = cx + dx as i32;
            if screen_x < 0 || screen_x >= console.fb_width() as i32 {
                continue;
            }
            let sx = screen_x as usize;
            state.saved_bg[dy * CURSOR_SIZE + dx] = console.read_pixel(sx, sy);
        }
    }
    state.saved_x = cx;
    state.saved_y = cy;
}

/// Восстанавливает сохранённый фон
fn restore_background(console: &Console, state: &CursorState) {
    let cx = state.saved_x;
    let cy = state.saved_y;
    if cx < 0 || cy < 0 {
        return;
    }
    for dy in 0..CURSOR_SIZE {
        let screen_y = cy + dy as i32;
        if screen_y < 0 || screen_y >= console.fb_height() as i32 {
            continue;
        }
        let sy = screen_y as usize;
        for dx in 0..CURSOR_SIZE {
            let screen_x = cx + dx as i32;
            if screen_x < 0 || screen_x >= console.fb_width() as i32 {
                continue;
            }
            let sx = screen_x as usize;
            let bitmap_idx = dy * CURSOR_SIZE + dx;
            if CURSOR_BITMAP[bitmap_idx] != 0 {
                console.put_pixel(sx, sy, state.saved_bg[bitmap_idx]);
            }
        }
    }
}

/// Рисует курсор через XOR с сохранённым фоном
fn draw_cursor_pixels(console: &Console, state: &CursorState) {
    let cx = state.x;
    let cy = state.y;
    for dy in 0..CURSOR_SIZE {
        let screen_y = cy + dy as i32;
        if screen_y < 0 {
            continue;
        }
        let sy = screen_y as usize;
        if sy >= console.fb_height() {
            break;
        }
        for dx in 0..CURSOR_SIZE {
            let screen_x = cx + dx as i32;
            if screen_x < 0 {
                continue;
            }
            let sx = screen_x as usize;
            if sx >= console.fb_width() {
                break;
            }
            let bitmap_idx = dy * CURSOR_SIZE + dx;
            if CURSOR_BITMAP[bitmap_idx] != 0 {
                let existing = console.read_pixel(sx, sy);
                console.put_pixel(sx, sy, existing ^ XOR_COLOR);
            }
        }
    }
}

/// Стереть курсор (вызывать перед перерисовкой фона)
pub fn undraw(console: &Console) {
    let mut state = STATE.lock();
    if state.drawn {
        state.drawn = false;
        // Восстанавливаем сохранённый фон вместо XOR-стирания
        restore_background(console, &state);
    }
}

/// Нарисовать курсор (вызывать после перерисовки фона)
pub fn draw(console: &Console) {
    let mut state = STATE.lock();
    // Сохраняем фон перед рисованием
    save_background(console, &mut state);
    state.drawn = true;
    draw_cursor_pixels(console, &state);
}

/// Переместить курсор в новую позицию
pub fn move_to(x: i32, y: i32) {
    let mut state = STATE.lock();
    state.x = x;
    state.y = y;
}

/// Получить текущую позицию курсора
pub fn get_position() -> (i32, i32) {
    let state = STATE.lock();
    (state.x, state.y)
}
