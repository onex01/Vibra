// User Management System — система пользователей Vibra OS
//
// Поддерживает: uid/gid, домашние директории, аутентификация.
// Пока упрощённая реализация без шифрования паролей.

use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

/// Пользователь системы
#[derive(Clone)]
pub struct User {
    pub uid: u32,
    pub gid: u32,
    pub name: String,
    pub home: String,
    pub shell: String,
    pub password_hash: u64, // Простой хеш (для demo)
}

impl User {
    pub fn new(uid: u32, gid: u32, name: &str, home: &str, shell: &str) -> Self {
        Self {
            uid,
            gid,
            name: String::from(name),
            home: String::from(home),
            shell: String::from(shell),
            password_hash: 0, // Нет пароля по умолчанию
        }
    }
}

/// Текущая сессия пользователя
#[derive(Clone)]
pub struct Session {
    pub uid: u32,
    pub gid: u32,
    pub username: String,
    pub home: String,
}

static USERS: Mutex<Vec<User>> = Mutex::new(Vec::new());
static CURRENT_SESSION: Mutex<Option<Session>> = Mutex::new(None);

/// Инициализация системы пользователей
pub fn init() {
    // Создаём стандартных пользователей
    let mut users = USERS.lock();
    users.push(User::new(0, 0, "root", "/home/root", "/bin/sh"));
    users.push(User::new(1000, 1000, "user", "/home/user", "/bin/sh"));

    // Устанавливаем текущую сессию (root)
    *CURRENT_SESSION.lock() = Some(Session {
        uid: 0,
        gid: 0,
        username: String::from("root"),
        home: String::from("/home/root"),
    });

    crate::println!("[USER] User system initialized (root, user)");
}

/// Получить текущего пользователя
pub fn current_user() -> Session {
    CURRENT_SESSION.lock().clone().unwrap_or_else(|| Session {
        uid: 0,
        gid: 0,
        username: String::from("root"),
        home: String::from("/home/root"),
    })
}

/// Получить uid текущего пользователя
pub fn current_uid() -> u32 {
    current_user().uid
}

/// Получить имя текущего пользователя
pub fn current_username() -> String {
    current_user().username
}

/// Переключить пользователя (su)
pub fn switch_user(username: &str) -> Result<(), &'static str> {
    let users = USERS.lock();
    for user in users.iter() {
        if user.name == username {
            *CURRENT_SESSION.lock() = Some(Session {
                uid: user.uid,
                gid: user.gid,
                username: user.name.clone(),
                home: user.home.clone(),
            });
            return Ok(());
        }
    }
    Err("user not found")
}

/// Проверить пароль (упрощённо — всегда OK для demo)
pub fn check_password(_username: &str, _password: &str) -> bool {
    // В демо-режиме все пароли принимаются
    true
}

/// Получить информацию о пользователе по uid
pub fn get_user_by_uid(uid: u32) -> Option<User> {
    let users = USERS.lock();
    for user in users.iter() {
        if user.uid == uid {
            return Some(user.clone());
        }
    }
    None
}

/// Получить список всех пользователей
pub fn list_users() -> Vec<User> {
    USERS.lock().clone()
}

/// Добавить нового пользователя
pub fn add_user(uid: u32, gid: u32, name: &str, home: &str) -> Result<(), &'static str> {
    let mut users = USERS.lock();
    // Проверяем уникальность uid
    if users.iter().any(|u| u.uid == uid) {
        return Err("uid already exists");
    }
    if users.iter().any(|u| u.name == name) {
        return Err("username already exists");
    }
    users.push(User::new(uid, gid, name, home, "/bin/sh"));
    Ok(())
}

/// Удалить пользователя
pub fn remove_user(name: &str) -> Result<(), &'static str> {
    let mut users = USERS.lock();
    if let Some(pos) = users.iter().position(|u| u.name == name) {
        if users[pos].uid == 0 {
            return Err("cannot remove root");
        }
        users.remove(pos);
        Ok(())
    } else {
        Err("user not found")
    }
}
