use anyhow::{Context, Result};
use hound::{SampleFormat, WavSpec, WavWriter};
use std::io::Cursor;

pub fn pcm_to_wav(samples: &[i16], sample_rate: u32) -> Result<Vec<u8>> {
    let spec = WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };

    let mut cursor = Cursor::new(Vec::<u8>::new());
    {
        let mut writer = WavWriter::new(&mut cursor, spec).context("WavWriter init")?;
        for &s in samples {
            writer.write_sample(s).context("write sample")?;
        }
        writer.finalize().context("WavWriter finalize")?;
    }
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produces_valid_riff_header() {
        let wav = pcm_to_wav(&[0i16; 1600], 16000).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }
}
