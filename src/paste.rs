use anyhow::{Context, Result};
use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use std::thread::sleep;
use std::time::Duration;

/// 把文本写入系统剪贴板，然后模拟一次 Ctrl+V 粘贴到当前焦点窗口。
/// 不恢复原剪贴板内容（Phase 2 再做）。
pub fn paste(text: &str) -> Result<()> {
    let mut clipboard = Clipboard::new().context("init clipboard")?;
    clipboard.set_text(text).context("write clipboard")?;

    // 给目标程序一点时间感知剪贴板更新，避免某些 IME / 浏览器吞掉 Ctrl+V。
    sleep(Duration::from_millis(50));

    let mut enigo = Enigo::new(&Settings::default()).context("init enigo")?;
    enigo
        .key(Key::Control, Direction::Press)
        .context("press Ctrl")?;
    enigo
        .key(Key::Unicode('v'), Direction::Click)
        .context("press V")?;
    enigo
        .key(Key::Control, Direction::Release)
        .context("release Ctrl")?;
    Ok(())
}
