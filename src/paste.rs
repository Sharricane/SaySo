use anyhow::{Context, Result};
use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use std::thread::sleep;
use std::time::Duration;

/// 把文本写入系统剪贴板，模拟一次 Ctrl+V 粘贴到当前焦点窗口，
/// 然后恢复用户原来剪贴板里的内容（如果是文本）。
///
/// 恢复有 100ms 的窗口期，期间如果用户主动复制了新内容会被覆盖；
/// 但日常使用中粘贴完很少会立即去复制别的，覆盖风险可接受。
pub fn paste(text: &str) -> Result<()> {
    let mut clipboard = Clipboard::new().context("init clipboard")?;

    // 备份原剪贴板（仅支持文本；图片/文件等非文本内容无法保留，会丢）
    let original = clipboard.get_text().ok();

    clipboard.set_text(text).context("write clipboard")?;
    // 给目标程序一点时间感知剪贴板更新
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

    // 等粘贴动作落地，再恢复原剪贴板内容
    sleep(Duration::from_millis(100));
    if let Some(orig) = original {
        let _ = clipboard.set_text(orig);
    }

    Ok(())
}
