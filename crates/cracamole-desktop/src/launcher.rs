//! 預設的 Windows 桌面入口：啟動現代化的 GPUI 版（`desktop-gpui/`）。
//!
//! Win32 原生版是備援，執行檔為 `cracamole-desktop-win32`
//! （`cargo run -p cracamole-desktop --bin cracamole-desktop-win32`，或設定
//! `CRACAMOLE_DESKTOP_UI=win32` 後再執行本入口）。
//!
//! 本入口不處理任何帳密：管理員密碼等環境變數原樣傳給子程序，不記錄也不顯示。

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const GPUI_BINARY: &str = "cracamole-desktop-gpui";
const WIN32_BINARY: &str = "cracamole-desktop-win32";

fn profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

fn gpui_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../desktop-gpui")
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// 啟動子程序並沿用其結束碼。
fn run(binary: &Path) -> ExitCode {
    match Command::new(binary).status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8),
        Err(error) => {
            eprintln!("無法啟動 {}：{error}", binary.display());
            ExitCode::FAILURE
        }
    }
}

/// 啟動 Win32 備援版：必須已建置在本入口旁邊。
fn run_win32() -> ExitCode {
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|dir| dir.join(exe(WIN32_BINARY))));
    match sibling.filter(|path| path.exists()) {
        Some(path) => run(&path),
        None => {
            eprintln!(
                "找不到 Win32 備援版。請先建置：cargo build -p cracamole-desktop --bin {WIN32_BINARY} --locked"
            );
            ExitCode::FAILURE
        }
    }
}

/// 第一次執行時建置 GPUI 版（元件樹很深，rustc 需要較大的堆疊）。
fn build_gpui(root: &Path) -> bool {
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .env("RUST_MIN_STACK", "536870912");
    if !cfg!(debug_assertions) {
        command.arg("--release");
    }
    eprintln!("第一次啟動：建置 GPUI 版（約數分鐘）…");
    command.status().is_ok_and(|status| status.success())
}

fn main() -> ExitCode {
    if !cfg!(windows) {
        eprintln!("桌面介面目前僅支援 Windows。");
        return ExitCode::FAILURE;
    }
    let forced_win32 =
        std::env::var("CRACAMOLE_DESKTOP_UI").is_ok_and(|v| v.eq_ignore_ascii_case("win32"));
    if forced_win32 || std::env::args().any(|arg| arg == "--win32") {
        return run_win32();
    }
    let root = gpui_root();
    let binary = root.join("target").join(profile()).join(exe(GPUI_BINARY));
    if !binary.exists() && !build_gpui(&root) {
        eprintln!(
            "GPUI 版建置失敗。可改用 Win32 備援版：cargo run -p cracamole-desktop --bin {WIN32_BINARY} --locked"
        );
        return ExitCode::FAILURE;
    }
    run(&binary)
}
