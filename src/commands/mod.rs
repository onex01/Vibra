pub mod help;
pub mod version;
pub mod ls;
pub mod touch;
pub mod cd;
pub mod cp;
pub mod mv;
pub mod mkdir;
pub mod cat;
pub mod rm;
pub mod edit;
pub mod echo;
pub mod clear;
pub mod tasks;
pub mod pwd;
pub mod uptime;
pub mod quit;
pub mod diag;
pub mod heap;
pub mod neofetch;
pub mod mount;
pub mod test_disk;
pub mod df;
pub mod kstat;
pub mod uname;
pub mod reboot;
pub mod tree;
pub mod top;
pub mod whoami;
pub mod su;
pub mod id;
pub mod passwd;
pub mod free;
pub mod ps;
pub mod chmod;
pub mod hostname;
pub mod date;
pub mod lspci;
pub mod usertest;
pub mod kill;
pub mod apic;

use crate::framebuffer::Console;

/// Результат выполнения команды
pub enum CmdResult {
    Ok,
    Continue, // для команд типа clear, которые меняют состояние консоли
    Exit,
}

pub type CmdFn = fn(&[&str], &mut Console) -> CmdResult;

pub struct Command {
    pub name: &'static str,
    pub help: &'static str,
    pub func: CmdFn,
}

pub const COMMANDS: &[Command] = &[
    Command { name: "help",    help: "show this help",             func: help::run },
    Command { name: "version", help: "show OS/kernel version",     func: version::run },
    Command { name: "clear",   help: "clear screen",               func: clear::run },
    Command { name: "ls",      help: "list files and directories", func: ls::run },
    Command { name: "pwd",     help: "print working directory",    func: pwd::run },
    Command { name: "touch",   help: "create empty file",          func: touch::run },
    Command { name: "cd",      help: "change directory",           func: cd::run },
    Command { name: "cp",      help: "copy file",                  func: cp::run },
    Command { name: "mv",      help: "move/rename file",           func: mv::run },
    Command { name: "mkdir",   help: "create directory",           func: mkdir::run },
    Command { name: "cat",     help: "print file contents",        func: cat::run },
    Command { name: "edit",    help: "edit/create file contents",  func: edit::run },
    Command { name: "echo",    help: "print text to screen",       func: echo::run },
    Command { name: "rm",      help: "remove file or directory",   func: rm::run },
    Command { name: "tasks",   help: "show running processes",     func: tasks::run },
    Command { name: "df",      help: "show disk/filesystem usage", func: df::run },
    Command { name: "uptime",  help: "show system uptime",         func: uptime::run },
    Command { name: "diag",    help: "kernel diagnostics tests",   func: diag::run },
    Command { name: "heap",    help: "show heap usage",            func: heap::run },
    Command { name: "neofetch", help: "system info (logo + info)",  func: neofetch::run },
    Command { name: "mount",   help: "mount filesystems",          func: mount::run },
    Command { name: "test-disk", help: "test disk operations",     func: test_disk::run },
    Command { name: "kstat",    help: "show interrupt statistics", func: kstat::run },
    Command { name: "uname",    help: "system information",        func: uname::run },
    Command { name: "reboot",   help: "reboot the system",        func: reboot::run },
    Command { name: "tree",     help: "show directory tree",      func: tree::run },
    Command { name: "top",      help: "system monitor (htop-like)", func: top::run },
    Command { name: "whoami",   help: "show current user",         func: whoami::run },
    Command { name: "id",       help: "show user/group ids",       func: id::run },
    Command { name: "su",       help: "switch user",               func: su::run },
    Command { name: "passwd",   help: "change password",           func: passwd::run },
    Command { name: "free",     help: "show memory usage",         func: free::run },
    Command { name: "ps",       help: "show running processes",    func: ps::run },
    Command { name: "chmod",    help: "change file permissions",   func: chmod::run },
    Command { name: "hostname", help: "show/set system hostname",  func: hostname::run },
    Command { name: "date",     help: "show current time",        func: date::run },
    Command { name: "apic",    help: "APIC management/status",    func: apic::run },
    Command { name: "lspci",   help: "list PCI devices",          func: lspci::run },
    Command { name: "usertest", help: "run user-space process",  func: usertest::run },
    Command { name: "kill",    help: "kill a process by PID",   func: kill::run },
    Command { name: "quit",    help: "halt the system",            func: quit::run },
];

pub fn find_command(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.name == name)
}

/// Для tab-completion: список всех имен команд
pub fn command_names() -> impl Iterator<Item = &'static str> {
    COMMANDS.iter().map(|c| c.name)
}