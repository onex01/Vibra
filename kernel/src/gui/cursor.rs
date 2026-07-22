// Курсор мыши — 12x12 стрелка с XOR-отрисовкой.
//
// XOR-подход гарантирует видимость курсора на любом фоне:
// при повторном XOR изображение исчезает (автоматическое стирание).
// Курсор рисуется поверх всех окон поверхностью.

use spin::Mutex;
use crate::framebuffer::Console;

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

/// Состояние курсора
struct CursorState {
    x: i32,
    y: i32,
    drawn: bool,
}

static STATE: Mutex<CursorState> = Mutex::new(CursorState {
    x: 100,
    y: 100,
    drawn: false,
});

/// Инициализация курсора (устанавливает начальную позицию)
pub fn init() {
    let mut state = STATE.lock();
    state.x = 100;
    state.y = 100;
    state.drawn = false;
    crate::println!("[GUI] Курсор инициализирован");
}

/// Применяет XOR-рисование для одного пикселя курсора
fn xor_cursor_pixel(console: &Console, sx: usize, sy: usize) {
    if sx < console.fb_width() && sy < console.fb_height() {
        let existing = console.read_pixel(sx, sy);
        console.put_pixel(sx, sy, existing ^ XOR_COLOR);
    }
}

/// Рисует/стирает курсор через XOR
fn apply_cursor(console: &Console) {
    let state = STATE.lock();
    let cx = state.x;
    let cy = state.y;
    drop(state);

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
                xor_cursor_pixel(console, sx, sy);
            }
        }
    }
}

/// Стереть курсор (вызывать перед перерисовкой фона)
pub fn undraw(console: &Console) {
    let mut state = STATE.lock();
    if state.drawn {
        state.drawn = false;
        drop(state);
        apply_cursor(console);
    }
}

/// Нарисовать курсор (вызывать после перерисовки фона)
pub fn draw(console: &Console) {
    let mut state = STATE.lock();
    state.drawn = true;
    drop(state);
    apply_cursor(console);
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
