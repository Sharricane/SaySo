use anyhow::Result;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Press,
    Release,
}

/// 起全局热键监听，把目标键的按下/松开透传到 mpsc。
///
/// macOS：用自建的 CGEventTap，只监听 flagsChanged 并读修饰键标志位——
/// **不碰 rdev 那条会崩的 TSM/TIS 字符翻译路径**（macOS 26 起那些 API 断言
/// 必须在主线程，配 eframe 主循环就 SIGTRAP）。其它平台仍用 rdev。
pub fn spawn_listener(hotkey_name: &str) -> Result<mpsc::UnboundedReceiver<HotkeyEvent>> {
    let (tx, rx) = mpsc::unbounded_channel();

    #[cfg(target_os = "macos")]
    {
        let mask = macos::mask_for(hotkey_name)?;
        macos::spawn(mask, tx);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let target = parse_hotkey(hotkey_name)?;
        rdev_listen::spawn(target, tx);
    }

    Ok(rx)
}

/// 阻塞 await 直到收到指定事件，丢弃其它。
pub async fn wait_for(rx: &mut mpsc::UnboundedReceiver<HotkeyEvent>, want: HotkeyEvent) -> Result<()> {
    while let Some(ev) = rx.recv().await {
        if ev == want {
            return Ok(());
        }
    }
    anyhow::bail!("hotkey channel closed unexpectedly")
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{HotkeyEvent, mpsc};
    use anyhow::Result;
    use std::ffi::c_void;
    use std::ptr;

    // CGEventFlags 里的"设备相关"修饰键位（区分左右）。enigo 合成右修饰键时也会置这些位，
    // 所以同一套 mask 既能给真人按键用，也能用 enigo 合成做自动化测试。
    const DEV_L_CTRL: u64 = 0x0000_0001;
    const DEV_L_SHIFT: u64 = 0x0000_0002;
    const DEV_R_SHIFT: u64 = 0x0000_0004;
    const DEV_L_CMD: u64 = 0x0000_0008;
    const DEV_R_CMD: u64 = 0x0000_0010;
    const DEV_L_ALT: u64 = 0x0000_0020;
    const DEV_R_ALT: u64 = 0x0000_0040;
    const DEV_R_CTRL: u64 = 0x0000_2000;
    const FN_MASK: u64 = 0x0080_0000; // kCGEventFlagMaskSecondaryFn

    // CGEventTapCreate 参数 / 事件类型常量
    const SESSION_EVENT_TAP: u32 = 1; // kCGSessionEventTap（物理 + 会话内合成都看得到）
    const HEAD_INSERT: u32 = 0;
    const LISTEN_ONLY: u32 = 1; // kCGEventTapOptionListenOnly（只需"输入监控"权限）
    const FLAGS_CHANGED: u32 = 12; // kCGEventFlagsChanged
    const TAP_DISABLED_TIMEOUT: u32 = 0xFFFF_FFFE;
    const TAP_DISABLED_USER_INPUT: u32 = 0xFFFF_FFFF;

    type CFRef = *mut c_void;
    type CGEventRef = *mut c_void;
    type CGEventTapProxy = *mut c_void;
    type Callback = extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventTapCreate(
            tap: u32,
            place: u32,
            options: u32,
            events_of_interest: u64,
            callback: Callback,
            user_info: *mut c_void,
        ) -> CFRef;
        fn CGEventTapEnable(tap: CFRef, enable: bool);
        fn CGEventGetFlags(event: CGEventRef) -> u64;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFMachPortCreateRunLoopSource(allocator: CFRef, port: CFRef, order: isize) -> CFRef;
        fn CFRunLoopGetCurrent() -> CFRef;
        fn CFRunLoopAddSource(rl: CFRef, source: CFRef, mode: *const c_void);
        fn CFRunLoopRun();
        static kCFRunLoopCommonModes: *const c_void;
    }

    struct TapState {
        tx: mpsc::UnboundedSender<HotkeyEvent>,
        mask: u64,
        down: bool,
        port: CFRef,
    }

    pub fn mask_for(name: &str) -> Result<u64> {
        Ok(match name.trim().to_lowercase().as_str() {
            "rightoption" | "roption" | "optionright" | "rightalt" | "ralt" | "altgr" => DEV_R_ALT,
            "leftoption" | "loption" | "optionleft" | "leftalt" | "lalt" => DEV_L_ALT,
            "rightcommand" | "rcommand" | "rightcmd" | "rcmd" | "commandright" | "rightmeta"
            | "rmeta" => DEV_R_CMD,
            "leftcommand" | "lcommand" | "leftcmd" | "lcmd" | "leftmeta" | "lmeta" => DEV_L_CMD,
            "rightctrl" | "rctrl" | "controlright" | "rightcontrol" => DEV_R_CTRL,
            "leftctrl" | "lctrl" | "controlleft" | "leftcontrol" => DEV_L_CTRL,
            "rightshift" | "rshift" | "shiftright" => DEV_R_SHIFT,
            "leftshift" | "lshift" | "shiftleft" => DEV_L_SHIFT,
            "fn" | "function" | "globe" => FN_MASK,
            other => anyhow::bail!(
                "unknown hotkey '{other}' — macOS: RightOption(default), RightCommand, Fn, RightCtrl, RightShift"
            ),
        })
    }

    extern "C" fn tap_callback(
        _proxy: CGEventTapProxy,
        etype: u32,
        event: CGEventRef,
        user_info: *mut c_void,
    ) -> CGEventRef {
        // SAFETY: user_info 是 spawn 里 leak 的 TapState，且回调只在本 tap 线程跑，独占访问。
        let st = unsafe { &mut *(user_info as *mut TapState) };
        if etype == TAP_DISABLED_TIMEOUT || etype == TAP_DISABLED_USER_INPUT {
            if !st.port.is_null() {
                unsafe { CGEventTapEnable(st.port, true) };
            }
            return event;
        }
        if etype == FLAGS_CHANGED {
            let flags = unsafe { CGEventGetFlags(event) };
            let now = (flags & st.mask) != 0;
            if now != st.down {
                st.down = now;
                let _ = st.tx.send(if now { HotkeyEvent::Press } else { HotkeyEvent::Release });
            }
        }
        event
    }

    pub fn spawn(mask: u64, tx: mpsc::UnboundedSender<HotkeyEvent>) {
        std::thread::Builder::new()
            .name("hotkey-tap".into())
            .spawn(move || unsafe {
                let state = Box::into_raw(Box::new(TapState {
                    tx,
                    mask,
                    down: false,
                    port: ptr::null_mut(),
                }));

                // 关键：tap 一旦成功建过就绝不 drop tx（系统偶尔会把 listen tap 拔掉、
                // CFRunLoopRun 返回，旧写法在那 drop(tx) → 通道关闭 → 引擎永久停摆）。改成原地重建。
                let mut created_once = false;
                loop {
                    // 只监听 flagsChanged（修饰键）。重要：监听修饰键状态**不需要**"输入监控"
                    // 权限——它不暴露你打了什么字，只有 keyDown/keyUp 才要。所以这里不查、不要
                    // 输入监控权限（之前误加 CGPreflight 把能用的热键给挡了）。
                    let port = CGEventTapCreate(
                        SESSION_EVENT_TAP,
                        HEAD_INSERT,
                        LISTEN_ONLY,
                        1u64 << FLAGS_CHANGED,
                        tap_callback,
                        state as *mut c_void,
                    );
                    if port.is_null() {
                        if !created_once {
                            tracing::error!("CGEventTapCreate 返回空，无法监听热键（极少见）。");
                            drop(Box::from_raw(state));
                            return;
                        }
                        tracing::warn!("event tap 重建失败，1s 后重试");
                        std::thread::sleep(std::time::Duration::from_secs(1));
                        continue;
                    }
                    created_once = true;
                    (*state).port = port;
                    // 注：重建时旧 source/port 未 CFRelease（极少触发，泄漏可忽略）。
                    let source = CFMachPortCreateRunLoopSource(ptr::null_mut(), port, 0);
                    CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
                    CGEventTapEnable(port, true);
                    CFRunLoopRun(); // 阻塞常驻；若被系统拔掉而返回，下面重建
                    tracing::warn!("event tap 被系统停用，重建中…");
                    std::thread::sleep(std::time::Duration::from_millis(300));
                }
            })
            .expect("spawn hotkey-tap thread");
    }
}

#[cfg(not(target_os = "macos"))]
pub fn parse_hotkey(s: &str) -> Result<rdev::Key> {
    use rdev::Key;
    Ok(match s.trim().to_lowercase().as_str() {
        "rightalt" | "ralt" | "altgr" | "rightoption" | "roption" => Key::AltGr,
        "rightctrl" | "rctrl" | "controlright" => Key::ControlRight,
        "leftctrl" | "lctrl" | "controlleft" => Key::ControlLeft,
        "rightshift" | "rshift" | "shiftright" => Key::ShiftRight,
        "leftshift" | "lshift" | "shiftleft" => Key::ShiftLeft,
        "pause" | "pausebreak" => Key::Pause,
        "scrolllock" => Key::ScrollLock,
        other => anyhow::bail!("unknown hotkey '{other}'"),
    })
}

#[cfg(not(target_os = "macos"))]
mod rdev_listen {
    use super::{HotkeyEvent, mpsc};
    use rdev::{Event, EventType, Key};

    pub fn spawn(target: Key, tx: mpsc::UnboundedSender<HotkeyEvent>) {
        std::thread::Builder::new()
            .name("hotkey-listener".into())
            .spawn(move || {
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
    }
}
