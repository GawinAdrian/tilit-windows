//! Kurze eigene Signaltöne, im Speicher erzeugt: keine fremden Klänge, keine Dateien.

use std::f32::consts::TAU;
use std::sync::OnceLock;
use tilit_core::audio::encode_wav;
use tilit_core::session::Sound;
use windows::core::PCWSTR;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

const RATE: u32 = 16_000;

fn tone(frequency: f32, milliseconds: u32) -> Vec<u8> {
    let count = RATE * milliseconds / 1_000;
    let samples: Vec<i16> = (0..count)
        .map(|index| {
            let time = index as f32 / RATE as f32;
            // Weiche Hüllkurve gegen Knacken.
            let envelope = (index as f32 / count as f32 * std::f32::consts::PI).sin();
            ((TAU * frequency * time).sin() * envelope * 6_000.0) as i16
        })
        .collect();
    encode_wav(&samples, RATE)
}

pub fn play(sound: Sound) {
    // Müssen statisch sein: asynchrones Abspielen liest noch nach der Rückkehr aus dem Puffer.
    static START: OnceLock<Vec<u8>> = OnceLock::new();
    static STOP: OnceLock<Vec<u8>> = OnceLock::new();
    static ERROR: OnceLock<Vec<u8>> = OnceLock::new();
    let wav = match sound {
        Sound::Start => START.get_or_init(|| tone(880.0, 70)),
        Sound::Stop => STOP.get_or_init(|| tone(620.0, 70)),
        Sound::Error => ERROR.get_or_init(|| tone(220.0, 180)),
    };
    // SAFETY: `wav` ist statisch und mit SND_MEMORY liest Windows direkt daraus.
    let played = unsafe {
        PlaySoundW(PCWSTR(wav.as_ptr().cast()), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT)
    };
    if !played.as_bool() {
        log::warn!("app.sound.play_failed");
    }
}
