//! Waiting music for the app: the same procedural composer as the web client.

/// One composed piece, ready to play.
pub struct MuzakPiece {
    /// A made-up title, like "Elevator Sunrise in E♭".
    pub title: String,
    /// The piece as a 16-bit mono WAV file.
    pub wav: Vec<u8>,
}

/// Composes and renders the piece for `seed` (takes a moment, so async).
pub fn compose_muzak(seed: u32) -> MuzakPiece {
    let piece = whisperpoll_muzak::compose(seed as u64);
    MuzakPiece {
        title: piece.title,
        wav: whisperpoll_muzak::wav(&piece.samples),
    }
}
