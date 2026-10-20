//! One poll's server-side state machine. Pure logic, no I/O, so it can be
//! tested end to end with real cryptography and no sockets.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

use rand_core::{OsRng, RngCore};

use crate::push::{Device, Push};
use whisperpoll_core::{
    combine_tally, Ballot, BulletinBoard, Ceremony, Complaint, Dealing, DkgError,
    PartialDecryption, Poll, Receipt, Registration, TallyResult, TieredResult,
};
use whisperpoll_protocol::slot::Slot;
use whisperpoll_protocol::{
    round_winner, CeremonyView, ClientMsg, Invitation, InviteLink, ParticipantView, Phase,
    PollSummary, PollView, RoundView, SlotSuggestion, BALLOT_KIND,
};

const MAX_NAME: usize = 40;
const MAX_TITLE: usize = 100;
const MAX_PARTICIPANTS: usize = 50;
const MAX_SLOTS: usize = 30;
const MAX_DEVICES: usize = 5;

/// What a participant's device is needed for right now. A push goes out
/// once per need, and only while they're offline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Need {
    SuggestTimes,
    Deal { attempt: u32 },
    Check { attempt: u32 },
    Vote { round: usize },
    Decrypt { round: usize },
    Results,
}

#[derive(Serialize, Deserialize)]
struct Member {
    name: String,
    token: String,
    creator: bool,
    registration: Option<Registration>,
    devices: Vec<Device>,
    /// The last need a push went out for.
    notified: Option<Need>,
}

/// One round of voting and tallying, looking for one day.
#[derive(Default, Serialize, Deserialize)]
struct Round {
    /// Each member's current ballot, carried over from the previous round
    /// until they replace it.
    ballots: BTreeMap<usize, Ballot>,
    /// Members who voted or kept their ballot this round.
    confirmed: BTreeSet<usize>,
    /// Fixed when voting closes.
    board: Option<BulletinBoard>,
    partials: Vec<PartialDecryption>,
    tally: Option<TallyResult>,
    winner: Option<usize>,
}

impl Round {
    /// Ballots in board order: by receipt, so position doesn't reveal the voter.
    fn sorted_ballots(&self) -> Vec<Ballot> {
        let mut ballots: Vec<Ballot> = self.ballots.values().cloned().collect();
        ballots.sort_by_key(|b| b.receipt().0);
        ballots
    }
}

#[derive(Serialize, Deserialize)]
struct KeyCeremony {
    ceremony: Ceremony,
    attempt: u32,
    threshold: usize,
    /// Participant index per trustee; trustee `j` is `trustees[j − 1]`.
    trustees: Vec<usize>,
    dealings: Vec<Option<Dealing>>,
    complaints: Vec<Option<Vec<Complaint>>>,
}

/// Participants (by index) with a live connection right now. Tracked by the
/// storage layer, not stored in the room, so that a crashed server can't
/// leave anyone looking online.
pub type Online = BTreeSet<usize>;

#[derive(Serialize, Deserialize)]
pub struct Room {
    id: String,
    title: String,
    phase: Phase,
    members: Vec<Member>,
    slots: Vec<SlotSuggestion>,
    key: Option<KeyCeremony>,
    options: Vec<String>,
    days: usize,
    poll: Option<Poll>,
    rounds: Vec<Round>,
    /// Every ballot ever accepted, so none can be replayed in a later round.
    used: HashSet<Receipt>,
    /// The open invite (QR code): anyone with it can join under their own
    /// name. Polls stored before it existed get one when loaded.
    #[serde(default = "random_token")]
    join_code: String,
}

pub fn random_token() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn clean_name(name: &str, max: usize) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("name can't be empty".into());
    }
    if name.chars().count() > max {
        return Err(format!("keep it under {max} characters"));
    }
    Ok(name.to_string())
}

impl Room {
    /// A new poll and its creator's token.
    pub fn new(id: String, title: &str, creator: &str) -> Result<(Room, String), String> {
        let token = random_token();
        let room = Room {
            id,
            title: clean_name(title, MAX_TITLE)?,
            phase: Phase::Inviting,
            members: vec![Member {
                name: clean_name(creator, MAX_NAME)?,
                token: token.clone(),
                creator: true,
                registration: None,
                devices: Vec::new(),
                notified: None,
            }],
            slots: Vec::new(),
            key: None,
            options: Vec::new(),
            days: 1,
            poll: None,
            rounds: Vec::new(),
            used: HashSet::new(),
            join_code: random_token(),
        };
        Ok((room, token))
    }

    pub fn member_by_token(&self, token: &str) -> Option<usize> {
        self.members.iter().position(|m| m.token == token)
    }

    fn require_creator(&self, who: usize) -> Result<(), String> {
        if self.members[who].creator {
            Ok(())
        } else {
            Err("only the poll creator can do that".into())
        }
    }

    fn require_phase(&self, phase: Phase) -> Result<(), String> {
        if self.phase == phase {
            Ok(())
        } else {
            Err(format!("not possible during {:?}", self.phase))
        }
    }

    fn registered(&self) -> usize {
        self.members
            .iter()
            .filter(|m| m.registration.is_some())
            .count()
    }

    /// Trustee index (1-based) of participant `who`, if they're a trustee.
    fn trustee_index(&self, who: usize) -> Option<u32> {
        let key = self.key.as_ref()?;
        key.trustees
            .iter()
            .position(|&p| p == who)
            .map(|i| i as u32 + 1)
    }

    pub fn apply(&mut self, who: usize, msg: ClientMsg) -> Result<(), String> {
        match msg {
            ClientMsg::Register(reg) => self.register(who, reg),
            ClientMsg::Invite { name } => self.invite(who, &name),
            ClientMsg::NewJoinCode => {
                self.require_creator(who)?;
                self.join_code = random_token();
                Ok(())
            }
            ClientMsg::SuggestSlot { slot } => self.suggest_slot(who, &slot),
            ClientMsg::StartCeremony { threshold, days } => {
                self.start_ceremony(who, threshold, days)
            }
            ClientMsg::Deal(d) => self.deal(who, d),
            ClientMsg::Checked {
                attempt,
                complaints,
            } => self.checked(who, attempt, complaints),
            ClientMsg::Vote(ballot) => self.vote(who, ballot),
            ClientMsg::KeepVote => self.keep_vote(who),
            ClientMsg::FinishInviting => {
                self.require_creator(who)?;
                self.require_phase(Phase::Inviting)?;
                self.phase = Phase::Gathering;
                Ok(())
            }
            ClientMsg::CloseVoting => {
                self.require_creator(who)?;
                self.require_phase(Phase::Voting)?;
                self.close_voting();
                Ok(())
            }
            ClientMsg::Partial(p) => self.partial(who, p),
            ClientMsg::RegisterDevice { platform, token } => {
                if token.is_empty() || token.len() > 200 {
                    return Err("invalid device token".into());
                }
                let devices = &mut self.members[who].devices;
                devices.retain(|d| d.token != token);
                devices.push(Device { platform, token });
                if devices.len() > MAX_DEVICES {
                    devices.remove(0);
                }
                Ok(())
            }
        }
    }

    /// Registers (or replaces) a participant's trustee key. Keys are bound to
    /// a browser, so someone switching browsers re-registers from the new one.
    /// That's allowed until voting opens: during the key ceremony it restarts
    /// the ceremony, which is harmless as nothing is encrypted yet.
    fn register(&mut self, who: usize, reg: Registration) -> Result<(), String> {
        if !matches!(
            self.phase,
            Phase::Inviting | Phase::Gathering | Phase::KeyCeremony
        ) {
            return Err("trustee keys are fixed once voting opens".into());
        }
        if !reg.verify(&self.id) {
            return Err("invalid trustee registration".into());
        }
        let taken = self.members.iter().enumerate().any(|(i, m)| {
            i != who
                && m.registration
                    .as_ref()
                    .is_some_and(|r| r.key_bytes() == reg.key_bytes())
        });
        if taken {
            return Err("that trustee key is already registered".into());
        }
        self.members[who].registration = Some(reg);
        if self.phase == Phase::KeyCeremony {
            let k = self.key.as_ref().unwrap();
            let (threshold, attempt) = (k.threshold, k.attempt + 1);
            self.begin_ceremony(threshold, attempt)?;
        }
        Ok(())
    }

    /// Starts (or restarts) the key ceremony among everyone registered.
    fn begin_ceremony(&mut self, threshold: usize, attempt: u32) -> Result<(), String> {
        let (trustees, regs): (Vec<usize>, Vec<Registration>) = self
            .members
            .iter()
            .enumerate()
            .filter_map(|(i, m)| m.registration.clone().map(|r| (i, r)))
            .unzip();
        // Restarts only replace or add keys, so the creator's threshold still fits.
        let ceremony = Ceremony::new(&self.id, threshold, &regs).map_err(|e| e.to_string())?;
        let n = trustees.len();
        self.key = Some(KeyCeremony {
            ceremony,
            attempt,
            threshold,
            trustees,
            dealings: vec![None; n],
            complaints: vec![None; n],
        });
        Ok(())
    }

    fn invite(&mut self, who: usize, name: &str) -> Result<(), String> {
        self.require_creator(who)?;
        self.add_member(name).map(|_| ())
    }

    fn invites_open(&self) -> bool {
        matches!(
            self.phase,
            Phase::Inviting | Phase::Gathering | Phase::Voting
        )
    }

    /// What the open invite `code` is for, if it's this poll's and people
    /// can still join.
    pub fn invitation(&self, code: &str) -> Option<Invitation> {
        (self.invites_open() && code == self.join_code).then(|| Invitation {
            title: self.title.clone(),
            from: self.members[0].name.clone(),
        })
    }

    /// Joins through the open invite under `name`; returns the new
    /// participant's token.
    pub fn join(&mut self, code: &str, name: &str) -> Result<String, String> {
        if code != self.join_code {
            return Err("this invite code is no longer valid".into());
        }
        self.add_member(name)
    }

    fn add_member(&mut self, name: &str) -> Result<String, String> {
        if !self.invites_open() {
            return Err("invites are closed".into());
        }
        if self.members.len() >= MAX_PARTICIPANTS {
            return Err("too many participants".into());
        }
        let name = clean_name(name, MAX_NAME)?;
        if self
            .members
            .iter()
            .any(|m| m.name.eq_ignore_ascii_case(&name))
        {
            return Err(format!(
                "someone called {name} is already in this poll; pick another name"
            ));
        }
        let token = random_token();
        self.members.push(Member {
            name,
            token: token.clone(),
            creator: false,
            registration: None,
            devices: Vec::new(),
            notified: None,
        });
        Ok(token)
    }

    fn suggest_slot(&mut self, who: usize, slot: &str) -> Result<(), String> {
        self.require_phase(Phase::Gathering)?;
        let slot = Slot::parse(slot)
            .ok_or("a slot needs a date, a start time and a duration of 15 minutes to a day")?;
        let text = slot.to_string();
        if self.slots.iter().any(|s| s.slot == text) {
            return Ok(());
        }
        if self.slots.len() >= MAX_SLOTS {
            return Err("too many suggestions".into());
        }
        self.slots.push(SlotSuggestion {
            slot: text,
            by: self.members[who].name.clone(),
        });
        // Earliest first, comparing instants (suggesters may be in different time zones).
        self.slots
            .sort_by_key(|s| Slot::parse(&s.slot).map(|s| s.start_epoch_minutes()));
        Ok(())
    }

    fn start_ceremony(&mut self, who: usize, threshold: usize, days: usize) -> Result<(), String> {
        self.require_creator(who)?;
        self.require_phase(Phase::Gathering)?;
        if self.slots.is_empty() {
            return Err("suggest at least one time first".into());
        }
        if !(1..=self.slots.len()).contains(&days) {
            return Err(format!("you can look for 1 to {} times", self.slots.len()));
        }
        if !(1..=self.registered()).contains(&threshold) {
            return Err(DkgError::InvalidThreshold.to_string());
        }
        self.begin_ceremony(threshold, 0)?;
        self.options = self.slots.iter().map(|s| s.slot.clone()).collect();
        self.days = days;
        self.phase = Phase::KeyCeremony;
        Ok(())
    }

    fn deal(&mut self, who: usize, d: Dealing) -> Result<(), String> {
        self.require_phase(Phase::KeyCeremony)?;
        let j = self.trustee_index(who).ok_or("you're not a trustee")?;
        if d.dealer != j {
            return Err("dealing is for another trustee".into());
        }
        let key = self.key.as_mut().unwrap();
        if !key.ceremony.verify_dealing(&d) {
            // Typically dealt for a ceremony that has since restarted.
            return Err("dealing doesn't match the current key ceremony".into());
        }
        let slot = &mut key.dealings[j as usize - 1];
        if slot.is_some() {
            // First dealing wins; a second one would be equivocation.
            return Err("you've already dealt".into());
        }
        *slot = Some(d);
        Ok(())
    }

    fn checked(
        &mut self,
        who: usize,
        attempt: u32,
        complaints: Vec<Complaint>,
    ) -> Result<(), String> {
        self.require_phase(Phase::KeyCeremony)?;
        let j = self.trustee_index(who).ok_or("you're not a trustee")?;
        let key = self.key.as_mut().unwrap();
        if attempt != key.attempt {
            return Err("checked an earlier key ceremony".into());
        }
        if key.dealings.iter().any(Option::is_none) {
            return Err("not everyone has dealt yet".into());
        }
        if complaints.iter().any(|c| c.accuser != j) {
            return Err("complaints must be your own".into());
        }
        let slot = &mut key.complaints[j as usize - 1];
        if slot.is_some() {
            return Err("you've already checked".into());
        }
        *slot = Some(complaints);
        if key.complaints.iter().all(Option::is_some) {
            self.finish_ceremony();
        }
        Ok(())
    }

    fn finish_ceremony(&mut self) {
        let key = self.key.as_ref().unwrap();
        let dealings: Vec<Dealing> = key.dealings.iter().flatten().cloned().collect();
        let complaints: Vec<Complaint> =
            key.complaints.iter().flatten().flatten().cloned().collect();
        match key.ceremony.finalize(&dealings, &complaints) {
            Ok(tally_key) => {
                self.poll = Some(Poll::new(
                    self.id.clone(),
                    self.options.clone(),
                    BALLOT_KIND,
                    tally_key,
                ));
                self.start_round();
            }
            Err(e) => {
                self.phase = Phase::Failed {
                    reason: format!("key ceremony failed: {e}"),
                }
            }
        }
    }

    fn round(&mut self) -> &mut Round {
        self.rounds.last_mut().expect("voting has started")
    }

    /// Opens the next round with everyone's ballots carried over.
    fn start_round(&mut self) {
        let ballots = self
            .rounds
            .last()
            .map(|r| r.ballots.clone())
            .unwrap_or_default();
        self.rounds.push(Round {
            ballots,
            ..Round::default()
        });
        self.phase = Phase::Voting;
    }

    fn vote(&mut self, who: usize, ballot: Ballot) -> Result<(), String> {
        self.require_phase(Phase::Voting)?;
        let poll = self.poll.as_ref().unwrap();
        poll.verify_ballot(&ballot)
            .map_err(|e| format!("ballot rejected: {e}"))?;
        if !self.used.insert(ballot.receipt()) {
            return Err("that exact ballot has been used before".into());
        }
        let round = self.round();
        round.ballots.insert(who, ballot);
        round.confirmed.insert(who);
        self.close_if_everyone_voted();
        Ok(())
    }

    fn keep_vote(&mut self, who: usize) -> Result<(), String> {
        self.require_phase(Phase::Voting)?;
        let round = self.round();
        if !round.ballots.contains_key(&who) {
            return Err("you have no earlier vote to keep".into());
        }
        round.confirmed.insert(who);
        self.close_if_everyone_voted();
        Ok(())
    }

    fn close_if_everyone_voted(&mut self) {
        let n = self.members.len();
        if (0..n).all(|m| self.round().confirmed.contains(&m)) {
            self.close_voting();
        }
    }

    fn close_voting(&mut self) {
        let mut board = BulletinBoard::new(self.poll.clone().unwrap());
        let round = self.round();
        for ballot in round.sorted_ballots() {
            board
                .submit(ballot)
                .expect("ballots were verified on entry");
        }
        round.board = Some(board);
        self.phase = Phase::Tallying;
    }

    fn partial(&mut self, who: usize, p: PartialDecryption) -> Result<(), String> {
        if self.phase != Phase::Tallying {
            // Trustees decrypt in parallel; ones arriving after the threshold
            // was met are simply not needed.
            return Ok(());
        }
        let j = self.trustee_index(who).ok_or("you're not a trustee")?;
        if p.trustee != j {
            return Err("partial decryption is for another trustee".into());
        }
        let round = self.round();
        if round.partials.iter().any(|q| q.trustee == j) {
            return Err("you've already decrypted".into());
        }
        let board = round.board.as_ref().unwrap();
        if !board.verify_partial(&p) {
            return Err("partial decryption doesn't verify".into());
        }
        round.partials.push(p);
        let Ok(tally) = combine_tally(board, &round.partials) else {
            return Ok(());
        };
        let result = TieredResult::from_tally(&board.poll, &tally).expect("tiered poll");
        let picked: Vec<usize> = self.rounds.iter().filter_map(|r| r.winner).collect();
        let winner = round_winner(&result, &picked);
        let round = self.round();
        round.tally = Some(tally);
        round.winner = winner;
        if winner.is_some() && self.rounds.len() < self.days {
            self.start_round();
        } else {
            self.phase = Phase::Done;
        }
        Ok(())
    }

    fn need(&self, who: usize) -> Option<Need> {
        let trustee = self.trustee_index(who).map(|j| j as usize - 1);
        match self.phase {
            // The creator moved the poll on themselves; everyone else hears of it.
            Phase::Gathering if !self.members[who].creator => Some(Need::SuggestTimes),
            Phase::KeyCeremony => {
                let (k, j) = (self.key.as_ref()?, trustee?);
                let attempt = k.attempt;
                if k.dealings[j].is_none() {
                    Some(Need::Deal { attempt })
                } else if k.dealings.iter().all(Option::is_some) && k.complaints[j].is_none() {
                    Some(Need::Check { attempt })
                } else {
                    None
                }
            }
            Phase::Voting => {
                let round = self.rounds.len();
                (!self.rounds.last()?.confirmed.contains(&who)).then_some(Need::Vote { round })
            }
            Phase::Tallying => {
                let j = trustee? as u32 + 1;
                let round = self.rounds.last()?;
                let done = round.partials.iter().any(|p| p.trustee == j);
                (!done).then_some(Need::Decrypt {
                    round: self.rounds.len(),
                })
            }
            Phase::Done => Some(Need::Results),
            _ => None,
        }
    }

    fn push_for(&self, need: Need) -> Push {
        let days = self.days;
        let (body, wake) = match need {
            Need::SuggestTimes => (
                "Everyone's invited. Suggest times that work for you.".to_string(),
                false,
            ),
            Need::Deal { .. } | Need::Check { .. } => (
                "Your phone is needed for a moment to set up the poll's key.".to_string(),
                true,
            ),
            Need::Vote { round: 1 } => (
                "Voting is open. How well does each time work for you?".to_string(),
                false,
            ),
            Need::Vote { round } => (
                format!("Meeting {round} of {days}: keep your vote, or update it."),
                false,
            ),
            Need::Decrypt { .. } => (
                "Your phone is needed for a moment to reveal the result.".to_string(),
                true,
            ),
            Need::Results => {
                let picked: Vec<String> = self
                    .rounds
                    .iter()
                    .filter_map(|r| r.winner)
                    .filter_map(|o| Slot::parse(&self.options[o]))
                    .map(|slot| slot.short())
                    .collect();
                let body = if picked.is_empty() {
                    "The result is in: no time works for everyone.".to_string()
                } else {
                    format!("The result is in: {}.", picked.join(", "))
                };
                (body, false)
            }
        };
        Push {
            poll: self.id.clone(),
            title: self.title.clone(),
            body,
            wake,
        }
    }

    /// Pushes now due: one per new need of each offline participant who has
    /// a device. Call after every change, including someone going offline.
    pub fn pushes(&mut self, online: &Online) -> Vec<(Vec<Device>, Push)> {
        let mut due = Vec::new();
        for who in 0..self.members.len() {
            let Some(need) = self.need(who) else {
                continue;
            };
            let m = &self.members[who];
            if online.contains(&who) || m.devices.is_empty() || m.notified == Some(need) {
                continue;
            }
            due.push((m.devices.clone(), self.push_for(need)));
            self.members[who].notified = Some(need);
        }
        due
    }

    pub fn summary(&self, who: usize) -> PollSummary {
        PollSummary {
            title: self.title.clone(),
            you: self.members[who].name.clone(),
            phase: self.phase.clone(),
            picked: self
                .rounds
                .iter()
                .filter_map(|r| r.winner)
                .map(|o| self.options[o].clone())
                .collect(),
            your_turn: !matches!(self.need(who), None | Some(Need::Results)),
            participants: self.members.len(),
        }
    }

    pub fn view(&self, who: usize, online: &Online) -> PollView {
        let creator = self.members[who].creator;
        PollView {
            id: self.id.clone(),
            title: self.title.clone(),
            phase: self.phase.clone(),
            me: who,
            participants: self
                .members
                .iter()
                .enumerate()
                .map(|(i, m)| ParticipantView {
                    name: m.name.clone(),
                    creator: m.creator,
                    online: online.contains(&i),
                    registration: m.registration.clone(),
                    has_ballot: self
                        .rounds
                        .last()
                        .is_some_and(|r| r.ballots.contains_key(&i)),
                    voted: self.rounds.last().is_some_and(|r| r.confirmed.contains(&i)),
                })
                .collect(),
            invites: if creator {
                self.members
                    .iter()
                    .filter(|m| !m.creator)
                    .map(|m| InviteLink {
                        name: m.name.clone(),
                        token: m.token.clone(),
                    })
                    .collect()
            } else {
                Vec::new()
            },
            join_code: (creator && self.invites_open()).then(|| self.join_code.clone()),
            slots: self.slots.clone(),
            ceremony: self.key.as_ref().map(|k| CeremonyView {
                attempt: k.attempt,
                threshold: k.threshold,
                trustees: k.trustees.clone(),
                registrations: k
                    .trustees
                    .iter()
                    .map(|&p| self.members[p].registration.clone().unwrap())
                    .collect(),
                dealings: k.dealings.iter().flatten().cloned().collect(),
                checked: k.complaints.iter().map(Option::is_some).collect(),
                complaints: k.complaints.iter().flatten().flatten().cloned().collect(),
            }),
            options: self.options.clone(),
            days: self.days,
            rounds: self
                .rounds
                .iter()
                .map(|r| RoundView {
                    ballots: match &r.board {
                        Some(board) => board.ballots().to_vec(),
                        None => r.sorted_ballots(),
                    },
                    partials: r.partials.clone(),
                    tally: r.tally.clone(),
                    winner: r.winner,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use whisperpoll_core::{cast_tiered_ballot, partial_decrypt, verify_tally, TrusteeIdentity};
    use whisperpoll_protocol::client::{next_step, round_outcome};

    const P: Option<usize> = Some(0);
    const OK: Option<usize> = Some(1);
    const IN: Option<usize> = Some(2);
    const X: Option<usize> = None;

    /// Three people, three dates, key ceremony done: voting is open. Every
    /// step does in-process exactly what the browsers do from their `PollView`.
    fn setup(days: usize) -> (Room, Vec<TrusteeIdentity>) {
        let (mut room, _) = Room::new("p1".into(), "Team dinner", "alice").unwrap();
        for name in ["bob", "carol"] {
            let name = name.into();
            room.apply(0, ClientMsg::Invite { name }).unwrap();
        }
        assert_eq!(room.view(0, &Online::new()).invites.len(), 2);
        assert!(room.view(1, &Online::new()).invites.is_empty());

        let ids: Vec<_> = (0..3)
            .map(|_| TrusteeIdentity::generate(&mut OsRng))
            .collect();
        // Each client registers its key on its own while alice invites.
        settle(&mut room, &ids);
        let early = ClientMsg::SuggestSlot {
            slot: "2026-10-25T18:00+02:00/120".into(),
        };
        assert!(room.apply(1, early).is_err(), "times come after inviting");
        assert!(room.apply(1, ClientMsg::FinishInviting).is_err());
        room.apply(0, ClientMsg::FinishInviting).unwrap();
        for slot in [
            "2026-10-25T18:00+02:00/120",
            "2026-10-24T18:00+02:00/120",
            "2026-10-26T18:00+02:00/120",
        ] {
            let slot = slot.into();
            room.apply(1, ClientMsg::SuggestSlot { slot }).unwrap();
        }
        let start = ClientMsg::StartCeremony { threshold: 2, days };
        assert!(room.apply(1, start.clone()).is_err());
        room.apply(0, start).unwrap();
        assert_eq!(
            room.view(0, &Online::new()).options,
            [
                "2026-10-24T18:00+02:00/120",
                "2026-10-25T18:00+02:00/120",
                "2026-10-26T18:00+02:00/120",
            ]
        );

        // ...and deals and checks shares, until the key is ready.
        settle(&mut room, &ids);
        assert_eq!(room.view(0, &Online::new()).phase, Phase::Voting);
        (room, ids)
    }

    /// Runs every participant's automatic client steps (the shared
    /// `next_step`, as the web and mobile clients do) until nobody has
    /// anything left to send.
    fn settle(room: &mut Room, ids: &[TrusteeIdentity]) {
        let mut sent: Vec<HashSet<String>> = vec![HashSet::new(); ids.len()];
        loop {
            let mut progressed = false;
            for (who, id) in ids.iter().enumerate() {
                let view = room.view(who, &Online::new());
                let step = next_step(&view, id, |k| sent[who].contains(k), &mut OsRng);
                if let Some(step) = step {
                    sent[who].insert(step.key);
                    room.apply(who, step.msg).unwrap();
                    progressed = true;
                }
            }
            if !progressed {
                break;
            }
        }
    }

    fn ballot(room: &Room, choices: &[Option<usize>]) -> Ballot {
        let poll = room.view(0, &Online::new()).poll().unwrap();
        cast_tiered_ballot(&poll, choices, &mut OsRng).unwrap()
    }

    fn vote(room: &mut Room, who: usize, choices: &[Option<usize>]) {
        let b = ballot(room, choices);
        room.apply(who, ClientMsg::Vote(b)).unwrap();
    }

    fn decrypt(room: &mut Room, ids: &[TrusteeIdentity], who: usize) {
        let v = room.view(who, &Online::new());
        let c = v.ceremony.as_ref().unwrap();
        let share = v
            .ceremony()
            .unwrap()
            .key_share(&ids[who], &c.dealings, &c.complaints)
            .unwrap();
        let p = partial_decrypt(&share, &v.board().unwrap(), &mut OsRng);
        room.apply(who, ClientMsg::Partial(p)).unwrap();
    }

    /// What a browser does to check a finished round on its own.
    fn verified_result(room: &Room, round: usize) -> (TieredResult, Option<usize>) {
        let v = room.view(0, &Online::new());
        let board = v.board_for(round).unwrap();
        let r = &v.rounds[round - 1];
        let tally = r.tally.as_ref().unwrap();
        verify_tally(&board, tally).unwrap();
        let result = TieredResult::from_tally(&board.poll, tally).unwrap();
        assert_eq!(round_winner(&result, &v.picked_before(round)), r.winner);
        // The shared client-side check agrees.
        let outcome = round_outcome(&v, round, None).unwrap();
        assert!(outcome.checks_out());
        assert_eq!(
            outcome.winner.map(|p| p.slot),
            r.winner.map(|o| v.options[o].clone())
        );
        (result, r.winner)
    }

    #[test]
    fn single_day_poll() {
        let (mut room, ids) = setup(1);
        // A second ballot from the same person replaces their first.
        vote(&mut room, 0, &[X, X, X]);
        vote(&mut room, 0, &[P, OK, X]);
        vote(&mut room, 1, &[X, P, P]);
        vote(&mut room, 2, &[IN, P, OK]);
        // Everyone voted, so voting closed by itself.
        assert_eq!(room.view(0, &Online::new()).phase, Phase::Tallying);
        assert_eq!(room.view(0, &Online::new()).rounds[0].ballots.len(), 3);

        // Trustees 1 and 3 decrypt; 2 turns up late, which is harmless.
        decrypt(&mut room, &ids, 0);
        decrypt(&mut room, &ids, 2);
        decrypt(&mut room, &ids, 1);

        let v = room.view(1, &Online::new());
        assert_eq!(v.phase, Phase::Done);
        assert_eq!(v.rounds[0].partials.len(), 2);
        let (result, winner) = verified_result(&room, 1);
        assert_eq!(result.available, vec![2, 3, 2]);
        assert_eq!(winner, Some(1));
        assert_eq!(room.view(0, &Online::new()).picked(), [1]);
    }

    #[test]
    fn two_days_with_carried_and_updated_votes() {
        let (mut room, ids) = setup(2);
        vote(&mut room, 0, &[P, OK, X]);
        vote(&mut room, 1, &[X, P, P]);
        vote(&mut room, 2, &[IN, P, OK]);
        decrypt(&mut room, &ids, 0);
        decrypt(&mut room, &ids, 1);
        assert_eq!(verified_result(&room, 1).1, Some(1));

        // Round 2 opens with everyone's ballot carried over, not yet confirmed.
        let v = room.view(0, &Online::new());
        assert_eq!((v.phase.clone(), v.rounds.len()), (Phase::Voting, 2));
        assert!(v.participants.iter().all(|p| p.has_ballot && !p.voted));
        assert_eq!(v.rounds[1].ballots, v.rounds[0].ballots);

        // Nobody can replay a ballot from round 1 as their own.
        let alices_old = v.rounds[0].ballots[0].clone();
        assert!(room.apply(2, ClientMsg::Vote(alices_old)).is_err());

        // bob now prefers Fri; alice and carol keep their votes.
        vote(&mut room, 1, &[P, X, X]);
        room.apply(0, ClientMsg::KeepVote).unwrap();
        assert_eq!(room.view(0, &Online::new()).phase, Phase::Voting);
        room.apply(2, ClientMsg::KeepVote).unwrap();
        assert_eq!(room.view(0, &Online::new()).phase, Phase::Tallying);

        decrypt(&mut room, &ids, 2);
        decrypt(&mut room, &ids, 0);
        let (result, winner) = verified_result(&room, 2);
        // Everyone can make Fri now; Sat is already picked anyway.
        assert_eq!(result.available, vec![3, 2, 1]);
        assert_eq!(winner, Some(0));
        let v = room.view(0, &Online::new());
        assert_eq!(v.phase, Phase::Done);
        assert_eq!(v.picked(), [1, 0]);
    }

    #[test]
    fn a_later_round_can_find_no_date() {
        let (mut room, ids) = setup(3);
        for who in 0..3 {
            vote(&mut room, who, &[X, P, X]);
        }
        decrypt(&mut room, &ids, 0);
        decrypt(&mut room, &ids, 1);
        assert_eq!(room.view(0, &Online::new()).rounds[0].winner, Some(1));

        // Round 2: only Sat works for anyone, and Sat is taken.
        assert!(room.apply(0, ClientMsg::CloseVoting).is_ok());
        decrypt(&mut room, &ids, 1);
        decrypt(&mut room, &ids, 2);
        let v = room.view(0, &Online::new());
        assert_eq!(verified_result(&room, 2).1, None);
        // No third round after a round that found nothing.
        assert_eq!((v.phase.clone(), v.rounds.len()), (Phase::Done, 2));
        assert_eq!(v.picked(), [1]);
    }

    /// bob opens his link in a chat app's browser, then switches to his real
    /// browser after the key ceremony has started.
    #[test]
    fn switching_browsers_restarts_the_key_ceremony() {
        let (mut room, _) = Room::new("p1".into(), "Dinner", "alice").unwrap();
        room.apply(0, ClientMsg::Invite { name: "bob".into() })
            .unwrap();
        let alice = TrusteeIdentity::generate(&mut OsRng);
        let bob_chat_app = TrusteeIdentity::generate(&mut OsRng);
        let bob_browser = TrusteeIdentity::generate(&mut OsRng);
        let register = |id: &TrusteeIdentity| ClientMsg::Register(id.register("p1", &mut OsRng));
        room.apply(0, register(&alice)).unwrap();
        room.apply(1, register(&bob_chat_app)).unwrap();
        room.apply(0, ClientMsg::FinishInviting).unwrap();
        let slot = ClientMsg::SuggestSlot {
            slot: "2026-10-24T18:00+02:00/120".into(),
        };
        room.apply(1, slot).unwrap();
        let start = ClientMsg::StartCeremony {
            threshold: 2,
            days: 1,
        };
        room.apply(0, start).unwrap();

        // alice deals right away; bob's chat-app browser is already closed.
        let c = room.view(0, &Online::new()).ceremony().unwrap();
        let stale = c.deal(&alice, &mut OsRng).unwrap();
        room.apply(0, ClientMsg::Deal(stale.clone())).unwrap();

        // bob's real browser registers its own key: the ceremony restarts.
        room.apply(1, register(&bob_browser)).unwrap();
        let v = room.view(0, &Online::new());
        let cv = v.ceremony.as_ref().unwrap();
        assert_eq!((cv.attempt, cv.dealings.len()), (1, 0));
        let c = v.ceremony().unwrap();
        assert_eq!(c.index_of(&bob_browser), Some(2));
        assert_eq!(c.index_of(&bob_chat_app), None);

        // Leftovers from the first attempt are refused.
        assert!(room.apply(0, ClientMsg::Deal(stale)).is_err());
        let stale_check = ClientMsg::Checked {
            attempt: 0,
            complaints: vec![],
        };
        assert!(room.apply(0, stale_check).is_err());

        // Both deal and check for the new attempt, and voting opens.
        for (who, id) in [(0, &alice), (1, &bob_browser)] {
            let dealing = c.deal(id, &mut OsRng).unwrap();
            room.apply(who, ClientMsg::Deal(dealing)).unwrap();
        }
        let dealings = room.view(0, &Online::new()).ceremony.unwrap().dealings;
        for (who, id) in [(0, &alice), (1, &bob_browser)] {
            let complaints = c.check_dealings(id, &dealings, &mut OsRng).unwrap();
            let checked = ClientMsg::Checked {
                attempt: 1,
                complaints,
            };
            room.apply(who, checked).unwrap();
        }
        assert_eq!(room.view(0, &Online::new()).phase, Phase::Voting);

        // Once voting is open, keys are fixed.
        assert!(room.apply(1, register(&bob_chat_app)).is_err());
    }

    #[test]
    fn moving_on_to_times_pushes_to_invitees() {
        let (mut room, _) = Room::new("p1".into(), "Dinner", "alice").unwrap();
        let online = Online::new();
        room.apply(0, ClientMsg::Invite { name: "bob".into() })
            .unwrap();
        let device = ClientMsg::RegisterDevice {
            platform: whisperpoll_protocol::PushPlatform::Apns,
            token: "b0b".into(),
        };
        room.apply(1, device).unwrap();
        assert!(
            room.pushes(&online).is_empty(),
            "nothing to do while alice invites"
        );
        room.apply(0, ClientMsg::FinishInviting).unwrap();
        let pushes = room.pushes(&online);
        assert_eq!(pushes.len(), 1);
        assert!(pushes[0].1.body.contains("Suggest times"));
    }

    #[test]
    fn pushes_go_to_offline_participants_once_per_need() {
        let (mut room, ids) = setup(1);
        let mut online = Online::new();
        let device = |token: &str| ClientMsg::RegisterDevice {
            platform: whisperpoll_protocol::PushPlatform::Apns,
            token: token.into(),
        };
        // bob is offline with a phone; alice has one too but is online;
        // carol has none.
        room.apply(1, device("b0b")).unwrap();
        room.apply(0, device("a11ce")).unwrap();
        online.insert(0);

        let pushes = room.pushes(&online);
        assert_eq!(pushes.len(), 1);
        let (devices, push) = &pushes[0];
        assert_eq!(devices[0].token, "b0b");
        assert!(push.body.contains("Voting is open") && !push.wake);
        assert!(room.pushes(&online).is_empty(), "one push per need");

        for (who, choices) in [[P, OK, X], [X, P, P], [IN, P, OK]].iter().enumerate() {
            vote(&mut room, who, choices);
        }
        // Revealing the result needs bob's phone: wake it.
        let pushes = room.pushes(&online);
        assert_eq!(pushes.len(), 1);
        assert!(pushes[0].1.wake && pushes[0].1.body.contains("reveal"));

        decrypt(&mut room, &ids, 0);
        decrypt(&mut room, &ids, 2);
        let pushes = room.pushes(&online);
        assert_eq!(pushes.len(), 1);
        assert_eq!(
            pushes[0].1.body,
            "The result is in: Sun 25 Oct, 18:00–20:00."
        );

        // alice gets it too once she goes offline.
        online.remove(&0);
        let pushes = room.pushes(&online);
        assert_eq!((pushes.len(), pushes[0].0[0].token.as_str()), (1, "a11ce"));
    }

    #[test]
    fn slots_are_canonical_and_sorted_by_instant() {
        let (mut room, _) = Room::new("p1".into(), "Dinner", "alice").unwrap();
        room.apply(0, ClientMsg::FinishInviting).unwrap();
        let suggest = |room: &mut Room, s: &str| {
            room.apply(0, ClientMsg::SuggestSlot { slot: s.into() })
                .unwrap()
        };
        // 19:00 at +02:00 is 17:00 UTC: before 18:30 at UTC.
        suggest(&mut room, "2026-10-24T18:30Z/60");
        suggest(&mut room, "2026-10-24T19:00+02:00/60");
        // Same day, later time: a separate slot. A repeat is ignored.
        suggest(&mut room, "2026-10-24T21:00+02:00/90");
        suggest(&mut room, "2026-10-24T21:00+02:00/90");
        let slots: Vec<String> = room
            .view(0, &Online::new())
            .slots
            .into_iter()
            .map(|s| s.slot)
            .collect();
        assert_eq!(
            slots,
            [
                "2026-10-24T19:00+02:00/60",
                "2026-10-24T18:30+00:00/60",
                "2026-10-24T21:00+02:00/90",
            ]
        );
    }

    #[test]
    fn keeping_needs_an_earlier_vote() {
        let (mut room, _) = setup(1);
        assert!(room.apply(0, ClientMsg::KeepVote).is_err());
    }

    #[test]
    fn bad_inputs_are_rejected() {
        let (mut room, _) = Room::new("p1".into(), "Dinner", "alice").unwrap();
        let slot = |s: &str| ClientMsg::SuggestSlot { slot: s.into() };
        let start = |days| ClientMsg::StartCeremony { threshold: 1, days };
        room.apply(0, ClientMsg::FinishInviting).unwrap();
        assert!(room.apply(0, ClientMsg::FinishInviting).is_err());
        assert!(room.apply(0, slot("tomorrow")).is_err());
        assert!(room.apply(0, slot("2026-10-24")).is_err());
        assert!(room.apply(0, slot("2026-10-24T18:00+02:00/5")).is_err());
        assert!(room
            .apply(0, ClientMsg::Invite { name: "  ".into() })
            .is_err());
        room.apply(0, ClientMsg::Invite { name: "bob".into() })
            .unwrap();
        assert!(room
            .apply(0, ClientMsg::Invite { name: "Bob".into() })
            .is_err());
        // A registration for another poll.
        let id = TrusteeIdentity::generate(&mut OsRng);
        let other = id.register("other", &mut OsRng);
        assert!(room.apply(1, ClientMsg::Register(other)).is_err());
        // Can't start without slots or with no trustees.
        assert!(room.apply(0, start(1)).is_err());
        room.apply(0, slot("2026-10-24T18:00+02:00/120")).unwrap();
        assert!(room.apply(0, start(1)).is_err());
        // Can't register someone else's key.
        let reg = id.register("p1", &mut OsRng);
        room.apply(0, ClientMsg::Register(reg.clone())).unwrap();
        assert!(room.apply(1, ClientMsg::Register(reg)).is_err());
        // More picks than slots, or none.
        assert!(room.apply(0, start(2)).is_err());
        assert!(room.apply(0, start(0)).is_err());
        room.apply(0, start(1)).unwrap();
    }

    #[test]
    fn the_open_invite_lets_people_join_under_their_own_name() {
        let (mut room, _) = Room::new("p1".into(), "Dinner", "alice").unwrap();
        let code = room.view(0, &Online::new()).join_code.unwrap();
        assert_eq!(
            room.invitation(&code),
            Some(Invitation {
                title: "Dinner".into(),
                from: "alice".into()
            })
        );
        assert_eq!(room.invitation("nope"), None);
        // Only the creator sees the code.
        let token = room.join(&code, "  carol ").unwrap();
        let carol = room.member_by_token(&token).unwrap();
        assert_eq!(
            room.view(carol, &Online::new()).participants[carol].name,
            "carol"
        );
        assert_eq!(room.view(carol, &Online::new()).join_code, None);
        assert!(room.join(&code, "Carol").is_err(), "names stay unique");
        assert!(room.join("nope", "dave").is_err());

        // A new code retires the old QR.
        assert!(room.apply(carol, ClientMsg::NewJoinCode).is_err());
        room.apply(0, ClientMsg::NewJoinCode).unwrap();
        assert!(room.join(&code, "dave").is_err());
        let fresh = room.view(0, &Online::new()).join_code.unwrap();
        room.join(&fresh, "dave").unwrap();
    }

    #[test]
    fn the_open_invite_closes_with_the_vote() {
        let (mut room, _) = setup(1);
        let code = room.join_code.clone();
        room.phase = Phase::Done;
        assert_eq!(room.invitation(&code), None);
        assert!(room.join(&code, "zed").is_err());
        assert_eq!(room.view(0, &Online::new()).join_code, None);
    }
}
