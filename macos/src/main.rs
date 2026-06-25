// macOS 下用菜单栏托盘 + egui 设置窗口；引擎跑在后台线程。
// 注意：不能用 #[tokio::main]——eframe 要独占主线程跑事件循环，
// tokio runtime 由 engine 线程自己起。
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod audio;
mod encode;
mod engine;
mod hotkey;
mod llm;
mod paste;
mod stt;
mod ui;

use app::{AppConfig, Shared};
use eframe::egui;

fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    init_logging();
    install_panic_hook();

    #[cfg(target_os = "macos")]
    macos_request_accessibility();

    let cfg = AppConfig::load();
    let hotkey_name = cfg.hotkey.clone();
    let shared = Shared::new(cfg);

    // 全局热键监听独立线程；引擎线程自己起麦克风（cpal Stream !Send，不能跨线程搬）。
    // 麦克风/权限出问题不会崩 UI，错误会写进 status 显示出来。
    let hotkey_rx = hotkey::spawn_listener(&hotkey_name).unwrap_or_else(|e| {
        tracing::warn!("hotkey {hotkey_name:?} 无效（{e}），退回默认");
        hotkey::spawn_listener(&app::default_hotkey()).expect("default hotkey valid")
    });
    engine::spawn(shared.clone(), hotkey_rx);

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([388.0, 272.0])
            .with_min_inner_size([340.0, 230.0])
            .with_title("SaySo"),
        ..Default::default()
    };

    let ui_shared = shared.clone();
    eframe::run_native(
        "SaySo",
        native_options,
        Box::new(move |cc| Ok(Box::new(ui::SaySoApp::new(cc, ui_shared)))),
    )
    .map_err(|e| anyhow::anyhow!("eframe: {e}"))?;
    Ok(())
}

/// 日志写到 ~/Library/Logs/SaySo.log（独立 app 没终端，靠这个排查），同时尽量也打 stderr。
fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info".into());
    let log_file = dirs::home_dir().and_then(|h| {
        let p = h.join("Library/Logs/SaySo.log");
        std::fs::OpenOptions::new().create(true).append(true).open(&p).ok()
    });
    match log_file {
        Some(f) => {
            let f = std::sync::Arc::new(f);
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_ansi(false)
                .with_writer(move || FileWriter(f.clone()))
                .init();
        }
        None => {
            tracing_subscriber::fmt().with_env_filter(filter).init();
        }
    }
}

struct FileWriter(std::sync::Arc<std::fs::File>);
impl std::io::Write for FileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut f: &std::fs::File = &self.0;
        f.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let mut f: &std::fs::File = &self.0;
        f.flush()
    }
}

/// macOS：主动请求"辅助功能"权限（粘贴用 enigo 合成 ⌘V 需要它）。
/// 带 prompt 选项调用，会把 SaySo 登记进 系统设置→辅助功能 列表并弹框，
/// 否则 app 不主动碰 AX、用户在列表里根本找不到它。
#[cfg(target_os = "macos")]
fn macos_request_accessibility() {
    use std::ffi::c_void;
    use std::ptr;
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
        static kAXTrustedCheckOptionPrompt: *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            num_values: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
        static kCFBooleanTrue: *const c_void;
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
    }
    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [kCFBooleanTrue];
        let opts = CFDictionaryCreate(
            ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &kCFTypeDictionaryKeyCallBacks as *const _ as *const c_void,
            &kCFTypeDictionaryValueCallBacks as *const _ as *const c_void,
        );
        let trusted = AXIsProcessTrustedWithOptions(opts);
        if !opts.is_null() {
            CFRelease(opts);
        }
        if !trusted {
            tracing::warn!("缺『辅助功能』权限（粘贴用）——已登记到列表并弹框，授权后重启 SaySo。");
        }
    }
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("!! PANIC !! {info}");
        if let Some(loc) = info.location() {
            tracing::error!("  at {}:{}:{}", loc.file(), loc.line(), loc.column());
        }
        default_hook(info);
    }));
}
