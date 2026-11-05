/// Централизованное хранилище версии Vibra Kernel.
/// Изменяй версии ТОЛЬКО здесь!

/// Версия ядра
pub const KERNEL_VERSION: &str = "0.2.0-rc.25";

/// Кодовое название ядра
pub const KERNEL_CODENAME: &str = "Foundation RC25";

/// Автор проекта
pub const AUTHOR: &str = "OneX01";

/// Год создания
pub const YEAR: &str = "2026";

/// Лицензия
pub const LICENSE: &str = "MIT";

/// Описание проекта
pub const DESCRIPTION: &str = "An experimental Rust kernel for x86_64.";

/// Архитектура
pub const ARCHITECTURE: &str = "x86_64";

/// Дата сборки (UTC, `YYYY-MM-DD`), задаётся `kernel/build.rs`.
pub const BUILD_DATE: &str = env!("VIBRA_BUILD_DATE");

/// Короткий git commit сборки.
pub const GIT_COMMIT: &str = env!("VIBRA_GIT_COMMIT");

/// Профиль сборки: `debug` или `release`.
pub const BUILD_PROFILE: &str = env!("VIBRA_BUILD_PROFILE");
