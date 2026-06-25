//! 中心状态：配置（落盘到 config.toml）+ 引擎运行状态。
//! UI 线程读/写配置，引擎线程写状态、UI 线程读状态显示。都走 Arc<Mutex<..>>，
//! 频率低（每次会话几次），锁竞争可忽略。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// 默认 PTT 键：Windows 右 Alt、macOS 右 Option，底层都落在 rdev 的 AltGr。
pub fn default_hotkey() -> String {
    if cfg!(target_os = "macos") { "RightOption" } else { "RightAlt" }.to_string()
}

/// 一个"输出风格"：名字 + 一行短风格描述（喂进固定的 system prompt 模板）。
/// 用户只写这一行风格（比如"改写成西海岸口语英语"），清理纪律由代码兜底。
/// style 为空 = 纯清理、保持原语言。
#[derive(Clone, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub style: String,
}

/// 只给一个「清理润色」打底（空风格=纯清理）。具体风格由用户在 UI 新建。
fn default_presets() -> Vec<Preset> {
    vec![
        Preset {
            name: "清理润色".into(),
            style: String::new(),
        },
        Preset {
            name: "翻译成英语".into(),
            style: "翻译成自然、地道的英语口语，像母语者日常会话那样说，不要书面腔、不要逐字直译"
                .into(),
        },
    ]
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// 配置里的 key；运行时 env `GROQ_API_KEY` 优先级更高（见 engine）。
    pub api_key: String,
    /// HTTP 代理，如 http://127.0.0.1:7890。独立 app 没有 env，国内直连 Groq 常被
    /// Cloudflare 拦 403，填上 Clash 端口走代理即可。空=直连/退回 HTTPS_PROXY 环境变量。
    pub proxy: String,
    pub hotkey: String,
    /// "hold"（按住说话松开停）/ "toggle"（点一下开、再点一下停）
    pub trigger_mode: String,
    pub silence_threshold: i16,

    pub stt_base_url: String,
    pub stt_model: String,
    /// 空串 = 不发 prompt
    pub stt_prompt: String,
    pub stt_min_logprob: f64,

    pub llm_enabled: bool,
    pub llm_base_url: String,
    pub llm_model: String,

    /// 输出风格预设；active_preset 是当前选中下标
    pub presets: Vec<Preset>,
    pub active_preset: usize,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            proxy: String::new(),
            hotkey: default_hotkey(),
            trigger_mode: "hold".into(),
            silence_threshold: 500,
            stt_base_url: crate::stt::DEFAULT_BASE_URL.into(),
            stt_model: crate::stt::DEFAULT_MODEL.into(),
            stt_prompt: crate::stt::DEFAULT_PROMPT.into(),
            stt_min_logprob: crate::stt::DEFAULT_MIN_LOGPROB,
            llm_enabled: true,
            llm_base_url: crate::llm::DEFAULT_BASE_URL.into(),
            llm_model: crate::llm::DEFAULT_MODEL.into(),
            presets: default_presets(),
            active_preset: 0,
        }
    }
}

impl AppConfig {
    /// 当前选中风格的短描述（越界/空表 = 空 = 纯清理）。
    pub fn active_style(&self) -> String {
        self.presets
            .get(self.active_preset)
            .map(|p| p.style.clone())
            .unwrap_or_default()
    }

    /// macOS: ~/Library/Application Support/SaySo/config.toml
    /// Windows: %APPDATA%\SaySo\config.toml
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("SaySo").join("config.toml"))
    }

    pub fn load() -> Self {
        let Some(p) = Self::path() else { return Self::default() };
        match std::fs::read_to_string(&p) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!("config.toml parse failed ({e}), using defaults");
                Self::default()
            }),
            Err(_) => Self::default(), // 不存在 = 首次运行，用默认
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let p = Self::path().ok_or_else(|| anyhow::anyhow!("no config dir"))?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&p, toml::to_string_pretty(self)?)?;
        tracing::info!("config saved to {}", p.display());
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EngineState {
    Idle,
    Recording,
    Transcribing,
    Polishing,
    Pasting,
    Error,
}

impl EngineState {
    pub fn label(self) -> &'static str {
        match self {
            EngineState::Idle => "空闲",
            EngineState::Recording => "录音中",
            EngineState::Transcribing => "转写中",
            EngineState::Polishing => "润色中",
            EngineState::Pasting => "粘贴中",
            EngineState::Error => "出错",
        }
    }
    /// 托盘小圆点的 RGB
    pub fn color(self) -> [u8; 3] {
        match self {
            EngineState::Idle => [142, 142, 147],                                  // 灰
            EngineState::Recording => [0, 122, 255],                               // 蓝（按住说话）
            EngineState::Transcribing | EngineState::Polishing => [255, 159, 10],  // 橙（处理中）
            EngineState::Pasting => [52, 199, 89],                                 // 绿
            EngineState::Error => [255, 59, 48],                                   // 红（出故障）
        }
    }
}

pub struct Status {
    pub state: EngineState,
    pub last_raw: String,
    pub last_final: String,
    pub last_error: String,
    /// 最近若干条最终文本，新的在前
    pub history: Vec<String>,
    /// 上次录音峰值，给 UI 当电平参考
    pub last_peak: i16,
    /// rdev/权限异常时置位，UI 弹权限提示
    pub permission_problem: bool,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            state: EngineState::Idle,
            last_raw: String::new(),
            last_final: String::new(),
            last_error: String::new(),
            history: Vec::new(),
            last_peak: 0,
            permission_problem: false,
        }
    }
}

/// 引擎线程和 UI 线程共享的句柄。clone 廉价（只 clone Arc）。
#[derive(Clone)]
pub struct Shared {
    pub config: Arc<Mutex<AppConfig>>,
    pub status: Arc<Mutex<Status>>,
}

impl Shared {
    pub fn new(config: AppConfig) -> Self {
        Self {
            config: Arc::new(Mutex::new(config)),
            status: Arc::new(Mutex::new(Status::default())),
        }
    }

    /// 锁中毒也强行恢复——单次会话的脏数据不值得拖垮整个 app。
    pub fn config_snapshot(&self) -> AppConfig {
        self.config.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn with_status<R>(&self, f: impl FnOnce(&mut Status) -> R) -> R {
        let mut s = self.status.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut s)
    }

    pub fn set_state(&self, state: EngineState) {
        self.with_status(|s| s.state = state);
    }
}
