//! Wire protocol between the whisperpoll server and browser clients.
//!
//! The server is an untrusted relay and public bulletin board: it stores
//! what clients publish and re-checks what it can, but never holds a key.
//! Every client receives the full public state as a [`PollView`] and
//! re-derives the poll key, the board and the result from it with
//! [`whisperpoll_core`], so nobody has to take the server's word for anything.
//!
//! A poll moves through these phases:
//!
//! 1. [`Phase::Gathering`] — the creator invites people; everyone suggests
//!    dates; each browser registers a trustee key as soon as it connects.
//! 2. [`Phase::KeyCeremony`] — the creator locks the dates and starts the
//!    distributed key generation; browsers deal and check shares on their own.
//! 3. [`Phase::Voting`] — everyone files a tiered ballot under the joint key.
//! 4. [`Phase::Tallying`] — trustee browsers publish partial decryptions of
//!    the aggregate until the threshold is met.
//! 5. [`Phase::Done`] — the result is public and anyone can verify it.
//!
//! A poll can look for several days. Each day is a round of steps 3–4:
//! when a round picks a day and more are wanted, the next round opens with
//! everyone's ballot carried over, so voters may update their choices or keep
//! them. Days already picked can't win again; a round where no remaining day
//! works for anyone picks nothing and ends the poll.

pub mod calendar;

use serde::{Deserialize, Serialize};
use whisperpoll_core::{
    Ballot, BallotKind, BulletinBoard, Ceremony, Complaint, Dealing, PartialDecryption, Poll,
    Registration, TallyKey, TallyResult, TieredResult,
};

/// Tier names for the ballot, most preferred first. Leaving a day out of
/// every tier means "can't make it".
pub const TIERS: [&str; 3] = ["Prefer", "OK", "If needed"];

pub const BALLOT_KIND: BallotKind = BallotKind::Tiered { tiers: TIERS.len() };

/// `POST /api/polls` body.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CreatePoll {
    pub title: String,
    pub name: String,
}

/// `POST /api/polls` response: the creator's own link is `/p/{id}?t={token}`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Created {
    pub id: String,
    pub token: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Gathering,
    KeyCeremony,
    Voting,
    Tallying,
    Done,
    Failed { reason: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ParticipantView {
    pub name: String,
    pub creator: bool,
    pub online: bool,
    pub registration: Option<Registration>,
    /// Has a ballot in the current round (cast now or carried over).
    pub has_ballot: bool,
    /// Has voted or confirmed their carried-over ballot this round.
    pub voted: bool,
}

/// An invite link's secret part. Only sent to the poll creator.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct InviteLink {
    pub name: String,
    pub token: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DateSuggestion {
    /// ISO `YYYY-MM-DD`.
    pub date: String,
    pub by: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CeremonyView {
    pub threshold: usize,
    /// Participant index of each trustee; trustee `j` is `trustees[j − 1]`.
    pub trustees: Vec<usize>,
    pub registrations: Vec<Registration>,
    pub dealings: Vec<Dealing>,
    /// Per trustee: whether they've checked their shares.
    pub checked: Vec<bool>,
    pub complaints: Vec<Complaint>,
}

/// Everything one participant is allowed to see, sent on every change.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PollView {
    pub id: String,
    pub title: String,
    pub phase: Phase,
    /// The receiving participant's index into `participants`.
    pub me: usize,
    pub participants: Vec<ParticipantView>,
    pub invites: Vec<InviteLink>,
    pub dates: Vec<DateSuggestion>,
    pub ceremony: Option<CeremonyView>,
    /// The locked candidate dates, once the ceremony has started.
    pub options: Vec<String>,
    /// How many days the poll is looking for.
    pub days: usize,
    /// One round per day looked for so far; the last is the current one.
    pub rounds: Vec<RoundView>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RoundView {
    /// Ballots on this round's board, in board order (sorted by receipt, so
    /// the order says nothing about who cast what).
    pub ballots: Vec<Ballot>,
    pub partials: Vec<PartialDecryption>,
    pub tally: Option<TallyResult>,
    /// The option this round picked, per [`round_winner`]; `None` while
    /// undecided or if no remaining day works for anyone.
    pub winner: Option<usize>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ClientMsg {
    Register(Registration),
    Invite {
        name: String,
    },
    SuggestDate {
        date: String,
    },
    StartCeremony {
        threshold: usize,
        days: usize,
    },
    Deal(Dealing),
    Checked(Vec<Complaint>),
    Vote(Ballot),
    /// Keep the ballot carried over from the previous round as is.
    KeepVote,
    CloseVoting,
    Partial(PartialDecryption),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ServerMsg {
    State(Box<PollView>),
    Error(String),
}

impl PollView {
    pub fn is_creator(&self) -> bool {
        self.participants[self.me].creator
    }

    /// The key ceremony, rebuilt from the published registrations.
    pub fn ceremony(&self) -> Option<Ceremony> {
        let c = self.ceremony.as_ref()?;
        Ceremony::new(&self.id, c.threshold, &c.registrations).ok()
    }

    /// The poll key, derived locally from the published dealings and complaints.
    pub fn tally_key(&self) -> Option<TallyKey> {
        let c = self.ceremony.as_ref()?;
        if !c.checked.iter().all(|&done| done) {
            return None;
        }
        self.ceremony()?.finalize(&c.dealings, &c.complaints).ok()
    }

    pub fn poll(&self) -> Option<Poll> {
        Some(Poll::new(
            self.id.clone(),
            self.options.clone(),
            BALLOT_KIND,
            self.tally_key()?,
        ))
    }

    /// Round `round` (1-based)'s bulletin board, every ballot re-verified locally.
    pub fn board_for(&self, round: usize) -> Option<BulletinBoard> {
        let r = self.rounds.get(round.checked_sub(1)?)?;
        let mut board = BulletinBoard::new(self.poll()?);
        for ballot in &r.ballots {
            board.submit(ballot.clone()).ok()?;
        }
        Some(board)
    }

    /// The current round's bulletin board.
    pub fn board(&self) -> Option<BulletinBoard> {
        self.board_for(self.rounds.len())
    }

    /// Options picked by the rounds before `round` (1-based).
    pub fn picked_before(&self, round: usize) -> Vec<usize> {
        self.rounds
            .iter()
            .take(round.saturating_sub(1))
            .filter_map(|r| r.winner)
            .collect()
    }

    /// Every option picked so far, in round order.
    pub fn picked(&self) -> Vec<usize> {
        self.picked_before(self.rounds.len() + 1)
    }
}

/// A round's pick: the best day not already picked that at least one person
/// can make, in [`TieredResult::order`] (most available, then preference).
pub fn round_winner(result: &TieredResult, already_picked: &[usize]) -> Option<usize> {
    result
        .order()
        .into_iter()
        .find(|o| !already_picked.contains(o) && result.available[*o] > 0)
}

/// `YYYY-MM-DD` with plausible month and day.
pub fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r.clone()].iter().all(u8::is_ascii_digit);
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || !digits(0..4)
        || !digits(5..7)
        || !digits(8..10)
    {
        return false;
    }
    let month: u8 = s[5..7].parse().unwrap_or(0);
    let day: u8 = s[8..10].parse().unwrap_or(0);
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_core::OsRng;
    use whisperpoll_core::TrusteeIdentity;

    #[test]
    fn messages_roundtrip_through_json() {
        let reg = TrusteeIdentity::generate(&mut OsRng).register("p", &mut OsRng);
        let json = serde_json::to_string(&ClientMsg::Register(reg.clone())).unwrap();
        match serde_json::from_str(&json).unwrap() {
            ClientMsg::Register(back) => assert_eq!(back, reg),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn iso_dates() {
        assert!(is_iso_date("2026-10-24"));
        for bad in [
            "2026-13-01",
            "2026-1-01",
            "26-10-24",
            "2026-10-24x",
            "2026/10/24",
            "2026-10-00",
        ] {
            assert!(!is_iso_date(bad), "{bad}");
        }
    }
}
