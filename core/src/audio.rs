/// Zielrate für alle Anbieter.
pub const SAMPLE_RATE: u32 = 16_000;

/// Verpackt 16-Bit-PCM-Samples (mono) in einen WAV-Container.
pub fn encode_wav(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_size = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + samples.len() * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_size).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // Größe des fmt-Blocks
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // Bytes pro Sekunde
    wav.extend_from_slice(&2u16.to_le_bytes()); // Blockgröße
    wav.extend_from_slice(&16u16.to_le_bytes()); // Bits pro Sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    wav.extend(samples.iter().flat_map(|sample| sample.to_le_bytes()));
    wav
}

/// Samples als Little-Endian-Bytes (Format des Deepgram-Live-Streams).
pub fn pcm_bytes(samples: &[i16]) -> Vec<u8> {
    samples.iter().flat_map(|sample| sample.to_le_bytes()).collect()
}

/// Wandelt einen Geräte-Stream (beliebige Rate und Kanäle) laufend in 16 kHz mono.
/// Mittelt alle Eingangswerte eines Ausgabeschritts; das dämpft auch Aliasing beim Herunterrechnen.
#[derive(Debug)]
pub struct Resampler {
    channels: usize,
    step: f64,
    position: f64,
    sum: f64,
    count: u32,
    last: f32,
}

impl Resampler {
    pub fn new(input_rate: u32, channels: usize) -> Self {
        Self {
            channels: channels.max(1),
            step: f64::from(input_rate) / f64::from(SAMPLE_RATE),
            position: 0.0,
            sum: 0.0,
            count: 0,
            last: 0.0,
        }
    }

    /// Nimmt verschachtelte Float-Samples (-1..1) und liefert die fertigen 16-Bit-Samples.
    pub fn process(&mut self, interleaved: &[f32]) -> Vec<i16> {
        let mut output = Vec::with_capacity((interleaved.len() as f64 / self.step) as usize + 2);
        for frame in interleaved.chunks_exact(self.channels) {
            let mono = frame.iter().sum::<f32>() / self.channels as f32;
            self.sum += f64::from(mono);
            self.count += 1;
            self.position += 1.0;
            while self.position >= self.step {
                if self.count > 0 {
                    self.last = (self.sum / f64::from(self.count)) as f32;
                    self.sum = 0.0;
                    self.count = 0;
                }
                output.push((self.last.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16);
                self.position -= self.step;
            }
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32_at(wav: &[u8], offset: usize) -> Option<u32> {
        Some(u32::from_le_bytes(wav.get(offset..offset + 4)?.try_into().ok()?))
    }

    #[test]
    fn wav_header_and_size() {
        let wav = encode_wav(&[1, -2, 3], 16_000);
        assert_eq!(wav.len(), 44 + 6);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(u32_at(&wav, 4), Some(36 + 6));
        assert_eq!(u32_at(&wav, 24), Some(16_000));
        assert_eq!(i16::from_le_bytes([wav[44], wav[45]]), 1);
        assert_eq!(i16::from_le_bytes([wav[46], wav[47]]), -2);
    }

    #[test]
    fn resampler_keeps_duration_across_chunks() {
        let mut resampler = Resampler::new(48_000, 2);
        let chunk = vec![0.0f32; 4_800 * 2];
        let total: usize = (0..10).map(|_| resampler.process(&chunk).len()).sum();
        assert_eq!(total, 16_000);
    }

    #[test]
    fn resampler_handles_odd_rates_and_upsampling() {
        let total = Resampler::new(44_100, 1).process(&vec![0.0; 44_100]).len();
        assert!((15_999..=16_001).contains(&total), "{total}");
        let total = Resampler::new(8_000, 1).process(&vec![0.5; 8_000]).len();
        assert_eq!(total, 16_000);
    }

    #[test]
    fn resampler_averages_channels_and_scales() {
        let samples = Resampler::new(16_000, 2).process(&[1.0, 0.0, -1.0, -1.0]);
        assert_eq!(samples, vec![16_383, -32_767]);
    }
}
