//! Procedurally generated lounge muzak: elevator music in the spirit of a
//! 1996 General MIDI soundtrack, for whisperpoll's waiting rooms.
//!
//! [`compose`] writes a 16-bar piece from a seed: a key, a tempo, a style
//! (bossa or swing), a chord progression drawn from a small Markov chain of
//! diatonic seventh chords, a two-bar melodic motif that is sequenced and
//! varied over the changes, and parts for electric piano, bass, a lead
//! (vibes, flute or square), and soft drums. It's all synthesized here — no
//! samples — into mono `f32` audio, so the web client and the app play the
//! same music. Each seed gives a different piece; clients chain pieces with
//! fresh seeds instead of looping one.

use std::f64::consts::TAU;

/// Samples per second of the rendered audio. Low on purpose: it's muzak.
pub const SAMPLE_RATE: u32 = 22_050;

const BARS: usize = 16;

/// A rendered piece.
pub struct Piece {
    /// A made-up title, like "Elevator Sunrise in E♭".
    pub title: String,
    /// Mono audio at [`SAMPLE_RATE`], within -1..1.
    pub samples: Vec<f32>,
}

/// SplitMix64: tiny, seedable, good enough for music.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in 0..1.
    fn f(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len())]
    }

    fn chance(&mut self, p: f64) -> bool {
        self.f() < p
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Style {
    Bossa,
    Swing,
}

#[derive(Clone, Copy, PartialEq)]
enum Inst {
    EPiano,
    Bass,
    Vibes,
    Flute,
    Square,
    Kick,
    Hat,
    Rim,
}

struct Note {
    start: f64,
    dur: f64,
    midi: f64,
    vel: f64,
    inst: Inst,
}

/// Semitones of the major scale.
const MAJOR: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];

/// Semitones above the key of diatonic scale step `step` (any integer).
fn scale(step: i32) -> i32 {
    let octave = step.div_euclid(7);
    MAJOR[step.rem_euclid(7) as usize] + 12 * octave
}

/// Where each chord (by scale degree, 0 = I) may go next.
fn next_degree(rng: &mut Rng, from: i32) -> i32 {
    let options: &[i32] = match from {
        0 => &[1, 3, 5, 2, 3],
        1 => &[4, 4, 4, 6],
        2 => &[5, 3],
        3 => &[4, 1, 0, 2],
        4 => &[0, 5, 2, 0],
        5 => &[1, 3, 1],
        _ => &[2, 4],
    };
    rng.pick(options)
}

/// Sixteen bars: two eight-bar phrases on the same opening, the first
/// ending on ii–V, the last on V–I.
fn progression(rng: &mut Rng) -> Vec<i32> {
    let mut a = vec![0];
    while a.len() < 6 {
        let last = *a.last().unwrap();
        a.push(next_degree(rng, last));
    }
    let mut bars = a.clone();
    bars.extend([1, 4]);
    bars.extend_from_slice(&a[..4]);
    let bridge = next_degree(rng, a[3]);
    bars.extend([bridge, 1, 4, 0]);
    bars
}

/// The chord on `degree` as scale steps (root, 3rd, 5th, 7th).
fn chord_steps(degree: i32) -> [i32; 4] {
    [degree, degree + 2, degree + 4, degree + 6]
}

struct Song {
    rng: Rng,
    style: Style,
    key: i32,
    beat: f64,
    chords: Vec<i32>,
    notes: Vec<Note>,
}

impl Song {
    /// Seconds at `eighth` (0..8) of bar `bar`, swung if the style swings.
    fn at(&self, bar: usize, eighth: f64) -> f64 {
        let beats = match self.style {
            Style::Swing if eighth.fract() == 0.0 && (eighth as i32) % 2 == 1 => {
                (eighth - 1.0) / 2.0 + 2.0 / 3.0
            }
            _ => eighth / 2.0,
        };
        (bar as f64 * 4.0 + beats) * self.beat
    }

    fn add(&mut self, inst: Inst, bar: usize, eighth: f64, eighths: f64, midi: f64, vel: f64) {
        let start = self.at(bar, eighth);
        let dur = eighths / 2.0 * self.beat;
        // A little human looseness.
        let jitter = (self.rng.f() - 0.5) * 0.012;
        let vel = vel * (0.9 + 0.2 * self.rng.f());
        self.notes.push(Note {
            start: (start + jitter).max(0.0),
            dur,
            midi,
            vel,
            inst,
        });
    }

    fn midi(&self, base: i32, step: i32) -> f64 {
        (base + self.key + scale(step)) as f64
    }

    fn comp(&mut self) {
        let pattern: &[(f64, f64)] = match self.style {
            Style::Bossa => &[(0.0, 2.5), (3.0, 1.5), (5.0, 2.5)],
            Style::Swing => &[(2.0, 1.0), (5.0, 2.5)],
        };
        for bar in 0..BARS {
            let steps = chord_steps(self.chords[bar]);
            // Rootless-ish voicing around middle C: 3rd, 5th, 7th, 9th.
            let voicing = [steps[1], steps[2], steps[3], steps[0] + 8];
            for &(at, len) in pattern {
                let vel = if at == 0.0 { 0.32 } else { 0.26 };
                for &step in &voicing {
                    let midi = self.midi(48, step);
                    // Keep it between E3 and G4.
                    let midi = if midi > 67.0 {
                        midi - 12.0
                    } else if midi < 52.0 {
                        midi + 12.0
                    } else {
                        midi
                    };
                    self.add(Inst::EPiano, bar, at, len, midi, vel);
                }
            }
        }
    }

    fn bass(&mut self) {
        for bar in 0..BARS {
            let root = self.chords[bar];
            let next = self.chords[(bar + 1) % BARS];
            let r = self.midi(36, root);
            let fifth = self.midi(36, root + 4);
            let next_root = self.midi(36, next);
            let approach = if self.rng.chance(0.5) {
                next_root - 1.0
            } else {
                next_root + 1.0
            };
            let line: Vec<(f64, f64, f64)> = match self.style {
                Style::Bossa => vec![
                    (0.0, 2.5, r),
                    (3.0, 1.0, fifth),
                    (4.0, 2.5, r),
                    (7.0, 1.0, approach),
                ],
                Style::Swing => {
                    let third = self.midi(36, root + 2);
                    vec![
                        (0.0, 2.0, r),
                        (2.0, 2.0, third),
                        (4.0, 2.0, fifth),
                        (6.0, 2.0, approach),
                    ]
                }
            };
            for (at, len, midi) in line {
                self.add(Inst::Bass, bar, at, len * 0.9, midi, 0.8);
            }
        }
    }

    fn drums(&mut self) {
        for bar in 0..BARS {
            for e in 0..8 {
                let e = e as f64;
                let hat = if (e as i32) % 2 == 0 { 0.22 } else { 0.14 };
                self.add(Inst::Hat, bar, e, 0.5, 0.0, hat);
            }
            let kicks: &[f64] = match self.style {
                Style::Bossa => &[0.0, 3.0, 4.0, 7.0],
                Style::Swing => &[0.0, 4.0],
            };
            for &k in kicks {
                self.add(Inst::Kick, bar, k, 1.0, 0.0, 0.5);
            }
            // Bossa clave on the rim; swing gets 2 and 4.
            let rims: &[f64] = match (self.style, bar % 2) {
                (Style::Bossa, 0) => &[0.0, 3.0, 6.0],
                (Style::Bossa, _) => &[2.0, 4.0],
                (Style::Swing, _) => &[2.0, 6.0],
            };
            for &r in rims {
                self.add(Inst::Rim, bar, r, 0.5, 0.0, 0.18);
            }
        }
    }

    /// A two-bar motif: (eighth from the start, length in eighths, scale
    /// steps above the chord's root).
    fn motif(&mut self) -> Vec<(f64, f64, i32)> {
        let rhythms: &[&[(f64, f64)]] = &[
            &[(0.0, 3.0), (3.0, 1.0), (4.0, 2.0), (6.0, 2.0), (8.0, 6.0)],
            &[
                (1.0, 1.0),
                (2.0, 2.0),
                (4.0, 1.0),
                (5.0, 3.0),
                (10.0, 2.0),
                (12.0, 4.0),
            ],
            &[
                (0.0, 2.0),
                (2.0, 2.0),
                (4.0, 4.0),
                (9.0, 1.0),
                (10.0, 1.0),
                (11.0, 5.0),
            ],
            &[
                (0.0, 1.5),
                (1.5, 1.5),
                (3.0, 3.0),
                (8.0, 2.0),
                (10.0, 2.0),
                (12.0, 4.0),
            ],
        ];
        let rhythm = self.rng.pick(rhythms);
        let mut step = self.rng.pick(&[2, 4, 7]);
        let mut out = Vec::new();
        for &(at, len) in rhythm {
            out.push((at, len, step));
            let leap = self.rng.pick(&[-2, -1, -1, 1, 1, 2, 3, -3]);
            step = (step + leap).clamp(0, 9);
        }
        out
    }

    /// Plays `motif` over bars `bar`, `bar + 1`, shifted by `shift` steps,
    /// with long or strong notes nudged onto chord tones.
    fn play_motif(
        &mut self,
        inst: Inst,
        motif: &[(f64, f64, i32)],
        bar: usize,
        shift: i32,
        vel: f64,
    ) {
        for &(at, len, step) in motif {
            let b = bar + (at / 8.0) as usize;
            let e = at % 8.0;
            let root = self.chords[b];
            let mut s = root + step + shift;
            if len >= 2.0 || e == 0.0 || e == 4.0 {
                // Nearest chord tone.
                let tones = chord_steps(root);
                s = (s - 1..=s + 1)
                    .min_by_key(|c| {
                        if tones.iter().any(|t| (c - t).rem_euclid(7) == 0) {
                            0
                        } else {
                            1
                        }
                    })
                    .unwrap();
            }
            let midi = self.midi(60, s);
            let midi = if midi > 86.0 {
                midi - 12.0
            } else if midi < 64.0 {
                midi + 12.0
            } else {
                midi
            };
            self.add(inst, b, e, len * 0.95, midi, vel);
        }
    }

    fn melody(&mut self, inst: Inst) {
        let a = self.motif();
        let b = self.motif();
        let vel = 0.5;
        for phrase in [0, 8] {
            self.play_motif(inst, &a, phrase, 0, vel);
            let up = self.rng.pick(&[1, 2, -1]);
            self.play_motif(inst, &a, phrase + 2, up, vel);
            let answer = if phrase == 0 { b.clone() } else { self.motif() };
            self.play_motif(inst, &answer, phrase + 4, 0, vel);
        }
        // Cadences: hang on the 5th of V, then land on the tonic.
        let five = self.midi(60, 4 + 4);
        self.add(inst, 6, 0.0, 12.0, five.min(79.0), vel);
        let home = self.midi(72, 0);
        self.add(inst, 14, 0.0, 4.0, self.midi(60, 1 + 7).min(79.0), vel);
        self.add(inst, 14, 4.0, 4.0, self.midi(60, 6), vel);
        self.add(
            inst,
            15,
            0.0,
            8.0,
            if home > 79.0 { home - 12.0 } else { home },
            vel,
        );
    }
}

fn hz(midi: f64) -> f64 {
    440.0 * 2f64.powf((midi - 69.0) / 12.0)
}

/// One sample of instrument `inst` at `t` seconds into a note of `dur`.
fn voice(inst: Inst, t: f64, dur: f64, f: f64, noise: &mut Rng) -> f64 {
    // Attack-decay envelope with a short release after `dur`.
    let release = |t: f64, dur: f64, r: f64| {
        if t > dur {
            (1.0 - (t - dur) / r).max(0.0)
        } else {
            1.0
        }
    };
    let attack = |t: f64, a: f64| (t / a).min(1.0);
    match inst {
        Inst::EPiano => {
            // Two-operator FM, the sound of every 90s lobby.
            let index = 1.4 * (-t * 5.0).exp() + 0.25;
            let s = (TAU * f * t + index * (TAU * f * t).sin()).sin();
            s * attack(t, 0.004) * (-t * 1.6).exp() * release(t, dur, 0.08)
        }
        Inst::Bass => {
            let s = (TAU * f * t).sin()
                + 0.35 * (TAU * 2.0 * f * t).sin()
                + 0.1 * (TAU * 3.0 * f * t).sin();
            s * attack(t, 0.006) * (-t * 2.2).exp() * release(t, dur, 0.05)
        }
        Inst::Vibes => {
            let tremolo = 1.0 + 0.25 * (TAU * 5.5 * t).sin();
            let s = (TAU * f * t).sin() + 0.25 * (TAU * 4.0 * f * t).sin() * (-t * 8.0).exp();
            s * tremolo * attack(t, 0.003) * (-t * 1.3).exp() * release(t, dur, 0.15)
        }
        Inst::Flute => {
            let vibrato = 1.0 + 0.006 * (TAU * 5.0 * t).sin() * (t * 2.0).min(1.0);
            let ph = TAU * f * vibrato * t;
            let breath = (noise.f() - 0.5) * 0.08 * (-t * 10.0).exp();
            let s = ph.sin() + 0.12 * (2.0 * ph).sin() + breath;
            s * attack(t, 0.06) * release(t, dur, 0.08)
        }
        Inst::Square => {
            // A soft 25% pulse, approximated by a few harmonics.
            let ph = TAU * f * t * (1.0 + 0.004 * (TAU * 5.0 * t).sin());
            let mut s = 0.0;
            for k in 1..=6 {
                let k = k as f64;
                s += (k * std::f64::consts::FRAC_PI_4).sin() / k * (k * ph).cos();
            }
            0.8 * s * attack(t, 0.01) * (0.6 + 0.4 * (-t * 3.0).exp()) * release(t, dur, 0.06)
        }
        Inst::Kick => {
            // A sine sweeping from 115 Hz down to 45 Hz (the phase is its integral).
            let ph = TAU * (45.0 * t + 70.0 * (1.0 - (-t * 30.0).exp()) / 30.0);
            ph.sin() * (-t * 14.0).exp()
        }
        Inst::Hat => (noise.f() * 2.0 - 1.0) * (-t * 60.0).exp() * 0.5,
        Inst::Rim => {
            let s = (TAU * 1700.0 * t).sin() + (noise.f() - 0.5) * 0.6;
            s * (-t * 90.0).exp()
        }
    }
}

/// How long a note of `inst` keeps sounding past its nominal end.
fn tail(inst: Inst) -> f64 {
    match inst {
        Inst::Kick => 0.35,
        Inst::Hat | Inst::Rim => 0.08,
        Inst::Vibes => 0.15,
        _ => 0.1,
    }
}

fn gain(inst: Inst) -> f64 {
    match inst {
        Inst::EPiano => 0.18,
        Inst::Bass => 0.42,
        Inst::Vibes => 0.3,
        Inst::Flute => 0.24,
        Inst::Square => 0.15,
        Inst::Kick => 0.55,
        Inst::Hat => 0.12,
        Inst::Rim => 0.18,
    }
}

const KEYS: [(i32, &str); 7] = [
    (0, "C"),
    (2, "D"),
    (3, "E♭"),
    (5, "F"),
    (7, "G"),
    (8, "A♭"),
    (10, "B♭"),
];
const WORDS_A: [&str; 10] = [
    "Elevator",
    "Lobby",
    "Escalator",
    "Atrium",
    "Mezzanine",
    "Fax Machine",
    "Hold Line",
    "Concourse",
    "Waiting Room",
    "Lounge",
];
const WORDS_B: [&str; 10] = [
    "Sunrise",
    "Serenade",
    "Bossa",
    "Daydream",
    "Reverie",
    "Interlude",
    "Shuffle",
    "Breeze",
    "Rendezvous",
    "Groove",
];

/// Composes and renders the piece for `seed`.
pub fn compose(seed: u64) -> Piece {
    let mut rng = Rng(seed ^ 0x6D75_7A61_6B21);
    let (key, key_name) = rng.pick(&KEYS);
    let style = if rng.chance(0.6) {
        Style::Bossa
    } else {
        Style::Swing
    };
    let bpm = 84.0 + rng.f() * 24.0;
    let lead = rng.pick(&[Inst::Vibes, Inst::Flute, Inst::Square, Inst::Vibes]);
    let chords = progression(&mut rng);
    let title = format!(
        "{} {} in {}",
        rng.pick(&WORDS_A),
        rng.pick(&WORDS_B),
        key_name
    );

    let mut song = Song {
        rng,
        style,
        key,
        beat: 60.0 / bpm,
        chords,
        notes: Vec::new(),
    };
    song.comp();
    song.bass();
    song.drums();
    song.melody(lead);

    Piece {
        title,
        samples: render(&song),
    }
}

fn render(song: &Song) -> Vec<f32> {
    let rate = SAMPLE_RATE as f64;
    let length = BARS as f64 * 4.0 * song.beat;
    let echo_delay = 0.75 * song.beat;
    // Room for the last echoes to die away.
    let total = ((length + 2.0) * rate) as usize;
    let mut dry = vec![0f64; total];
    let mut send = vec![0f64; total];
    let mut noise = Rng(0x5EED);

    for note in &song.notes {
        let f = hz(note.midi);
        let start = (note.start * rate) as usize;
        let len = ((note.dur + tail(note.inst)) * rate) as usize;
        let amp = note.vel * gain(note.inst);
        let wet = matches!(
            note.inst,
            Inst::EPiano | Inst::Vibes | Inst::Flute | Inst::Square | Inst::Rim
        );
        for i in 0..len {
            let Some(slot) = dry.get_mut(start + i) else {
                break;
            };
            let t = i as f64 / rate;
            let s = voice(note.inst, t, note.dur, f, &mut noise) * amp;
            *slot += s;
            if wet {
                send[start + i] += s;
            }
        }
    }

    // A feedback echo, the cheapest way to sound expensive: `send` becomes
    // the echo line, heard one delay later through a gentle low-pass.
    let d = (echo_delay * rate) as usize;
    for i in d..total {
        send[i] += send[i - d] * 0.32;
    }
    let mut lp = 0.0;
    let mut out: Vec<f64> = dry
        .iter()
        .enumerate()
        .map(|(i, &x)| {
            let echoed = if i >= d { send[i - d] } else { 0.0 };
            lp += 0.35 * (echoed - lp);
            x + 0.3 * lp
        })
        .collect();

    // Fade the very end, normalize, and soft-clip.
    let fade = (0.5 * rate) as usize;
    for (k, s) in out.iter_mut().rev().take(fade).enumerate() {
        *s *= k as f64 / fade as f64;
    }
    let peak = out.iter().fold(0f64, |m, s| m.max(s.abs())).max(1e-9);
    out.iter()
        .map(|s| ((s / peak) * 1.1).tanh() as f32 * 0.8)
        .collect()
}

/// `samples` at [`SAMPLE_RATE`] as a 16-bit mono WAV file.
pub fn wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pieces_are_deterministic_and_differ_by_seed() {
        let a = compose(1);
        let b = compose(1);
        let c = compose(2);
        assert_eq!(a.title, b.title);
        assert_eq!(a.samples, b.samples);
        assert_ne!(a.samples, c.samples);
    }

    #[test]
    fn pieces_are_a_sensible_length_and_level() {
        for seed in 0..8 {
            let p = compose(seed);
            let secs = p.samples.len() as f64 / SAMPLE_RATE as f64;
            assert!((30.0..60.0).contains(&secs), "{secs}s");
            let peak = p.samples.iter().fold(0f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.5 && peak <= 0.8, "peak {peak}");
            assert!(p.samples.iter().all(|s| s.is_finite()));
        }
    }

    #[test]
    fn progressions_open_on_the_tonic_and_close_with_v_i() {
        for seed in 0..32 {
            let chords = progression(&mut Rng(seed));
            assert_eq!(chords.len(), BARS);
            assert_eq!(chords[0], 0);
            assert_eq!(&chords[6..8], &[1, 4]);
            assert_eq!(&chords[14..], &[4, 0]);
        }
    }

    #[test]
    fn wav_header_is_valid() {
        let bytes = wav(&[0.0, 0.5, -0.5]);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(bytes.len(), 44 + 6);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 6);
    }
}
