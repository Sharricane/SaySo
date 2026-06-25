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

    /// 均方根（持续能量）。比 peak 更能区分"真的有人在说话"和"安静房间里偶发的
    /// 键盘声/关门声"——后者 peak 很高但 RMS 很低。嘈杂环境用这个做门限更稳。
    pub fn rms_amplitude(&self) -> i16 {
        if self.samples.is_empty() {
            return 0;
        }
        let sum_sq: f64 = self.samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
        (sum_sq / self.samples.len() as f64).sqrt() as i16
    }

    /// 低于此 peak amplitude 阈值视为"没人说话"，跳过 Whisper 避免幻觉。
    /// 500/32768 ≈ -36 dBFS 是安静办公室的经验值；嘈杂环境用户应调高
    /// （比如 800-1500），极安静环境调低（200-300）。
    pub fn is_silent(&self, threshold: i16) -> bool {
        self.peak_amplitude() < threshold
    }

    pub fn duration_secs(&self) -> f32 {
        self.samples.len() as f32 / self.sample_rate as f32
    }
}

/// 音频引擎：cpal 流启动时建好（建一次避免每次的激活盲区），但**空闲时 pause、
/// 录音时才 play**。这样 macOS 那个橙色麦克风指示灯只在录音时亮——既不再常驻占用
/// 麦克风，又天然成了"正在录"的反馈（系统级、不会被刘海挤没）。play 是在已建好的
/// 流上重启，几乎没延迟，不会吞字。
pub struct AudioEngine {
    sample_rate: u32,
    buffer: Arc<Mutex<Vec<i16>>>,
    collecting: Arc<AtomicBool>,
    stream: cpal::Stream,
}

impl AudioEngine {
    pub fn start() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("no default input device — check microphone permissions/availability")?;

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

        // 先 play 再 pause：预热一次（做掉首次激活），之后空闲保持 pause（麦克风灯灭）。
        stream.play().context("failed to start input stream")?;
        let _ = stream.pause();

        Ok(AudioEngine {
            sample_rate,
            buffer,
            collecting,
            stream,
        })
    }

    pub fn begin_session(&self) {
        // 即使 mutex 被毒化（其他线程持锁时 panic）也强行往前——清掉数据 + 解除毒化。
        let mut b = self.buffer.lock().unwrap_or_else(|e| {
            tracing::warn!("audio buffer mutex was poisoned, recovering");
            e.into_inner()
        });
        b.clear();
        drop(b);
        self.buffer.clear_poison();
        let _ = self.stream.play(); // 麦克风开（系统橙色指示灯亮=正在录）
        self.collecting.store(true, Ordering::Release);
    }

    pub fn end_session(&self) -> Recording {
        self.collecting.store(false, Ordering::Release);
        // 给音频回调几毫秒落地最后一批样本
        std::thread::sleep(std::time::Duration::from_millis(20));
        let mut b = self.buffer.lock().unwrap_or_else(|e| {
            tracing::warn!("audio buffer mutex was poisoned on session end");
            e.into_inner()
        });
        let samples = std::mem::take(&mut *b);
        drop(b);
        self.buffer.clear_poison();
        let _ = self.stream.pause(); // 麦克风关（指示灯灭）
        Recording {
            samples,
            sample_rate: self.sample_rate,
        }
    }
}

// cpal 的回调跑在它内部线程上。如果回调里 panic（譬如样本除零、buf push OOM），
// 整个进程会 abort。这里用 try_lock + 错误降级 + 通道数兜底，保证回调永不 panic。
fn append_mono_f32(buf: &Mutex<Vec<i16>>, data: &[f32], channels: usize) {
    let Ok(mut b) = buf.lock() else { return };
    let ch = channels.max(1);
    for chunk in data.chunks(ch) {
        let avg: f32 = chunk.iter().sum::<f32>() / ch as f32;
        let clamped = (avg * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32);
        b.push(clamped as i16);
    }
}

fn append_mono_i16(buf: &Mutex<Vec<i16>>, data: &[i16], channels: usize) {
    let Ok(mut b) = buf.lock() else { return };
    let ch = channels.max(1);
    for chunk in data.chunks(ch) {
        let avg: i32 = chunk.iter().map(|&s| s as i32).sum::<i32>() / ch as i32;
        b.push(avg as i16);
    }
}

fn append_mono_u16(buf: &Mutex<Vec<i16>>, data: &[u16], channels: usize) {
    let Ok(mut b) = buf.lock() else { return };
    let ch = channels.max(1);
    for chunk in data.chunks(ch) {
        let avg: i32 = chunk.iter().map(|&s| s as i32 - 32768).sum::<i32>() / ch as i32;
        b.push(avg as i16);
    }
}
