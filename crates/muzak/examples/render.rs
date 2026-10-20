//! Writes one piece to a WAV file, to listen to while working on the composer:
//! `cargo run -p whisperpoll-muzak --example render -- 42 /tmp/muzak.wav && afplay /tmp/muzak.wav`

fn main() {
    let mut args = std::env::args().skip(1);
    let seed = args.next().and_then(|s| s.parse().ok()).unwrap_or(1);
    let path = args.next().unwrap_or_else(|| "muzak.wav".into());
    let piece = whisperpoll_muzak::compose(seed);
    std::fs::write(&path, whisperpoll_muzak::wav(&piece.samples)).expect("couldn't write the file");
    println!("{} → {path}", piece.title);
}
