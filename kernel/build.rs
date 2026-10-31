// Build metadata for `version`, `uname` and release artifacts.
//
// Значения попадают в ядро через env!() и пересобираются при изменении
// version.rs. Дату можно зафиксировать через SOURCE_DATE_EPOCH для
// воспроизводимых сборок.

use std::process::Command;

fn command_output(command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    Some(text.trim().to_string())
}

fn main() {
    println!("cargo:rerun-if-changed=src/version.rs");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");

    let date = match std::env::var("SOURCE_DATE_EPOCH") {
        Ok(epoch) if !epoch.is_empty() => command_output(
            "date",
            &["-u", "-d", &format!("@{}", epoch), "+%Y-%m-%d"],
        )
        .unwrap_or_else(|| String::from("unknown")),
        _ => command_output("date", &["-u", "+%Y-%m-%d"])
            .unwrap_or_else(|| String::from("unknown")),
    };

    let commit = command_output("git", &["rev-parse", "--short=12", "HEAD"])
        .unwrap_or_else(|| String::from("unknown"));

    let profile = std::env::var("PROFILE").unwrap_or_else(|_| String::from("unknown"));

    println!("cargo:rustc-env=VIBRA_BUILD_DATE={}", date);
    println!("cargo:rustc-env=VIBRA_GIT_COMMIT={}", commit);
    println!("cargo:rustc-env=VIBRA_BUILD_PROFILE={}", profile);
}
