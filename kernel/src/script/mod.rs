// Vibra Script (.vs) — простой скриптовый язык для Vibra OS.

pub mod lexer;
pub mod parser;
pub mod vm;

use alloc::string::String;

pub enum ScriptResult {
    Ok,
    Error(String),
    Exit,
}

pub fn run_script(source: &str, console: &mut crate::framebuffer::Console) -> ScriptResult {
    let tokens = lexer::tokenize(source);
    if tokens.is_empty() { return ScriptResult::Ok; }

    match parser::parse(&tokens) {
        Ok(program) => vm::execute(&program, console),
        Err(e) => ScriptResult::Error(e),
    }
}

pub fn run_file(path: &str, console: &mut crate::framebuffer::Console) -> ScriptResult {
    match crate::fs::read_file(path) {
        Ok(data) => {
            if let Ok(source) = core::str::from_utf8(&data) {
                run_script(source, console)
            } else {
                ScriptResult::Error(String::from("Invalid UTF-8"))
            }
        }
        Err(_) => ScriptResult::Error(alloc::format!("File not found: {}", path)),
    }
}
