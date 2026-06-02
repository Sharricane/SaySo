# SaySo

按住说话，松开就把转写好的文字粘贴到光标位置。给开发场景用的语音口述工具——尤其适合在终端里用 Claude Code 编程时直接说出需求。

## 核心思路

```
按下快捷键 → 录音
松开快捷键 → Whisper 转写 → LLM 润色 → 写入剪贴板 → 模拟粘贴
```

不做复杂的"模拟打字"，统一走"剪贴板 + 粘贴"。任何接受 Ctrl+V / Cmd+V 的输入框都能用——终端、浏览器、IDE 通吃。

## 目标平台

- [x] Windows 10/11（首要目标，覆盖 WSL2 用户）
- [ ] macOS 13+（Phase 3）
- [ ] Linux 原生（不做。WSL2 用户用 Windows 版即可）

## 主要使用场景

1. 在 Windows Terminal / iTerm2 里用 Claude Code 时口述编程需求
2. 在 claude.ai 等浏览器页面里口述输入

## 技术栈

| 模块 | 选型 |
|---|---|
| 桌面框架 | Tauri 2.x |
| 后端语言 | Rust |
| 前端 | React + Vite + TypeScript + Tailwind |
| 音频采集 | `cpal` |
| 全局快捷键 | `tauri-plugin-global-shortcut` |
| 剪贴板 | `tauri-plugin-clipboard-manager` |
| 模拟粘贴 | `enigo` |
| STT | OpenAI Whisper API（或 Groq 兼容接口） |
| LLM 润色 | Claude Haiku 4.5（或 OpenAI GPT-4o-mini） |

## 参考项目

- [Whisperi](https://github.com/xarthurx/whisperi) — Tauri + 云端 API，架构最贴近
- [local-dictation-app](https://github.com/fiorelorenzo/local-dictation-app) — Tauri + Svelte 项目结构参考
- [VoiceTypr](https://github.com/moinulmoin/voicetypr) — Wispr Flow 风格 UI 参考
- [Awesome-Whisper-Apps](https://github.com/danielrosehill/Awesome-Whisper-Apps) — 同类工具横向对比

## 开发计划

按 [`CLAUDE.md`](./CLAUDE.md) 中定义的 Phase 推进。当前：Phase 1（Windows MVP）。
