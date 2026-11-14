//! Audio in (XC-37): a [`Source`] delivers mono `f32` samples at its own
//! rate while the dictation key is held; [`Microphone`] is the default
//! input device through cpal (CoreAudio, WASAPI, ALSA), opened on press and
//! closed on release, so nothing listens otherwise. A test feeds a WAV file
//! through the same path ([`Samples`]). [`to_16k`] resamples a clip for
//! whisper.cpp; [`level`] is the pill's meter.

use super::SpeechError;

/// Receives captured samples (mono, at the source's rate).
pub type SampleSink = Box<dyn FnMut(&[f32]) + Send>;

/// Where dictation audio comes from.
pub trait Source: Send {
    /// Start delivering samples to `sink`; the sample rate on success.
    fn start(&mut self, sink: SampleSink) -> Result<u32, SpeechError>;
    /// Stop and release the device; no samples arrive after it returns.
    fn stop(&mut self);
}

/// A clip already in memory (tests, the WAV fixture): delivered at once.
pub struct Samples {
    pub rate: u32,
    pub samples: Vec<f32>,
}

impl Source for Samples {
    fn start(&mut self, mut sink: SampleSink) -> Result<u32, SpeechError> {
        for chunk in self.samples.chunks(1024) {
            sink(chunk);
        }
        Ok(self.rate)
    }

    fn stop(&mut self) {}
}

/// The meter: the RMS of `samples` on a -60..0 dB scale, as 0..1.
pub fn level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    if rms <= 0.0 {
        return 0.0;
    }
    ((20.0 * rms.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

/// Read a 16-bit PCM WAV (mono or interleaved channels, averaged) into
/// `f32` samples and its rate. Enough for the fixtures; not a general reader.
pub fn read_wav(bytes: &[u8]) -> Result<Samples, SpeechError> {
    let bad = |what: &str| SpeechError::Io(format!("not a 16-bit PCM WAV: {what}"));
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad("no RIFF/WAVE header"));
    }
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at =
        |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let (mut channels, mut rate, mut bits) = (0u16, 0u32, 0u16);
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let len = u32_at(i + 4) as usize;
        let body = i + 8;
        let end = (body + len).min(bytes.len());
        if id == b"fmt " && len >= 16 {
            if u16_at(body) != 1 {
                return Err(bad("compressed"));
            }
            channels = u16_at(body + 2);
            rate = u32_at(body + 4);
            bits = u16_at(body + 14);
        } else if id == b"data" {
            if bits != 16 || channels == 0 {
                return Err(bad("not 16-bit"));
            }
            let frame = 2 * channels as usize;
            let samples = bytes[body..end]
                .chunks_exact(frame)
                .map(|f| {
                    let sum: f32 = f
                        .chunks_exact(2)
                        .map(|s| i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0)
                        .sum();
                    sum / channels as f32
                })
                .collect();
            return Ok(Samples { rate, samples });
        }
        i = body + len + (len & 1);
    }
    Err(bad("no data chunk"))
}

/// Resample a mono clip to 16 kHz (what whisper.cpp takes).
#[cfg(feature = "speech")]
pub fn to_16k(samples: &[f32], rate: u32) -> Result<Vec<f32>, SpeechError> {
    use rubato::audioadapter_buffers::direct::InterleavedSlice;
    use rubato::{Fft, FixedSync, Resampler};
    if rate == super::SAMPLE_RATE || samples.is_empty() {
        return Ok(samples.to_vec());
    }
    let engine = |e: String| SpeechError::Engine(format!("resampling failed: {e}"));
    let mut r = Fft::<f32>::new(
        rate as usize,
        super::SAMPLE_RATE as usize,
        1024,
        1,
        FixedSync::Input,
    )
    .map_err(|e| engine(e.to_string()))?;
    let input =
        InterleavedSlice::new(samples, 1, samples.len()).map_err(|e| engine(e.to_string()))?;
    let out = r
        .process_all(&input, samples.len(), None)
        .map_err(|e| engine(e.to_string()))?;
    Ok(out.take_data())
}

/// Without the speech engine only 16 kHz clips pass (the tests' fakes).
#[cfg(not(feature = "speech"))]
pub fn to_16k(samples: &[f32], rate: u32) -> Result<Vec<f32>, SpeechError> {
    if rate == super::SAMPLE_RATE {
        Ok(samples.to_vec())
    } else {
        Err(SpeechError::Unavailable)
    }
}

#[cfg(feature = "speech")]
pub use mic::Microphone;

#[cfg(feature = "speech")]
mod mic {
    use std::sync::mpsc;
    use std::thread::JoinHandle;

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, SampleFormat, SizedSample};

    use super::{SampleSink, Source};
    use crate::speech::SpeechError;

    /// The default input device, owned by its own thread for the length of
    /// one hold (a cpal stream is not `Send` on every platform).
    #[derive(Default)]
    pub struct Microphone {
        stop: Option<mpsc::Sender<()>>,
        thread: Option<JoinHandle<()>>,
    }

    impl Microphone {
        pub fn new() -> Microphone {
            Microphone::default()
        }
    }

    fn device_error(e: cpal::Error) -> SpeechError {
        match e.kind() {
            cpal::ErrorKind::PermissionDenied => SpeechError::MicDenied,
            cpal::ErrorKind::DeviceNotAvailable => SpeechError::NoMicrophone,
            _ => SpeechError::Device(e.to_string()),
        }
    }

    fn build<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        mut sink: SampleSink,
    ) -> Result<cpal::Stream, cpal::Error>
    where
        T: SizedSample,
        f32: FromSample<T>,
    {
        let channels = config.channels.max(1) as usize;
        let mut mono: Vec<f32> = Vec::with_capacity(4096);
        device.build_input_stream::<T, _, _>(
            *config,
            move |data: &[T], _| {
                mono.clear();
                mono.extend(data.chunks(channels).map(|frame| {
                    frame.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32
                }));
                sink(&mono);
            },
            |e| tracing::warn!(error = %e, "microphone stream error"),
            None,
        )
    }

    fn open(sink: SampleSink) -> Result<(cpal::Stream, u32), SpeechError> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or(SpeechError::NoMicrophone)?;
        let supported = device.default_input_config().map_err(device_error)?;
        let config: cpal::StreamConfig = supported.config();
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, &config, sink),
            SampleFormat::I16 => build::<i16>(&device, &config, sink),
            SampleFormat::U16 => build::<u16>(&device, &config, sink),
            SampleFormat::I32 => build::<i32>(&device, &config, sink),
            SampleFormat::I8 => build::<i8>(&device, &config, sink),
            other => {
                return Err(SpeechError::Device(format!(
                    "the microphone's sample format {other} is not supported"
                )));
            }
        }
        .map_err(device_error)?;
        stream.play().map_err(device_error)?;
        Ok((stream, config.sample_rate))
    }

    impl Source for Microphone {
        fn start(&mut self, sink: SampleSink) -> Result<u32, SpeechError> {
            self.stop();
            let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<u32, SpeechError>>(1);
            let (stop_tx, stop_rx) = mpsc::channel::<()>();
            let thread = std::thread::Builder::new()
                .name("mode-mic".into())
                .spawn(move || match open(sink) {
                    Ok((stream, rate)) => {
                        let _ = ready_tx.send(Ok(rate));
                        let _ = stop_rx.recv();
                        drop(stream);
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                })
                .map_err(|e| SpeechError::Device(e.to_string()))?;
            let rate = ready_rx
                .recv()
                .map_err(|_| SpeechError::Device("the microphone thread stopped".into()))??;
            self.stop = Some(stop_tx);
            self.thread = Some(thread);
            Ok(rate)
        }

        fn stop(&mut self) {
            if let Some(tx) = self.stop.take() {
                let _ = tx.send(());
            }
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    impl Drop for Microphone {
        fn drop(&mut self) {
            self.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_meter_maps_rms_to_a_decibel_scale() {
        assert_eq!(level(&[]), 0.0);
        assert_eq!(level(&[0.0; 100]), 0.0);
        assert!((level(&[1.0; 100]) - 1.0).abs() < 1e-6);
        let quiet = level(&[0.01; 100]);
        assert!((quiet - (1.0 / 3.0)).abs() < 1e-3, "{quiet}");
    }

    #[test]
    fn wav_headers_are_checked() {
        assert!(read_wav(b"nope").is_err());
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF\0\0\0\0WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&8000u32.to_le_bytes());
        wav.extend_from_slice(&32000u32.to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&8u32.to_le_bytes());
        for s in [16384i16, 0, -16384, -16384] {
            wav.extend_from_slice(&s.to_le_bytes());
        }
        let s = read_wav(&wav).unwrap();
        assert_eq!(s.rate, 8000);
        assert_eq!(s.samples, vec![0.25, -0.5]);
    }
}
