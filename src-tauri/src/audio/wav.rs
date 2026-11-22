//! 16-bit PCM mono WAV encoding and decoding for the ASR boundary.
//!
//! Encoding to memory lets the persistent ASR sidecar receive audio over loopback HTTP without
//! a temporary file ever touching disk. The one-shot CLI fallback still needs a file, so a
//! path-based writer is provided too. Decoding is used by the benchmark harness.

use std::{io::Cursor, path::Path};

use super::resample::ASR_SAMPLE_RATE;

fn asr_wav_spec() -> hound::WavSpec {
    hound::WavSpec {
        channels: 1,
        sample_rate: ASR_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    }
}

fn to_pcm16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

/// Encode mono 16 kHz samples as an in-memory 16-bit PCM WAV file.
pub fn encode_wav_16k_mono(samples: &[f32]) -> Result<Vec<u8>, String> {
    let mut buffer = Cursor::new(Vec::with_capacity(44 + samples.len() * 2));
    {
        let mut writer = hound::WavWriter::new(&mut buffer, asr_wav_spec())
            .map_err(|error| format!("WAV encode failed: {error}"))?;
        for sample in samples {
            writer
                .write_sample(to_pcm16(*sample))
                .map_err(|error| format!("WAV sample encode failed: {error}"))?;
        }
        writer
            .finalize()
            .map_err(|error| format!("WAV finalize failed: {error}"))?;
    }
    Ok(buffer.into_inner())
}

/// Encode mono samples at `sample_rate` as an in-memory 16-bit PCM WAV file.
pub fn encode_wav_mono(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, String> {
    let spec = hound::WavSpec {
        sample_rate,
        ..asr_wav_spec()
    };
    let mut buffer = Cursor::new(Vec::with_capacity(44 + samples.len() * 2));
    {
        let mut writer = hound::WavWriter::new(&mut buffer, spec)
            .map_err(|error| format!("WAV encode failed: {error}"))?;
        for sample in samples {
            writer
                .write_sample(to_pcm16(*sample))
                .map_err(|error| format!("WAV sample encode failed: {error}"))?;
        }
        writer
            .finalize()
            .map_err(|error| format!("WAV finalize failed: {error}"))?;
    }
    Ok(buffer.into_inner())
}

/// Write mono 16 kHz samples to `path` as a 16-bit PCM WAV file.
pub fn write_wav_16k_mono(path: &Path, samples: &[f32]) -> Result<(), String> {
    let bytes = encode_wav_16k_mono(samples)?;
    std::fs::write(path, bytes).map_err(|error| format!("WAV write failed: {error}"))
}

/// Decoded audio: mono samples in `[-1, 1]` plus the file's sample rate.
#[derive(Debug, Clone)]
pub struct DecodedWav {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

/// Read a WAV file, averaging channels to mono. Supports integer and float PCM.
pub fn read_wav_mono(path: &Path) -> Result<DecodedWav, String> {
    let reader = hound::WavReader::open(path)
        .map_err(|error| format!("Could not open WAV {}: {error}", path.display()))?;
    decode_reader(reader)
}

/// Decode an in-memory WAV file, averaging channels to mono. Error messages describe the
/// format problem only and never include the bytes.
pub fn decode_wav_mono(bytes: &[u8]) -> Result<DecodedWav, String> {
    let reader = hound::WavReader::new(Cursor::new(bytes))
        .map_err(|error| format!("not a readable WAV file: {error}"))?;
    decode_reader(reader)
}

fn decode_reader<R: std::io::Read>(mut reader: hound::WavReader<R>) -> Result<DecodedWav, String> {
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;

    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|error| format!("Could not decode WAV samples: {error}"))?,
        hound::SampleFormat::Int => {
            let scale = (1_i64 << (spec.bits_per_sample.max(1) - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|sample| sample.map(|value| value as f32 / scale))
                .collect::<Result<_, _>>()
                .map_err(|error| format!("Could not decode WAV samples: {error}"))?
        }
    };

    let samples = if channels == 1 {
        interleaved
    } else {
        interleaved
            .chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
            .collect()
    };

    Ok(DecodedWav {
        samples,
        sample_rate: spec.sample_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_a_valid_16k_mono_pcm16_header() {
        let bytes = encode_wav_16k_mono(&[0.0, 0.5, -0.5, 1.0]).unwrap();

        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        // 44-byte canonical header plus two bytes per sample.
        assert_eq!(bytes.len(), 44 + 4 * 2);

        let reader = hound::WavReader::new(Cursor::new(bytes)).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.bits_per_sample, 16);
    }

    #[test]
    fn in_memory_round_trip_keeps_rate_and_samples_and_downmixes_stereo() {
        let encoded = encode_wav_mono(&[0.0, 0.5, -0.5], 44_100).unwrap();
        let decoded = decode_wav_mono(&encoded).unwrap();
        assert_eq!(decoded.sample_rate, 44_100);
        assert_eq!(decoded.samples.len(), 3);
        assert!((decoded.samples[1] - 0.5).abs() < 1e-3);

        // Stereo input is averaged to mono.
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 24_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut buffer, spec).unwrap();
            for value in [16_000_i16, 0, -16_000, 0] {
                writer.write_sample(value).unwrap();
            }
            writer.finalize().unwrap();
        }
        let stereo = decode_wav_mono(&buffer.into_inner()).unwrap();
        assert_eq!(stereo.samples.len(), 2);
        assert!((stereo.samples[0] - 0.244).abs() < 0.01);
    }

    #[test]
    fn junk_bytes_are_rejected_without_echoing_them() {
        let error = decode_wav_mono(b"definitely not audio").unwrap_err();
        assert!(error.starts_with("not a readable WAV file"));
        assert!(!error.contains("definitely"));
    }

    #[test]
    fn clamps_out_of_range_samples_instead_of_wrapping() {
        let bytes = encode_wav_16k_mono(&[2.0, -2.0]).unwrap();
        let samples: Vec<i16> = hound::WavReader::new(Cursor::new(bytes))
            .unwrap()
            .samples::<i16>()
            .map(Result::unwrap)
            .collect();

        assert_eq!(samples, vec![i16::MAX, -i16::MAX]);
    }

    #[test]
    fn file_round_trip_preserves_samples_within_quantisation_error() {
        let path = std::env::temp_dir().join(format!(
            "localflow-wav-test-{}-{:?}.wav",
            std::process::id(),
            std::thread::current().id()
        ));
        let input = vec![0.0_f32, 0.25, -0.25, 0.75];

        write_wav_16k_mono(&path, &input).unwrap();
        let decoded = read_wav_mono(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(decoded.sample_rate, 16_000);
        assert_eq!(decoded.samples.len(), input.len());
        for (expected, actual) in input.iter().zip(decoded.samples.iter()) {
            assert!((expected - actual).abs() < 1.0 / 16_000.0);
        }
    }
}
