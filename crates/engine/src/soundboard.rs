//! The soundboard: short sounds anyone in a voice channel can play for everyone there, like
//! Discord's. A sound is a small file (see `pyr_proto::sound`); every app plays it itself,
//! so it's heard at full quality, and the call only carries word that it was played.
//!
//! Better than Discord's: no sound is locked (one from any of your servers, or a built-in one,
//! plays in any call: it's uploaded to that call's server first if it isn't there), sounds can
//! be twice as long, uploads are trimmed and evened out in loudness (no sound is painfully
//! loud), everyone gets a join sound, and you can stop or mute other people's sounds.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use pyr_proto::sound;
use serde::{Deserialize, Serialize};

/// 48 kHz stereo, interleaved.
pub type Pcm = Arc<Vec<f32>>;

const RATE: usize = 48_000;
/// Samples (both channels) in one 20 ms frame.
const FRAME: usize = 960 * 2;

/// A sound, as a list of favorites (or the recently played ones) remembers it: where it's
/// from and what it's called, so it shows even while its server is offline.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SoundRef {
    /// The server it's from (its key), or empty for a built-in sound.
    #[serde(default)]
    pub server: String,
    /// Its file id on that server, or the built-in sound's id.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub emoji: String,
    #[serde(default = "full")]
    pub volume: f32,
}

fn full() -> f32 {
    1.0
}

impl SoundRef {
    pub fn is_builtin(&self) -> bool {
        self.server.is_empty()
    }

    /// The same sound (whatever it's called now).
    pub fn same(&self, other: &SoundRef) -> bool {
        self.server == other.server && self.id == other.id
    }

    /// What `PlaySound` calls it on a server.
    pub fn wire_id(&self) -> String {
        if self.is_builtin() { format!("{}{}", sound::BUILTIN_PREFIX, self.id) } else { self.id.clone() }
    }
}

/// One of the sounds every app has (drawn by tools/make-sounds.py: no recordings).
pub struct Builtin {
    pub id: &'static str,
    pub name: &'static str,
    pub emoji: &'static str,
    file: &'static [u8],
}

impl Builtin {
    pub fn sound_ref(&self) -> SoundRef {
        SoundRef { server: String::new(), id: self.id.into(), name: self.name.into(), emoji: self.emoji.into(), volume: 1.0 }
    }
}

macro_rules! builtin {
    ($id:literal, $name:literal, $emoji:literal) => {
        Builtin { id: $id, name: $name, emoji: $emoji, file: include_bytes!(concat!("../assets/sounds/", $id, ".ings")) }
    };
}

pub const BUILTINS: &[Builtin] = &[
    builtin!("airhorn", "Air horn", "📯"),
    builtin!("applause", "Applause", "👏"),
    builtin!("ba-dum-tss", "Ba dum tss", "🥁"),
    builtin!("boing", "Boing", "🌀"),
    builtin!("campfire", "Campfire", "🔥"),
    builtin!("crickets", "Crickets", "🦗"),
    builtin!("ding", "Ding", "🔔"),
    builtin!("drumroll", "Drum roll", "🪘"),
    builtin!("golf-clap", "Golf clap", "⛳"),
    builtin!("pew-pew", "Pew pew", "🔫"),
    builtin!("quack", "Quack", "🦆"),
    builtin!("sad-trombone", "Sad trombone", "🎺"),
    builtin!("spooky", "Spooky", "👻"),
    builtin!("ta-da", "Ta-da", "🎉"),
];

pub fn builtin(id: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.id == id)
}

pub fn builtin_file(id: &str) -> Option<&'static [u8]> {
    builtin(id).map(|b| b.file)
}

// ------------------------------------------------------------------ files

/// A sound file from 48 kHz stereo (cut to `sound::MAX_MS`). Opus can overshoot sharp sounds
/// (crackles, claps) by a few dB, so the file is checked once decoded and made quieter until
/// it stays under full scale.
pub fn encode(pcm: &[f32]) -> Result<Vec<u8>> {
    let mut file = encode_raw(pcm)?;
    let mut scale = 1.0f32;
    for _ in 0..4 {
        let peak = decode(&file)?.iter().fold(0f32, |m, v| m.max(v.abs()));
        if peak <= 0.97 {
            break;
        }
        scale *= 0.94 / peak;
        let quieter: Vec<f32> = pcm.iter().map(|v| v * scale).collect();
        file = encode_raw(&quieter)?;
    }
    Ok(file)
}

fn encode_raw(pcm: &[f32]) -> Result<Vec<u8>> {
    let mut enc = opus::Encoder::new(48_000, opus::Channels::Stereo, opus::Application::Audio)?;
    enc.set_bitrate(opus::Bitrate::Bits(112_000))?;
    enc.set_vbr(true)?;
    enc.set_complexity(10)?;
    // One frame more than the sound: Opus runs a few milliseconds behind, and the end would
    // be cut off otherwise.
    let frames = (pcm.len().div_ceil(FRAME) + 1).min(sound::MAX_FRAMES);
    let mut packets = Vec::with_capacity(frames);
    let mut buf = vec![0f32; FRAME];
    let mut out = vec![0u8; 1500];
    for i in 0..frames {
        let from = (i * FRAME).min(pcm.len());
        let to = ((i + 1) * FRAME).min(pcm.len());
        buf[..to - from].copy_from_slice(&pcm[from..to]);
        buf[to - from..].fill(0.0);
        let n = enc.encode_float(&buf, &mut out)?;
        packets.push(out[..n].to_vec());
    }
    Ok(sound::write(2, &packets))
}

/// 48 kHz stereo from a sound file.
pub fn decode(file: &[u8]) -> Result<Vec<f32>> {
    let (fmt, packets) = sound::packets(file).ok_or_else(|| anyhow!("not a Pyr sound"))?;
    let stereo = fmt.channels == 2;
    let mut dec = opus::Decoder::new(48_000, if stereo { opus::Channels::Stereo } else { opus::Channels::Mono })?;
    let mut out = Vec::with_capacity(fmt.frames * FRAME);
    let mut buf = vec![0f32; FRAME];
    for p in packets {
        let n = dec.decode_float(p, &mut buf[..960 * fmt.channels as usize], false)?;
        if stereo {
            out.extend_from_slice(&buf[..n * 2]);
        } else {
            for v in &buf[..n] {
                out.extend_from_slice(&[*v, *v]);
            }
        }
    }
    Ok(out)
}

/// Decoded sounds, by where they're from, so each is only decoded once.
#[derive(Default)]
pub struct Cache {
    pcm: HashMap<(String, String), Pcm>,
    files: HashMap<(String, String), Arc<Vec<u8>>>,
}

impl Cache {
    pub fn pcm(&self, server: &str, id: &str) -> Option<Pcm> {
        self.pcm.get(&(server.to_string(), id.to_string())).cloned()
    }

    pub fn file(&self, server: &str, id: &str) -> Option<Arc<Vec<u8>>> {
        if server.is_empty() {
            return builtin_file(id).map(|b| Arc::new(b.to_vec()));
        }
        self.files.get(&(server.to_string(), id.to_string())).cloned()
    }

    pub fn insert(&mut self, server: &str, id: &str, file: Arc<Vec<u8>>, pcm: Pcm) {
        // A few dozen sounds at most; a megabyte or two each once decoded.
        if self.pcm.len() > 200 {
            self.pcm.clear();
            self.files.clear();
        }
        let k = (server.to_string(), id.to_string());
        self.files.insert(k.clone(), file);
        self.pcm.insert(k, pcm);
    }
}

// ------------------------------------------------------------------ uploads

/// The longest stretch of a file kept for picking a sound from.
pub const MAX_IMPORT_SECS: usize = 180;

/// What a picked audio file holds: 48 kHz stereo (at most `MAX_IMPORT_SECS` of it).
pub struct Imported {
    pub pcm: Vec<f32>,
    /// The file is longer than what was kept.
    pub cut: bool,
}

/// How a picked file's sound is decoded: by Symphonia, or (Ogg Opus, which Symphonia reads
/// but can't decode) by the Opus decoder calls use, skipping the encoder's lead-in.
enum Decoding {
    Symphonia(Box<dyn symphonia::core::codecs::Decoder>),
    Opus { dec: opus::Decoder, channels: usize, skip: usize },
}

/// Read an audio file (MP3, Ogg Vorbis or Opus, WAV, FLAC, AAC/M4A, or a Pyr sound).
pub fn import(bytes: Vec<u8>, extension: Option<&str>) -> Result<Imported> {
    if sound::check(&bytes).is_some() {
        return Ok(Imported { pcm: decode(&bytes)?, cut: false });
    }
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{CODEC_TYPE_NULL, CODEC_TYPE_OPUS, DecoderOptions};
    use symphonia::core::errors::Error as SymErr;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    if let Some(e) = extension {
        hint.with_extension(e);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|_| anyhow!("That isn't a sound file Pyr can read (try MP3, OGG, WAV, FLAC or M4A)."))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| anyhow!("There's no sound in that file."))?;
    let track_id = track.id;
    let params = track.codec_params.clone();
    let mut decoding = if params.codec == CODEC_TYPE_OPUS {
        let channels = params.channels.map(|c| c.count()).unwrap_or(2);
        if !(1..=2).contains(&channels) {
            bail!("Pyr can't read Opus sound with more than two channels (try MP3, OGG, WAV or FLAC).");
        }
        let dec = opus::Decoder::new(48_000, if channels == 2 { opus::Channels::Stereo } else { opus::Channels::Mono })?;
        Decoding::Opus { dec, channels, skip: params.delay.unwrap_or(0) as usize }
    } else {
        let d = symphonia::default::get_codecs()
            .make(&params, &DecoderOptions::default())
            .map_err(|_| anyhow!("Pyr can't play that kind of sound (try MP3, OGG, WAV or FLAC)."))?;
        Decoding::Symphonia(d)
    };
    let mut rate = if params.codec == CODEC_TYPE_OPUS { RATE } else { params.sample_rate.unwrap_or(48_000) as usize };
    let (mut left, mut right) = (Vec::new(), Vec::new());
    let mut cut = false;
    // (Opus packets hold up to 120 ms.)
    let mut opus_buf = vec![0f32; 5760 * 2];
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymErr::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymErr::ResetRequired) => break,
            Err(e) => {
                if left.is_empty() {
                    return Err(anyhow!("couldn't read the sound: {e}"));
                }
                break;
            }
        };
        if packet.track_id() != track_id {
            continue;
        }
        match &mut decoding {
            Decoding::Symphonia(decoder) => {
                let decoded = match decoder.decode(&packet) {
                    Ok(d) => d,
                    // A damaged packet: skip it.
                    Err(SymErr::DecodeError(_)) => continue,
                    Err(e) => return Err(anyhow!("couldn't read the sound: {e}")),
                };
                let spec = *decoded.spec();
                rate = spec.rate as usize;
                let ch = spec.channels.count().max(1);
                let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                buf.copy_interleaved_ref(decoded);
                for frame in buf.samples().chunks(ch) {
                    left.push(frame[0]);
                    right.push(*frame.get(1).unwrap_or(&frame[0]));
                }
            }
            Decoding::Opus { dec, channels, skip } => {
                // A damaged packet: skip it.
                let Ok(n) = dec.decode_float(&packet.data, &mut opus_buf[..5760 * *channels], false) else { continue };
                for frame in opus_buf[..n * *channels].chunks(*channels) {
                    if *skip > 0 {
                        *skip -= 1;
                        continue;
                    }
                    left.push(frame[0]);
                    right.push(*frame.get(1).unwrap_or(&frame[0]));
                }
            }
        }
        if left.len() > MAX_IMPORT_SECS * rate.max(1) {
            cut = true;
            break;
        }
    }
    if left.is_empty() {
        bail!("There's no sound in that file.");
    }
    if !(8_000..=384_000).contains(&rate) {
        bail!("That file's sample rate ({rate} Hz) is unusual; Pyr can't use it.");
    }
    let (l, r) = if rate == RATE { (left, right) } else { (resample(&left, rate), resample(&right, rate)) };
    let mut pcm = Vec::with_capacity(l.len() * 2);
    for (a, b) in l.iter().zip(r.iter()) {
        pcm.push(*a);
        pcm.push(*b);
    }
    Ok(Imported { pcm, cut })
}

fn resample(x: &[f32], from: usize) -> Vec<f32> {
    let mut r = crate::audio::resample::Resampler::new(from as u32, RATE as u32);
    let mut out = Vec::with_capacity(x.len() * RATE / from + 64);
    r.process(x, &mut out);
    // Flush what's still inside the filter.
    r.process(&[0.0; 64], &mut out);
    out.truncate((x.len() as u64 * RATE as u64 / from as u64) as usize);
    out
}

/// How loud sounds are made: the loudest 0.4 s of each at this level (dBFS), peaks kept
/// below -1 dBFS. So nobody's sound is painfully loud, or too quiet to hear.
pub const LEVEL_DB: f32 = -16.0;

/// The part of `pcm` from `start` to `end` seconds, faded at the ends, evened out in
/// loudness and cut to the longest a sound can be: what gets uploaded.
pub fn prepare(pcm: &[f32], start: f32, end: f32) -> Vec<f32> {
    let total = pcm.len() / 2;
    let a = ((start.max(0.0) * RATE as f32) as usize).min(total);
    let b = ((end * RATE as f32) as usize).clamp(a, total).min(a + sound::MAX_MS as usize * RATE / 1000 - FRAME);
    let mut out = pcm[a * 2..b * 2].to_vec();
    // 5 ms fades: no click at the cut.
    let fade = (RATE / 200).min(out.len() / 4);
    for i in 0..fade {
        let g = i as f32 / fade as f32;
        out[i * 2] *= g;
        out[i * 2 + 1] *= g;
        let j = out.len() / 2 - 1 - i;
        out[j * 2] *= g;
        out[j * 2 + 1] *= g;
    }
    level(&mut out);
    out
}

/// Brings the loudest 0.4 s to `LEVEL_DB`, keeping peaks below -1 dBFS (softly).
pub fn level(pcm: &mut [f32]) {
    let frames = pcm.len() / 2;
    if frames == 0 {
        return;
    }
    let w = (RATE * 2 / 5).min(frames);
    let mut sum = 0.0f64;
    let sq = |i: usize| -> f64 {
        let m = 0.5 * (pcm[2 * i] + pcm[2 * i + 1]) as f64;
        m * m
    };
    for i in 0..w {
        sum += sq(i);
    }
    let mut best = sum;
    for i in w..frames {
        sum += sq(i) - sq(i - w);
        best = best.max(sum);
    }
    let rms = (best / w as f64).sqrt().max(1e-9);
    let now_db = 20.0 * rms.log10() as f32;
    // Nearly silent files aren't blown up into hiss.
    let gain = 10f32.powf(((LEVEL_DB - now_db).min(24.0)) / 20.0);
    let limit = 10f32.powf(-1.0 / 20.0);
    for v in pcm.iter_mut() {
        *v = (*v * gain / limit).tanh() * limit;
    }
}

/// How long `pcm` (48 kHz stereo) lasts, in seconds.
pub fn seconds(pcm: &[f32]) -> f32 {
    pcm.len() as f32 / 2.0 / RATE as f32
}

/// The loudness outline of `pcm` in `n` columns (0..1 each), for drawing it.
pub fn outline(pcm: &[f32], n: usize) -> Vec<f32> {
    let frames = pcm.len() / 2;
    if frames == 0 || n == 0 {
        return vec![0.0; n];
    }
    (0..n)
        .map(|c| {
            let a = c * frames / n;
            let b = ((c + 1) * frames / n).max(a + 1).min(frames);
            pcm[a * 2..b * 2].iter().fold(0f32, |m, v| m.max(v.abs())).min(1.0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(secs: f32, f: f32) -> Vec<f32> {
        let n = (secs * RATE as f32) as usize;
        (0..n).flat_map(|i| {
            let v = (i as f32 / RATE as f32 * f * std::f32::consts::TAU).sin() * 0.5;
            [v, v * 0.5]
        })
        .collect()
    }

    #[test]
    fn encode_decode_keeps_the_sound() {
        let pcm = tone(1.0, 440.0);
        let file = encode(&pcm).unwrap();
        let fmt = sound::check(&file).unwrap();
        assert_eq!(fmt.channels, 2);
        assert!((1000..=1040).contains(&fmt.length_ms()), "{} ms", fmt.length_ms());
        assert!(file.len() < 20_000, "{} bytes for a second", file.len());
        let back = decode(&file).unwrap();
        assert!(back.len() >= pcm.len());
        // Same loudness in each channel (Opus runs a few ms late, so compare levels).
        let rms = |x: &[f32], c: usize| (x.iter().skip(c).step_by(2).map(|v| v * v).sum::<f32>() / (x.len() / 2) as f32).sqrt();
        assert!((rms(&back[..pcm.len()], 0) - rms(&pcm, 0)).abs() < 0.05);
        assert!((rms(&back[..pcm.len()], 1) - rms(&pcm, 1)).abs() < 0.05);
    }

    #[test]
    fn sharp_sounds_stay_under_full_scale_once_encoded() {
        // Crackles at the limiter's ceiling: Opus overshoots these.
        let mut seed = 99u32;
        let pcm: Vec<f32> = (0..RATE)
            .flat_map(|i| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let v = if i % 2_400 < 24 { if seed >> 31 == 0 { 0.89 } else { -0.89 } } else { 0.0 };
                [v, v]
            })
            .collect();
        let back = decode(&encode(&pcm).unwrap()).unwrap();
        let peak = back.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak <= 0.97, "peak {peak}");
    }

    #[test]
    fn long_sounds_are_cut_to_the_limit() {
        let pcm = tone(14.0, 300.0);
        let file = encode(&pcm).unwrap();
        assert_eq!(sound::check(&file).unwrap().length_ms(), sound::MAX_MS);
        assert!(file.len() <= sound::MAX_BYTES);
        let prepared = prepare(&pcm, 2.0, 13.5);
        assert!(seconds(&prepared) <= sound::MAX_MS as f32 / 1000.0);
        assert!(sound::check(&encode(&prepared).unwrap()).is_some());
    }

    #[test]
    fn loudness_is_evened_out() {
        let peak = |x: &[f32]| x.iter().fold(0f32, |m, v| m.max(v.abs()));
        let mut quiet: Vec<f32> = tone(1.0, 440.0).iter().map(|v| v * 0.05).collect();
        let mut loud: Vec<f32> = tone(1.0, 440.0).iter().map(|v| (v * 40.0).clamp(-1.0, 1.0)).collect();
        level(&mut quiet);
        level(&mut loud);
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        let (q, l) = (20.0 * rms(&quiet).log10(), 20.0 * rms(&loud).log10());
        assert!((q - l).abs() < 3.0, "quiet {q:.1} dB, loud {l:.1} dB");
        assert!(peak(&loud) < 0.9 && peak(&quiet) < 0.9);
        // Near silence isn't blown up into hiss: at most 24 dB louder.
        let mut hiss: Vec<f32> = tone(1.0, 440.0).iter().map(|v| v * 0.001).collect();
        let before = rms(&hiss);
        level(&mut hiss);
        assert!(20.0 * (rms(&hiss) / before).log10() <= 24.1);
    }

    #[test]
    fn every_builtin_sound_plays() {
        for b in BUILTINS {
            let fmt = sound::check(b.file).unwrap_or_else(|| panic!("{} isn't a good sound file", b.id));
            assert!(fmt.length_ms() <= sound::MAX_MS, "{}", b.id);
            let pcm = decode(b.file).unwrap();
            let loudest = pcm.iter().fold(0f32, |m, v| m.max(v.abs()));
            assert!(loudest > 0.05 && loudest <= 0.97, "{}: peak {loudest}", b.id);
        }
        assert!(builtin("quack").is_some() && builtin("nope").is_none());
    }

    #[test]
    fn imports_a_wav_file() {
        // 0.5 s of 22.05 kHz mono 16-bit WAV.
        let rate = 22_050u32;
        let n = rate as usize / 2;
        let mut wav = Vec::new();
        let data: Vec<u8> = (0..n)
            .flat_map(|i| (((i as f32 / rate as f32 * 440.0 * std::f32::consts::TAU).sin() * 12_000.0) as i16).to_le_bytes())
            .collect();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        let got = import(wav, Some("wav")).unwrap();
        assert!(!got.cut);
        let secs = seconds(&got.pcm);
        assert!((secs - 0.5).abs() < 0.03, "{secs} s");
        assert!(import(b"definitely not audio".to_vec(), None).is_err());
    }

    /// An Ogg page holding one packet.
    fn ogg_page(out: &mut Vec<u8>, kind: u8, granule: u64, seq: u32, packet: &[u8]) {
        let start = out.len();
        out.extend_from_slice(b"OggS");
        out.push(0);
        out.push(kind);
        out.extend_from_slice(&granule.to_le_bytes());
        out.extend_from_slice(&7u32.to_le_bytes());
        out.extend_from_slice(&seq.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        let mut lacing = vec![255u8; packet.len() / 255];
        lacing.push((packet.len() % 255) as u8);
        out.push(lacing.len() as u8);
        out.extend_from_slice(&lacing);
        out.extend_from_slice(packet);
        // Ogg's CRC-32 (polynomial 0x04c11db7, not reflected), over the page.
        let mut crc = 0u32;
        for b in &out[start..] {
            crc ^= (*b as u32) << 24;
            for _ in 0..8 {
                crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04c1_1db7 } else { crc << 1 };
            }
        }
        out[start + 22..start + 26].copy_from_slice(&crc.to_le_bytes());
    }

    #[test]
    fn imports_an_ogg_opus_file() {
        // 0.1 s of silence then 0.3 s of 880 Hz, mono, as an Ogg Opus file (what voice note
        // and recording apps make). Symphonia reads it; Pyr's Opus decoder plays it.
        let mut enc = opus::Encoder::new(48_000, opus::Channels::Mono, opus::Application::Audio).unwrap();
        let pre_skip = enc.get_lookahead().unwrap() as u16;
        let n = RATE * 4 / 10;
        let pcm: Vec<f32> = (0..n)
            .map(|i| if i < RATE / 10 { 0.0 } else { (i as f32 / RATE as f32 * 880.0 * std::f32::consts::TAU).sin() * 0.5 })
            .collect();
        let mut file = Vec::new();
        let mut head = b"OpusHead".to_vec();
        head.push(1);
        head.push(1);
        head.extend_from_slice(&pre_skip.to_le_bytes());
        head.extend_from_slice(&48_000u32.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes());
        head.push(0);
        ogg_page(&mut file, 0x02, 0, 0, &head);
        let mut tags = b"OpusTags".to_vec();
        tags.extend_from_slice(&3u32.to_le_bytes());
        tags.extend_from_slice(b"pyr");
        tags.extend_from_slice(&0u32.to_le_bytes());
        ogg_page(&mut file, 0, 0, 1, &tags);
        // (The encoder's lead-in is pre-skip long: feed that much more at the end.)
        let mut padded = pcm.clone();
        padded.resize(n + pre_skip as usize + 960, 0.0);
        let frames = padded.len() / 960;
        for f in 0..frames {
            let packet = enc.encode_vec_float(&padded[f * 960..(f + 1) * 960], 4000).unwrap();
            let last = f + 1 == frames;
            let granule = ((f + 1) * 960) as u64;
            ogg_page(&mut file, if last { 0x04 } else { 0 }, granule, 2 + f as u32, &packet);
        }
        let got = import(file, Some("opus")).unwrap();
        let peak = got.pcm.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!((peak - 0.5).abs() < 0.1, "plays at the level it was made: {peak}");
        let starts = got.pcm.iter().position(|v| v.abs() > 0.1).unwrap() as f32 / 2.0 / RATE as f32;
        assert!((starts - 0.1).abs() < 0.003, "the encoder's lead-in is skipped: the tone starts at {starts} s");
        // Its pitch: 880 Hz (rising zero crossings over the tone's middle 0.2 s).
        let mid = &got.pcm[(RATE * 15 / 100) * 2..(RATE * 35 / 100) * 2];
        let ups = mid.chunks(2).collect::<Vec<_>>().windows(2).filter(|w| w[0][0] < 0.0 && w[1][0] >= 0.0).count();
        assert!((ups as i32 - 176).abs() <= 2, "{ups} cycles in 0.2 s");
    }
}
