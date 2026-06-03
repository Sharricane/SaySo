use anyhow::Result;
use rdev::{Event, EventType, Key};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Press,
    Release,
}

/// 解析 SAYSO_HOTKEY 配置（大小写不敏感，多写法兼容）
pub fn parse_hotkey(s: &str) -> Result<Key> {
    Ok(match s.trim().to_lowercase().as_str() {
        "rightalt" | "ralt" | "altgr" => Key::AltGr,
        "rightctrl" | "rctrl" | "controlright" => Key::ControlRight,
        "leftctrl" | "lctrl" | "controlleft" => Key::ControlLeft,
        "rightshift" | "rshift" | "shiftright" => Key::ShiftRight,
        "leftshift" | "lshift" | "shiftleft" => Key::ShiftLeft,
        "pause" | "pausebreak" => Key::Pause,
        "scrolllock" => Key::ScrollLock,
        other => anyhow::bail!(
            "unknown hotkey '{other}' — supported: RightAlt(default), RightCtrl, RightShift, Pause, ScrollLock"
        ),
    })
}

/// 在独立 OS 线程跑 rdev 全局钩子，把目标键的按下/松开事件透传到 mpsc。
///
/// rdev 是同步阻塞 API，不能在 tokio 任务里直接用；同时 Windows hook
/// 必须在创建它的线程上 pump 消息，所以必须独立 std::thread。
pub fn spawn_listener(target: Key) -> mpsc::UnboundedReceiver<HotkeyEvent> {
    let (tx, rx) = mpsc::unbounded_channel();

    std::thread::Builder::new()
        .name("hotkey-listener".into())
        .spawn(move || {
            // 去重：同一按键的连续 KeyPress 在系统层会触发 auto-repeat，
            // 我们只想要"第一次按下"和"最终松开"两次事件。
            let mut held = false;

            let callback = move |event: Event| match event.event_type {
                EventType::KeyPress(k) if k == target && !held => {
                    held = true;
                    let _ = tx.send(HotkeyEvent::Press);
                }
                EventType::KeyRelease(k) if k == target && held => {
                    held = false;
                    let _ = tx.send(HotkeyEvent::Release);
                }
                _ => {}
            };

            if let Err(e) = rdev::listen(callback) {
                tracing::error!("rdev listener exited: {e:?}");
            }
        })
        .expect("spawn hotkey-listener thread");

    rx
}

/// 阻塞 await 直到收到指定的事件，丢弃其他事件。
pub async fn wait_for(
    rx: &mut mpsc::UnboundedReceiver<HotkeyEvent>,
    want: HotkeyEvent,
) -> Result<()> {
    while let Some(ev) = rx.recv().await {
        if ev == want {
            return Ok(());
        }
    }
    anyhow::bail!("hotkey channel closed unexpectedly")
}
