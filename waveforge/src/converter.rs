use std::{fs::File, path::Path};

use symphonia::core::{
    audio::SampleBuffer,
    codecs::{DecoderOptions, CODEC_TYPE_NULL},
    errors::Error,
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};
pub type BoxErr = Box<dyn std::error::Error + Send + Sync>;
pub const MAX_SECONDS: u64 = 3 * 3600;

#[derive(Debug, PartialEq)]
pub struct Stats {
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
}

/// Decode the first audio track of `input` (m4a/mp4/wav/...) into a 16-bit PCM WAV at `output`.
pub fn convert(input: &Path, output: &Path) -> Result<Stats, BoxErr> {
    let mss = MediaSourceStream::new(Box::new(File::open(input)?), Default::default());
    let probed = symphonia::default::get_probe().format(
        &Hint::new(),
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL && t.codec_params.sample_rate.is_some())
        .ok_or("no audio track")?;
    let track_id = track.id;
    let params = track.codec_params.clone();
    let mut decoder = symphonia::default::get_codecs().make(&params, &DecoderOptions::default())?;

    // Format is taken from the first decoded frame; containers don't always know it up front.
    let mut writer: Option<hound::WavWriter<std::io::BufWriter<File>>> = None;
    let mut sbuf: Option<SampleBuffer<i16>> = None;
    let (mut sample_rate, mut channels, mut frames) = (0u32, 0u16, 0u64);

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                if writer.is_none() {
                    let spec = *decoded.spec();
                    sample_rate = spec.rate;
                    channels = spec.channels.count() as u16;
                    writer = Some(hound::WavWriter::create(
                        output,
                        hound::WavSpec {
                            channels,
                            sample_rate,
                            bits_per_sample: 16,
                            sample_format: hound::SampleFormat::Int,
                        },
                    )?);
                    sbuf = Some(SampleBuffer::<i16>::new(decoded.capacity() as u64, spec));
                }
                let buf = sbuf.as_mut().unwrap();
                buf.copy_interleaved_ref(decoded);
                let w = writer.as_mut().unwrap();
                for s in buf.samples() {
                    w.write_sample(*s)?;
                }
                frames += (buf.len() / channels as usize) as u64;
                if frames > MAX_SECONDS * sample_rate as u64 {
                    return Err("audio too long".into());
                }
            }
            Err(Error::DecodeError(_)) => continue, // skip corrupt frames
            Err(e) => return Err(e.into()),
        }
    }
    writer.ok_or("no decodable audio frames")?.finalize()?;
    Ok(Stats {
        sample_rate,
        channels,
        frames,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_sine_wav(path: &Path, rate: u32, channels: u16, frames: u32) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..frames {
            let v =
                ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 10_000.0) as i16;
            for _ in 0..channels {
                w.write_sample(v).unwrap();
            }
        }
        w.finalize().unwrap();
    }

    #[test]
    fn wav_roundtrip_preserves_format_and_length() {
        let dir = tempfile::tempdir().unwrap();
        let (i, o) = (dir.path().join("in.wav"), dir.path().join("out.wav"));
        write_sine_wav(&i, 44_100, 2, 44_100);

        let stats = convert(&i, &o).unwrap();
        assert_eq!(
            stats,
            Stats {
                sample_rate: 44_100,
                channels: 2,
                frames: 44_100
            }
        );

        let r = hound::WavReader::open(&o).unwrap();
        assert_eq!(r.spec().channels, 2);
        assert_eq!(r.spec().bits_per_sample, 16);
        assert_eq!(r.len(), 44_100 * 2); // total samples across channels
    }

    #[test]
    fn garbage_input_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let i = dir.path().join("junk.m4a");
        std::fs::write(&i, vec![0x42u8; 4096]).unwrap();
        assert!(convert(&i, &dir.path().join("out.wav")).is_err());
    }

    #[test]
    fn missing_input_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(convert(&dir.path().join("nope.m4a"), &dir.path().join("o.wav")).is_err());
    }
}
