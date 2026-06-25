use anyhow::{Context, Result};
use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use std::thread::sleep;
use std::time::Duration;

// 粘贴热键的修饰键随平台变：macOS 用 Command(⌘+V)，其余用 Ctrl(Ctrl+V)。
// enigo 在 macOS 把 Key::Meta 映射成 Command 键。
#[cfg(target_os = "macos")]
const PASTE_MODIFIER: Key = Key::Meta;
#[cfg(not(target_os = "macos"))]
const PASTE_MODIFIER: Key = Key::Control;

// V 键：macOS 直接用物理键码 kVK_ANSI_V=9（enigo Key::Other 发原始 CGKeyCode），
// 绕开 Key::Unicode('v') 的 UCKeyTranslate 布局反查——中文等非拉丁输入法激活时
// 那条路会以 -25340 失败，导致 ⌘V 的 V 根本没发出去、粘贴静默落空。
// Windows 端 Unicode('v') 没这个问题，保持不变。
#[cfg(target_os = "macos")]
const PASTE_V: Key = Key::Other(9);
#[cfg(not(target_os = "macos"))]
const PASTE_V: Key = Key::Unicode('v');

/// 把文本写入系统剪贴板，模拟一次粘贴（macOS=⌘V / 其它=Ctrl+V）到当前焦点窗口，
/// 然后恢复用户原来剪贴板里的内容（如果是文本）。
///
/// 恢复有 100ms 的窗口期，期间如果用户主动复制了新内容会被覆盖；
/// 但日常使用中粘贴完很少会立即去复制别的，覆盖风险可接受。
///
/// macOS 注意：合成按键需要"辅助功能(Accessibility)"权限，否则 enigo 静默失败、
/// 文本只会停在剪贴板里不会被粘贴。首次运行需在系统设置里给终端授权。
pub fn paste(text: &str) -> Result<()> {
    let mut clipboard = Clipboard::new().context("init clipboard")?;

    // 备份原剪贴板（仅支持文本；图片/文件等非文本内容无法保留，会丢）
    let original = clipboard.get_text().ok();

    clipboard.set_text(text).context("write clipboard")?;
    // 给目标程序一点时间感知剪贴板更新
    sleep(Duration::from_millis(50));

    let mut enigo = Enigo::new(&Settings::default()).context("init enigo")?;
    enigo
        .key(PASTE_MODIFIER, Direction::Press)
        .context("press paste modifier")?;
    enigo
        .key(PASTE_V, Direction::Click)
        .context("press V")?;
    enigo
        .key(PASTE_MODIFIER, Direction::Release)
        .context("release paste modifier")?;

    // 等粘贴动作落地，再恢复原剪贴板内容
    sleep(Duration::from_millis(100));
    if let Some(orig) = original {
        let _ = clipboard.set_text(orig);
    }

    Ok(())
}
