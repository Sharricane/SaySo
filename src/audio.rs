use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub struct Recording {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
}

impl Recording {
    pub fn peak_amplitude(&self) -> i16 {
        self.samples
            .iter()
            .map(|s| s.saturating_abs())
            .max()
            .unwrap_or(0)
    }

    /// 经验阈值：500/32768 ≈ -36 dBFS。低于此值视为"没人说话"，
    /// 避免触发 Whisper 在纯静音上的幻觉（典型表现：返回日文短句、订阅广告语）。
    /// Phase 2 设置面板会让用户根据自己环境噪音上下调。
    pub fn is_silent(&self) -> bool {
        self.peak_amplitude() < 500
    }

    pub fn duration_secs(&self) -> f32 {
        self.samples.len() as f32 / self.sample_rate as f32
    }
}

/// 常驻音频引擎：cpal 流在 app 启动时建好并保持播放，
/// 默认不写入缓冲；按下快捷键时翻转 `collecting` 原子布尔后才开始累积样本。
///
/// 目的：消除每次按下快捷键时 build_input_stream + WASAPI 激活带来的
/// 100-300ms 录音盲区——用户说出的第一个字常常因此被吞掉。
pub struct AudioEngine {
    sample_rate: u32,
    buffer: Arc<Mutex<Vec<i16>>>,
    collecting: Arc<AtomicBool>,
    _stream: cpal::Stream,
}

impl AudioEngine {
    pub fn start() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("no default input device — check Windows mic permissions")?;

        let config = device
            .default_input_config()
            .context("could not query default input config")?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels() as usize;
        let sample_format = config.sample_format();

        tracing::info!(
            device = device.name().unwrap_or_else(|_| "unknown".into()),
            sample_rate,
            channels,
            format = ?sample_format,
            "warming audio engine"
        );

        let buffer: Arc<Mutex<Vec<i16>>> =
            Arc::new(Mutex::new(Vec::with_capacity(sample_rate as usize * 4)));
        let collecting = Arc::new(AtomicBool::new(false));

        let buf_cb = buffer.clone();
        let coll_cb = collecting.clone();
        let stream_config: cpal::StreamConfig = config.clone().into();
        let err_fn = |err| tracing::error!("audio stream error: {err}");

        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &stream_config,
                move |data: &[f32], _: &_| {
                    if !coll_cb.load(Ordering::Relaxed) {
                        return;
                    }
                    append_mono_f32(&buf_cb, data, channels);
                },
                err_fn,
                None,
            )?,
            cpal::SampleFormat::I16 => device.build_input_stream(
                &stream_config,
                move |data: &[i16], _: &_| {
                    if !coll_cb.load(Ordering::Relaxed) {
                        return;
                    }
                    append_mono_i16(&buf_cb, data, channels);
                },
                err_fn,
                None,
            )?,
            cpal::SampleFormat::U16 => device.build_input_stream(
                &stream_config,
                move |data: &[u16], _: &_| {
                    if !coll_cb.load(Ordering::Relaxed) {
                        return;
                    }
                    append_mono_u16(&buf_cb, data, channels);
                },
                err_fn,
                None,
            )?,
            other => anyhow::bail!("unsupported sample format: {other:?}"),
        };

        stream.play().context("failed to start input stream")?;

        Ok(AudioEngine {
            sample_rate,
            buffer,
            collecting,
            _stream: stream,
        })
    }

    pub fn begin_session(&self) {
        self.buffer.lock().expect("buffer poisoned").clear();
        // 严格顺序：先 clear 再 set collecting=true，确保新会话不会带上一次的残尾
        self.collecting.store(true, Ordering::Release);
    }

    pub fn end_session(&self) -> Recording {
        self.collecting.store(false, Ordering::Release);
        // 给音频回调几毫秒落地最后一批样本
        std::thread::sleep(std::time::Duration::from_millis(20));
        let samples = std::mem::take(&mut *self.buffer.lock().expect("buffer poisoned"));
        Recording {
            samples,
            sample_rate: self.sample_rate,
        }
    }
}

fn append_mono_f32(buf: &Mutex<Vec<i16>>, data: &[f32], channels: usize) {
    let mut b = buf.lock().expect("buffer poisoned");
    for chunk in data.chunks(channels) {
        let avg: f32 = chunk.iter().sum::<f32>() / channels as f32;
        let clamped = (avg * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32);
        b.push(clamped as i16);
    }
}

fn append_mono_i16(buf: &Mutex<Vec<i16>>, data: &[i16], channels: usize) {
    let mut b = buf.lock().expect("buffer poisoned");
    for chunk in data.chunks(channels) {
        let avg: i32 = chunk.iter().map(|&s| s as i32).sum::<i32>() / channels as i32;
        b.push(avg as i16);
    }
}

fn append_mono_u16(buf: &Mutex<Vec<i16>>, data: &[u16], channels: usize) {
    let mut b = buf.lock().expect("buffer poisoned");
    for chunk in data.chunks(channels) {
        let avg: i32 = chunk.iter().map(|&s| s as i32 - 32768).sum::<i32>() / channels as i32;
        b.push(avg as i16);
    }
}
