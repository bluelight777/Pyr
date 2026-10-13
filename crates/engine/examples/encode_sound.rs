//! Turns audio files into Ingle sound files (`.ings`), prepared the way uploads are: cut to
//! the longest a sound can be, faded at the ends and evened out in loudness.
//!
//!   python3 tools/make-sounds.py /tmp/sounds
//!   cargo run -p ingle-engine --example encode_sound -- /tmp/sounds crates/engine/assets/sounds
//!
//! Every `.wav` (or `.mp3`, `.ogg`, `.flac`) in the first folder becomes `<name>.ings` in the
//! second.

use ingle_engine::soundboard;

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let (from, to) = (std::path::Path::new(&a[1]), std::path::Path::new(&a[2]));
    std::fs::create_dir_all(to)?;
    let mut names: Vec<_> = std::fs::read_dir(from)?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    names.sort();
    for p in names {
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        if !["wav", "mp3", "ogg", "flac"].contains(&ext.as_str()) {
            continue;
        }
        let got = soundboard::import(std::fs::read(&p)?, Some(&ext))?;
        let pcm = soundboard::prepare(&got.pcm, 0.0, soundboard::seconds(&got.pcm));
        let file = soundboard::encode(&pcm)?;
        let out = to.join(format!("{}.ings", p.file_stem().and_then(|s| s.to_str()).unwrap_or("sound")));
        std::fs::write(&out, &file)?;
        println!("{}: {:.2} s, {} bytes", out.display(), soundboard::seconds(&pcm), file.len());
    }
    Ok(())
}
