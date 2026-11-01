use super::CmdResult;
use crate::framebuffer::{Console, COLOR_RED, COLOR_YELLOW};
use crate::fs;
use alloc::format;
use alloc::vec::Vec;

/// Размер порции чтения. Фиксирован и не зависит от размера файла.
const CHUNK: usize = 16 * 1024;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    if args.is_empty() || args.iter().any(|a| *a == "--help" || *a == "-h") {
        console.print_colored("Usage: cat <file> [file...]\n", COLOR_YELLOW);
        return CmdResult::Ok;
    }

    for name in args {
        if let Err(error) = print_file(console, name) {
            console.print_colored("cat: ", COLOR_RED);
            console.print(name);
            console.print(": ");
            console.print(&format!("{}", error));
            console.put_char('\n');
        }
    }

    CmdResult::Ok
}

/// Печатает файл потоково: читает по 16 КиБ и не держит его в памяти целиком.
///
/// Раньше `cat` вызывал `fs::read_file`, который выделял буфер размером с файл.
/// На реальной машине `cat` большого лога съедал всю кучу и ядро падало в
/// `handle_alloc_error` (`alloc.rs:699`).
fn print_file(console: &mut Console, name: &str) -> Result<(), fs::FsError> {
    let mut file = fs::open_file(name)?;
    let size = file.size();

    let mut buf = alloc::vec![0u8; CHUNK];

    // Первая порция определяет, текст это или двоичные данные: так `cat`
    // не выводит мусор в консоль и не тратит кучу на весь файл.
    let first = file.read(&mut buf)?;
    if first == 0 {
        return Ok(());
    }
    let is_text = match core::str::from_utf8(&buf[..first]) {
        Ok(_) => true,
        // Неполный UTF-8 на границе порции — ещё не признак бинарника.
        Err(error) => error.error_len().is_none(),
    };

    if !is_text {
        console.print_colored("(binary data, ", COLOR_RED);
        console.print_num(size);
        console.print(" bytes)\n");
        return Ok(());
    }

    let mut pending: Vec<u8> = Vec::new();
    let mut last_byte: Option<u8> = None;
    let mut chunk = first;
    loop {
        if chunk == 0 {
            break;
        }
        pending.extend_from_slice(&buf[..chunk]);

        match core::str::from_utf8(&pending) {
            Ok(text) => {
                console.print(text);
                last_byte = text.as_bytes().last().copied();
                pending.clear();
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if valid > 0 {
                    if let Ok(text) = core::str::from_utf8(&pending[..valid]) {
                        console.print(text);
                        last_byte = text.as_bytes().last().copied();
                    }
                }
                if error.error_len().is_some() {
                    // Встретился настоящий мусор: дальше не печатаем.
                    console.print_colored("\n(binary data, ", COLOR_RED);
                    console.print_num(size);
                    console.print(" bytes)\n");
                    return Ok(());
                }
                // Неполный символ — переносим его в следующую порцию.
                pending.drain(..valid);
            }
        }

        chunk = file.read(&mut buf)?;
    }

    // Хвост, не сложившийся в целый символ, не печатаем: это обрезанный UTF-8.
    pending.clear();

    // Файл без завершающего перевода строки: добавляем, чтобы prompt не
    // приклеивался к последней строке.
    if let Some(byte) = last_byte {
        if byte != b'\n' {
            console.put_char('\n');
        }
    }

    Ok(())
}
