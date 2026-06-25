//! 口述引擎：在后台 tokio 线程上跑「等热键→录音→转写→润色→粘贴」的循环。
//! 每次会话开头从 Shared 取一份配置快照，所以 UI 改了设置下次会话即生效
//! （热键键位除外——rdev 监听在启动时固定，换键位需重启）。

use crate::app::{EngineState, Shared};
use crate::audio::AudioEngine;
use crate::hotkey::{self, HotkeyEvent};
use crate::{encode, llm, paste, stt};
use std::time::Instant;
use tokio::sync::mpsc::UnboundedReceiver;

/// env `GROQ_API_KEY` 优先，其次配置文件里的 api_key。
fn resolve_key(cfg_key: &str) -> Option<String> {
    if let Ok(k) = std::env::var("GROQ_API_KEY") {
        if !k.trim().is_empty() {
            return Some(k);
        }
    }
    let k = cfg_key.trim();
    if k.is_empty() {
        None
    } else {
        Some(k.to_string())
    }
}

/// 在新线程起当前线程 tokio runtime，跑引擎主循环。引擎崩了也不该带走 UI。
/// cpal::Stream 是 !Send，所以 AudioEngine 必须在这条线程里创建并常驻，不跨线程搬。
pub fn spawn(shared: Shared, hotkey_rx: UnboundedReceiver<HotkeyEvent>) {
    std::thread::Builder::new()
        .name("sayso-engine".into())
        .spawn(move || {
            let audio = match AudioEngine::start() {
                Ok(a) => a,
                Err(e) => {
                    tracing::error!("audio engine failed: {e:#}");
                    shared.with_status(|s| {
                        s.state = EngineState::Error;
                        s.last_error = format!("麦克风启动失败：{e:#}（检查麦克风权限/设备）");
                    });
                    return;
                }
            };
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    shared.with_status(|s| {
                        s.state = EngineState::Error;
                        s.last_error = format!("tokio runtime 启动失败: {e}");
                    });
                    return;
                }
            };
            rt.block_on(run_loop(shared, audio, hotkey_rx));
        })
        .expect("spawn engine thread");
}

async fn run_loop(
    shared: Shared,
    audio: AudioEngine,
    mut hotkey_rx: UnboundedReceiver<HotkeyEvent>,
) {
    loop {
        match one_session(&shared, &audio, &mut hotkey_rx).await {
            Ok(()) => {}
            Err(e) => {
                // 通道关闭 = rdev 监听线程已死，几乎都是 macOS 没授「输入监控」。
                if hotkey_rx.is_closed() {
                    shared.with_status(|s| {
                        s.state = EngineState::Error;
                        s.permission_problem = true;
                        s.last_error =
                            "全局热键监听异常退出，请退出并重新打开 SaySo。若粘贴不生效，到\
                             『系统设置 → 隐私与安全性 → 辅助功能』勾选 SaySo。"
                                .into();
                    });
                    tracing::error!("hotkey listener gone — engine loop exiting");
                    return;
                }
                tracing::error!("session error: {e:#}");
                shared.with_status(|s| {
                    s.state = EngineState::Error;
                    s.last_error = format!("{e:#}");
                });
            }
        }
    }
}

async fn one_session(
    shared: &Shared,
    audio: &AudioEngine,
    rx: &mut UnboundedReceiver<HotkeyEvent>,
) -> anyhow::Result<()> {
    // 等"开始"
    hotkey::wait_for(rx, HotkeyEvent::Press).await?;
    let cfg = shared.config_snapshot();
    let toggle = cfg.trigger_mode.trim().eq_ignore_ascii_case("toggle")
        || cfg.trigger_mode.trim().eq_ignore_ascii_case("tap");

    audio.begin_session();
    shared.set_state(EngineState::Recording);
    let started = Instant::now();
    tracing::info!("● recording…");

    let stop = if toggle { HotkeyEvent::Press } else { HotkeyEvent::Release };
    hotkey::wait_for(rx, stop).await?;

    let recording = audio.end_session();
    let dur = started.elapsed();
    let peak = recording.peak_amplitude();
    shared.with_status(|s| s.last_peak = peak);
    tracing::info!(
        "captured {:.2}s peak={} rms={}",
        recording.duration_secs(),
        peak,
        recording.rms_amplitude()
    );

    if recording.is_silent(cfg.silence_threshold) || dur.as_secs_f32() < 0.3 {
        tracing::warn!("too quiet/short (peak {peak} < {}), skipping", cfg.silence_threshold);
        shared.set_state(EngineState::Idle);
        return Ok(());
    }

    let Some(key) = resolve_key(&cfg.api_key) else {
        shared.with_status(|s| {
            s.state = EngineState::Error;
            s.last_error = "未设置 API key（env GROQ_API_KEY 或设置窗口里填）".into();
        });
        return Ok(());
    };

    shared.set_state(EngineState::Transcribing);
    let proxy = Some(cfg.proxy.clone()).filter(|p| !p.trim().is_empty());
    let wav = encode::pcm_to_wav(&recording.samples, recording.sample_rate)?;
    let stt_cfg = stt::Config {
        base_url: cfg.stt_base_url.clone(),
        model: cfg.stt_model.clone(),
        api_key: key.clone(),
        proxy: proxy.clone(),
        prompt: Some(cfg.stt_prompt.clone()).filter(|p| !p.trim().is_empty()),
        min_logprob: cfg.stt_min_logprob,
    };
    let raw = stt::transcribe(wav, peak, &stt_cfg).await?;
    let raw = raw.trim().to_string();
    if raw.is_empty() {
        tracing::warn!("empty/filtered transcription, skipping");
        shared.set_state(EngineState::Idle);
        return Ok(());
    }
    tracing::info!("raw: {raw}");
    shared.with_status(|s| s.last_raw = raw.clone());

    shared.set_state(EngineState::Polishing);
    let llm_cfg = llm::Config {
        base_url: cfg.llm_base_url.clone(),
        model: cfg.llm_model.clone(),
        api_key: key,
        proxy,
        system_prompt: llm::compose_system_prompt(&cfg.active_style()),
        enabled: cfg.llm_enabled,
    };
    let final_text = match llm::polish(&raw, &llm_cfg).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("polish failed ({e:#}), using raw");
            raw.clone()
        }
    };

    shared.set_state(EngineState::Pasting);
    let for_paste = final_text.clone();
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || paste::paste(&for_paste)));
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            tracing::error!("paste failed: {e:#}");
            shared.with_status(|s| s.last_error = format!("粘贴失败: {e:#}"));
        }
        Err(_) => tracing::error!("paste panicked"),
    }

    shared.with_status(|s| {
        s.last_final = final_text.clone();
        s.history.insert(0, final_text);
        s.history.truncate(20);
        s.state = EngineState::Idle;
    });
    Ok(())
}
