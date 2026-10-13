//! Turns audio files into Ingle sound files (`.ings`), prepared the way uploads are: cut to
//! the longest a sound can be, faded at the ends and evened out in loudness.
//!
//!   python3 tools/make-sounds.py /tmp/sounds
//!   cargo run -p ingle-engine --example encode_sound -- /tmp/sounds crates/engine/assets/sounds
//!
//! Every sound file in the first folder (WAV, MP3, OGG Vorbis or Opus, FLAC, M4A) becomes
//! `<name>.ings` in the second.

use ingle_engine::soundboard;

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let (from, to) = (std::path::Path::new(&a[1]), std::path::Path::new(&a[2]));
    std::fs::create_dir_all(to)?;
    let mut names: Vec<_> = std::fs::read_dir(from)?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    names.sort();
    for p in names {
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        if !["wav", "mp3", "ogg", "oga", "opus", "flac", "m4a", "aac", "mp4"].contains(&ext.as_str()) {
            continue;
        }
        let got = soundboard::import(std::fs::read(&p)?, Some(&ext))?;
        let peak = got.pcm.iter().fold(0f32, |m, v| m.max(v.abs()));
        let starts = got.pcm.iter().position(|v| v.abs() > peak / 4.0).unwrap_or(0) as f32 / 2.0 / 48_000.0;
        println!("{}: read {:.3} s, peak {:.3}, sound from {starts:.3} s", p.display(), soundboard::seconds(&got.pcm), peak);
        let pcm = soundboard::prepare(&got.pcm, 0.0, soundboard::seconds(&got.pcm));
        let file = soundboard::encode(&pcm)?;
        let out = to.join(format!("{}.ings", p.file_stem().and_then(|s| s.to_str()).unwrap_or("sound")));
        std::fs::write(&out, &file)?;
        println!("{}: {:.2} s, {} bytes", out.display(), soundboard::seconds(&pcm), file.len());
    }
    Ok(())
}
