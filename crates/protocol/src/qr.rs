//! The open invite's QR code, stylized the same way by every client.
//!
//! The code uses the highest error correction, so a star sticker can cover
//! its middle and it still scans. [`Qr::pieces`] lists what to draw: the
//! three finder patterns (big black squares with a dark purple center, kept
//! exactly in proportion, since that's what scanners look for), the data
//! modules (each a slightly inset square in one of three dark tones), and
//! the badge in the middle. Everything is in module units, with the quiet
//! zone outside `0..size`.

use qrcode::{Color, EcLevel, QrCode};

/// The join link a QR code points to.
pub fn join_link(origin: &str, poll: &str, code: &str) -> String {
    format!("{}/j/{poll}?c={code}", origin.trim_end_matches('/'))
}

/// Modules of white around the code, as scanners want.
pub const QUIET: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Qr {
    /// Modules per side.
    pub size: usize,
    /// Row-major, `size * size`.
    pub dark: Vec<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piece {
    /// A 7×7 finder pattern with its top-left module at (`x`, `y`).
    Finder { x: usize, y: usize },
    /// One dark data module; `tone` 0–2 picks its (dark) color.
    Module { x: usize, y: usize, tone: u8 },
    /// The star sticker: a square of `len` modules from (`at`, `at`).
    Badge { at: usize, len: usize },
}

impl Qr {
    pub fn new(text: &str) -> Option<Qr> {
        let code = QrCode::with_error_correction_level(text, EcLevel::H).ok()?;
        let size = code.width();
        let dark = code
            .to_colors()
            .into_iter()
            .map(|c| c == Color::Dark)
            .collect();
        Some(Qr { size, dark })
    }

    pub fn is_dark(&self, x: usize, y: usize) -> bool {
        self.dark[y * self.size + x]
    }

    fn in_finder(&self, x: usize, y: usize) -> bool {
        let far = self.size - 7;
        (y < 7 && (x < 7 || x >= far)) || (x < 7 && y >= far)
    }

    /// The middle square left to the badge: about a fifth of the side,
    /// odd so it centers on a module.
    pub fn badge(&self) -> (usize, usize) {
        let len = (self.size / 5) | 1;
        ((self.size - len) / 2, len)
    }

    pub fn pieces(&self) -> Vec<Piece> {
        let far = self.size - 7;
        let (at, len) = self.badge();
        let in_badge =
            |x: usize, y: usize| (at..at + len).contains(&x) && (at..at + len).contains(&y);
        let mut out = vec![
            Piece::Finder { x: 0, y: 0 },
            Piece::Finder { x: far, y: 0 },
            Piece::Finder { x: 0, y: far },
        ];
        for y in 0..self.size {
            for x in 0..self.size {
                if self.is_dark(x, y) && !self.in_finder(x, y) && !in_badge(x, y) {
                    out.push(Piece::Module {
                        x,
                        y,
                        tone: ((x * 7 + y * 3) % 5 % 3) as u8,
                    });
                }
            }
        }
        out.push(Piece::Badge { at, len });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_links_encode_with_room_for_the_badge() {
        let link = join_link(
            "https://whisperpoll.fly.dev/",
            "1fff6c4513ca",
            &"ab".repeat(16),
        );
        assert_eq!(
            link,
            format!(
                "https://whisperpoll.fly.dev/j/1fff6c4513ca?c={}",
                "ab".repeat(16)
            )
        );
        let qr = Qr::new(&link).unwrap();
        assert_eq!(qr.dark.len(), qr.size * qr.size);
        // Finder corners are dark, and the badge stays small enough for
        // high error correction (30%) to cover with plenty to spare.
        assert!(qr.is_dark(0, 0) && qr.is_dark(qr.size - 1, 0) && qr.is_dark(0, qr.size - 1));
        let (at, len) = qr.badge();
        assert!(len % 2 == 1 && at + len <= qr.size);
        assert!((len * len) as f64 / ((qr.size * qr.size) as f64) < 0.06);
        let pieces = qr.pieces();
        assert_eq!(
            pieces
                .iter()
                .filter(|p| matches!(p, Piece::Finder { .. }))
                .count(),
            3
        );
        assert!(pieces.contains(&Piece::Badge { at, len }));
    }

    /// Draws the pieces the way the clients do (inset modules, the badge
    /// as a blank square, the worst case) and checks a real decoder reads it.
    #[test]
    fn the_stylized_code_still_scans() {
        let link = join_link(
            "https://whisperpoll.fly.dev",
            "1fff6c4513ca",
            &"9f".repeat(16),
        );
        let qr = Qr::new(&link).unwrap();
        let px = 10;
        let side = (qr.size + 2 * QUIET) * px;
        let mut img = vec![255u8; side * side];
        let mut fill = |x0: usize, y0: usize, w: usize, h: usize, v: u8| {
            for y in y0..y0 + h {
                for x in x0..x0 + w {
                    img[y * side + x] = v;
                }
            }
        };
        let at = |m: usize| (m + QUIET) * px;
        for piece in qr.pieces() {
            match piece {
                Piece::Finder { x, y } => {
                    fill(at(x), at(y), 7 * px, 7 * px, 0);
                    fill(at(x + 1), at(y + 1), 5 * px, 5 * px, 255);
                    fill(at(x + 2), at(y + 2), 3 * px, 3 * px, 40);
                }
                Piece::Module { x, y, .. } => fill(at(x) + 1, at(y) + 1, px - 2, px - 2, 30),
                Piece::Badge { at: a, len } => fill(at(a), at(a), len * px, len * px, 255),
            }
        }
        let mut prepared =
            rqrr::PreparedImage::prepare_from_greyscale(side, side, |x, y| img[y * side + x]);
        let grids = prepared.detect_grids();
        assert_eq!(grids.len(), 1);
        let (_, content) = grids[0].decode().unwrap();
        assert_eq!(content, link);
    }
}
