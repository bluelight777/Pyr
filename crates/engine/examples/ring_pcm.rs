//! Writes a built-in ringtone's sound (decoded: 48 kHz stereo, f32, little-endian) to a file,
//! to compare with what Pyr played (a test's recording of its speakers):
//!
//!   cargo run --release -p pyr-engine --example ring_pcm -- ring-sunrise out.f32
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let id = args.next().unwrap_or_else(|| pyr_engine::ring::DEFAULT_RING.to_string());
    let out = args.next().unwrap_or_else(|| format!("{id}.f32"));
    let file = pyr_engine::soundboard::builtin_file(&id).ok_or_else(|| anyhow::anyhow!("no built-in sound {id}"))?;
    let pcm = pyr_engine::soundboard::decode(file)?;
    let bytes: Vec<u8> = pcm.iter().flat_map(|v| v.to_le_bytes()).collect();
    std::fs::write(&out, bytes)?;
    println!("{id}: {:.2} s to {out}", pcm.len() as f32 / 96_000.0);
    Ok(())
}
