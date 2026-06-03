use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct Recording {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
}

pub fn record(duration: Duration) -> Result<Recording> {
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
        "input stream configured"
    );

    let buffer: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::with_capacity(
        sample_rate as usize * duration.as_secs() as usize,
    )));
    let buffer_cb = buffer.clone();

    let stream_config: cpal::StreamConfig = config.clone().into();
    let err_fn = |err| tracing::error!("audio stream error: {err}");

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |data: &[f32], _: &_| append_mono_f32(&buffer_cb, data, channels),
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_input_stream(
            &stream_config,
            move |data: &[i16], _: &_| append_mono_i16(&buffer_cb, data, channels),
            err_fn,
            None,
        )?,
        cpal::SampleFormat::U16 => device.build_input_stream(
            &stream_config,
            move |data: &[u16], _: &_| append_mono_u16(&buffer_cb, data, channels),
            err_fn,
            None,
        )?,
        other => anyhow::bail!("unsupported sample format: {other:?}"),
    };

    stream.play().context("failed to start input stream")?;
    std::thread::sleep(duration);
    drop(stream);

    let samples = Arc::try_unwrap(buffer)
        .map_err(|_| anyhow::anyhow!("buffer still shared after stream drop"))?
        .into_inner()
        .map_err(|e| anyhow::anyhow!("buffer mutex poisoned: {e}"))?;

    Ok(Recording {
        samples,
        sample_rate,
    })
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
