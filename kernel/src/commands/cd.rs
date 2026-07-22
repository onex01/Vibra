use super::CmdResult;
use crate::framebuffer::{Console, COLOR_GREEN, COLOR_RED};
use crate::fs;
use alloc::string::String;
use alloc::format;

pub fn run(args: &[&str], console: &mut Console) -> CmdResult {
    let Some(path) = args.first() else {
        console.print_colored("Usage: cd <directory>\n", COLOR_RED);
        return CmdResult::Ok;
    };

    // Обработка特殊的 путей
    match *path {
        "/" => {
            fs::set_current_dir("/");
            return CmdResult::Ok;
        }
        "~" => {
            fs::set_current_dir("/home/root");
            return CmdResult::Ok;
        }
        "-" => {
            // Пока просто /
            fs::set_current_dir("/");
            return CmdResult::Ok;
        }
        _ => {}
    }

    // Обработка ".."
    if *path == ".." {
        let current = fs::get_current_dir();
        let parent = parent_path(&current);
        fs::set_current_dir(&parent);
        return CmdResult::Ok;
    }

    // Обработка относительных и абсолютных путей
    let target = if path.starts_with('/') {
        String::from(*path)
    } else {
        let current = fs::get_current_dir();
        if current == "/" {
            format!("/{}", path)
        } else {
            format!("{}/{}", current, path)
        }
    };

    // Нормализуем путь (убираем . и ..)
    let normalized = normalize_path(&target);

    if fs::dir_exists(&normalized) {
        fs::set_current_dir(&normalized);
    } else {
        console.print_colored("cd: '", COLOR_RED);
        console.print(path);
        console.print_colored("' not found\n", COLOR_RED);
    }

    CmdResult::Ok
}

/// Получить родительский каталог
fn parent_path(path: &str) -> String {
    if path == "/" {
        return String::from("/");
    }

    // Убираем trailing /
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        return String::from("/");
    }

    // Ищем последний /
    if let Some(pos) = path.rfind('/') {
        if pos == 0 {
            String::from("/")
        } else {
            String::from(&path[..pos])
        }
    } else {
        String::from("/")
    }
}

/// Нормализация пути: убирает . и ..
fn normalize_path(path: &str) -> String {
    let mut components: alloc::vec::Vec<&str> = alloc::vec::Vec::new();

    for component in path.split('/') {
        match component {
            "" | "." => continue,
            ".." => { components.pop(); }
            c => components.push(c),
        }
    }

    if components.is_empty() {
        String::from("/")
    } else {
        let mut result = String::new();
        for c in &components {
            result.push('/');
            result.push_str(c);
        }
        result
    }
}
