//! The open invite's QR code, stylized like the web client's (see
//! `whisperpoll_protocol::qr`).

use whisperpoll_protocol::qr::{self, Piece, Qr};

/// One thing to draw, in module units (the quiet zone is outside 0..size).
pub struct QrPiece {
    /// 0: a 7×7 finder pattern at (x, y); 1: a data module at (x, y) in
    /// dark tone `n` (0–2); 2: the star badge, a square of `n` modules at (x, y).
    pub kind: u8,
    pub x: u32,
    pub y: u32,
    pub n: u32,
}

pub struct QrArt {
    /// Modules per side, without the quiet zone.
    pub size: u32,
    pub quiet: u32,
    pub pieces: Vec<QrPiece>,
}

/// The join link the QR code holds: `{server}/j/{poll}?c={code}`.
#[flutter_rust_bridge::frb(sync)]
pub fn join_link(server: String, poll: String, code: String) -> String {
    qr::join_link(&server, &poll, &code)
}

#[flutter_rust_bridge::frb(sync)]
pub fn qr_art(text: String) -> Option<QrArt> {
    let code = Qr::new(&text)?;
    let pieces = code
        .pieces()
        .into_iter()
        .map(|p| match p {
            Piece::Finder { x, y } => QrPiece {
                kind: 0,
                x: x as u32,
                y: y as u32,
                n: 0,
            },
            Piece::Module { x, y, tone } => QrPiece {
                kind: 1,
                x: x as u32,
                y: y as u32,
                n: tone as u32,
            },
            Piece::Badge { at, len } => QrPiece {
                kind: 2,
                x: at as u32,
                y: at as u32,
                n: len as u32,
            },
        })
        .collect();
    Some(QrArt {
        size: code.size as u32,
        quiet: qr::QUIET as u32,
        pieces,
    })
}
