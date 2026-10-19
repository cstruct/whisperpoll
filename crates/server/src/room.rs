//! One poll's server-side state machine. Pure logic, no I/O, so it can be
//! tested end to end with real cryptography and no sockets.

use rand_core::{OsRng, RngCore};
use whisperpoll_core::{
    combine_tally, BulletinBoard, Ceremony, Complaint, Dealing, PartialDecryption, Poll,
    Registration, TallyResult,
};
use whisperpoll_protocol::{
    is_iso_date, CeremonyView, ClientMsg, DateSuggestion, InviteLink, ParticipantView, Phase,
    PollView, BALLOT_KIND,
};

const MAX_NAME: usize = 40;
const MAX_TITLE: usize = 100;
const MAX_PARTICIPANTS: usize = 50;
const MAX_DATES: usize = 30;

struct Member {
    name: String,
    token: String,
    creator: bool,
    registration: Option<Registration>,
    voted: bool,
    online: usize,
}

struct KeyCeremony {
    ceremony: Ceremony,
    threshold: usize,
    /// Participant index per trustee; trustee `j` is `trustees[j − 1]`.
    trustees: Vec<usize>,
    dealings: Vec<Option<Dealing>>,
    complaints: Vec<Option<Vec<Complaint>>>,
}

pub struct Room {
    id: String,
    title: String,
    phase: Phase,
    members: Vec<Member>,
    dates: Vec<DateSuggestion>,
    key: Option<KeyCeremony>,
    options: Vec<String>,
    board: Option<BulletinBoard>,
    partials: Vec<PartialDecryption>,
    tally: Option<TallyResult>,
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
            phase: Phase::Gathering,
            members: vec![Member {
                name: clean_name(creator, MAX_NAME)?,
                token: token.clone(),
                creator: true,
                registration: None,
                voted: false,
                online: 0,
            }],
            dates: Vec::new(),
            key: None,
            options: Vec::new(),
            board: None,
            partials: Vec::new(),
            tally: None,
        };
        Ok((room, token))
    }

    pub fn member_by_token(&self, token: &str) -> Option<usize> {
        self.members.iter().position(|m| m.token == token)
    }

    pub fn set_online(&mut self, who: usize, online: bool) {
        let m = &mut self.members[who];
        m.online = if online {
            m.online + 1
        } else {
            m.online.saturating_sub(1)
        };
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
            ClientMsg::SuggestDate { date } => self.suggest_date(who, date),
            ClientMsg::StartCeremony { threshold } => self.start_ceremony(who, threshold),
            ClientMsg::Deal(d) => self.deal(who, d),
            ClientMsg::Checked(cs) => self.checked(who, cs),
            ClientMsg::Vote(ballot) => self.vote(who, ballot),
            ClientMsg::CloseVoting => {
                self.require_creator(who)?;
                self.require_phase(Phase::Voting)?;
                self.phase = Phase::Tallying;
                Ok(())
            }
            ClientMsg::Partial(p) => self.partial(who, p),
        }
    }

    fn register(&mut self, who: usize, reg: Registration) -> Result<(), String> {
        self.require_phase(Phase::Gathering)?;
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
        Ok(())
    }

    fn invite(&mut self, who: usize, name: &str) -> Result<(), String> {
        self.require_creator(who)?;
        if !matches!(self.phase, Phase::Gathering | Phase::Voting) {
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
            return Err(format!("{name} is already invited"));
        }
        self.members.push(Member {
            name,
            token: random_token(),
            creator: false,
            registration: None,
            voted: false,
            online: 0,
        });
        Ok(())
    }

    fn suggest_date(&mut self, who: usize, date: String) -> Result<(), String> {
        self.require_phase(Phase::Gathering)?;
        if !is_iso_date(&date) {
            return Err("dates look like 2026-10-24".into());
        }
        if self.dates.iter().any(|d| d.date == date) {
            return Ok(());
        }
        if self.dates.len() >= MAX_DATES {
            return Err("too many dates".into());
        }
        self.dates.push(DateSuggestion {
            date,
            by: self.members[who].name.clone(),
        });
        self.dates.sort_by(|a, b| a.date.cmp(&b.date));
        Ok(())
    }

    fn start_ceremony(&mut self, who: usize, threshold: usize) -> Result<(), String> {
        self.require_creator(who)?;
        self.require_phase(Phase::Gathering)?;
        if self.dates.is_empty() {
            return Err("suggest at least one date first".into());
        }
        let (trustees, regs): (Vec<usize>, Vec<Registration>) = self
            .members
            .iter()
            .enumerate()
            .filter_map(|(i, m)| m.registration.clone().map(|r| (i, r)))
            .unzip();
        let ceremony = Ceremony::new(&self.id, threshold, &regs).map_err(|e| e.to_string())?;
        let n = trustees.len();
        self.key = Some(KeyCeremony {
            ceremony,
            threshold,
            trustees,
            dealings: vec![None; n],
            complaints: vec![None; n],
        });
        self.options = self.dates.iter().map(|d| d.date.clone()).collect();
        self.phase = Phase::KeyCeremony;
        Ok(())
    }

    fn deal(&mut self, who: usize, d: Dealing) -> Result<(), String> {
        self.require_phase(Phase::KeyCeremony)?;
        let j = self.trustee_index(who).ok_or("you're not a trustee")?;
        if d.dealer != j {
            return Err("dealing is for another trustee".into());
        }
        let slot = &mut self.key.as_mut().unwrap().dealings[j as usize - 1];
        if slot.is_some() {
            // First dealing wins; a second one would be equivocation.
            return Err("you've already dealt".into());
        }
        *slot = Some(d);
        Ok(())
    }

    fn checked(&mut self, who: usize, complaints: Vec<Complaint>) -> Result<(), String> {
        self.require_phase(Phase::KeyCeremony)?;
        let j = self.trustee_index(who).ok_or("you're not a trustee")?;
        let key = self.key.as_mut().unwrap();
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
                let poll = Poll::new(
                    self.id.clone(),
                    self.options.clone(),
                    BALLOT_KIND,
                    tally_key,
                );
                self.board = Some(BulletinBoard::new(poll));
                self.phase = Phase::Voting;
            }
            Err(e) => {
                self.phase = Phase::Failed {
                    reason: format!("key ceremony failed: {e}"),
                }
            }
        }
    }

    fn vote(&mut self, who: usize, ballot: whisperpoll_core::Ballot) -> Result<(), String> {
        self.require_phase(Phase::Voting)?;
        if self.members[who].voted {
            return Err("you've already voted".into());
        }
        let board = self.board.as_mut().unwrap();
        board
            .submit(ballot)
            .map_err(|e| format!("ballot rejected: {e}"))?;
        self.members[who].voted = true;
        if self.members.iter().all(|m| m.voted) {
            self.phase = Phase::Tallying;
        }
        Ok(())
    }

    fn partial(&mut self, who: usize, p: PartialDecryption) -> Result<(), String> {
        if self.phase == Phase::Done {
            // Trustees decrypt in parallel; ones arriving after the threshold
            // was met are simply not needed.
            return Ok(());
        }
        self.require_phase(Phase::Tallying)?;
        let j = self.trustee_index(who).ok_or("you're not a trustee")?;
        if p.trustee != j {
            return Err("partial decryption is for another trustee".into());
        }
        if self.partials.iter().any(|q| q.trustee == j) {
            return Err("you've already decrypted".into());
        }
        let board = self.board.as_ref().unwrap();
        if !board.verify_partial(&p) {
            return Err("partial decryption doesn't verify".into());
        }
        self.partials.push(p);
        if let Ok(tally) = combine_tally(board, &self.partials) {
            self.tally = Some(tally);
            self.phase = Phase::Done;
        }
        Ok(())
    }

    pub fn view(&self, who: usize) -> PollView {
        let creator = self.members[who].creator;
        PollView {
            id: self.id.clone(),
            title: self.title.clone(),
            phase: self.phase.clone(),
            me: who,
            participants: self
                .members
                .iter()
                .map(|m| ParticipantView {
                    name: m.name.clone(),
                    creator: m.creator,
                    online: m.online > 0,
                    registration: m.registration.clone(),
                    voted: m.voted,
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
            dates: self.dates.clone(),
            ceremony: self.key.as_ref().map(|k| CeremonyView {
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
            ballots: self
                .board
                .as_ref()
                .map(|b| b.ballots().to_vec())
                .unwrap_or_default(),
            partials: self.partials.clone(),
            tally: self.tally.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use whisperpoll_core::{
        cast_tiered_ballot, partial_decrypt, verify_tally, TieredResult, TrusteeIdentity,
    };

    /// Drives a full poll through `Room`, doing in-process exactly what the
    /// browsers do: everything they compute comes from their `PollView`.
    #[test]
    fn full_poll_lifecycle() {
        let (mut room, _) = Room::new("p1".into(), "Team dinner", "alice").unwrap();
        room.apply(0, ClientMsg::Invite { name: "bob".into() })
            .unwrap();
        room.apply(
            0,
            ClientMsg::Invite {
                name: "carol".into(),
            },
        )
        .unwrap();
        assert_eq!(room.view(0).invites.len(), 2);
        assert!(room.view(1).invites.is_empty());

        let ids: Vec<_> = (0..3)
            .map(|_| TrusteeIdentity::generate(&mut OsRng))
            .collect();
        for (who, id) in ids.iter().enumerate() {
            room.apply(who, ClientMsg::Register(id.register("p1", &mut OsRng)))
                .unwrap();
        }
        for date in ["2026-10-25", "2026-10-24", "2026-10-26"] {
            room.apply(1, ClientMsg::SuggestDate { date: date.into() })
                .unwrap();
        }
        assert!(room
            .apply(1, ClientMsg::StartCeremony { threshold: 2 })
            .is_err());
        room.apply(0, ClientMsg::StartCeremony { threshold: 2 })
            .unwrap();
        assert_eq!(
            room.view(0).options,
            ["2026-10-24", "2026-10-25", "2026-10-26"]
        );

        for (who, id) in ids.iter().enumerate() {
            let c = room.view(who).ceremony().unwrap();
            room.apply(who, ClientMsg::Deal(c.deal(id, &mut OsRng).unwrap()))
                .unwrap();
        }
        for (who, id) in ids.iter().enumerate() {
            let v = room.view(who);
            let dealings = &v.ceremony.as_ref().unwrap().dealings;
            let cs = v
                .ceremony()
                .unwrap()
                .check_dealings(id, dealings, &mut OsRng)
                .unwrap();
            room.apply(who, ClientMsg::Checked(cs)).unwrap();
        }
        assert_eq!(room.view(0).phase, Phase::Voting);

        let votes = [
            [Some(0), Some(1), None],
            [None, Some(0), Some(0)],
            [Some(2), Some(0), Some(1)],
        ];
        for (who, choices) in votes.iter().enumerate() {
            let poll = room.view(who).poll().unwrap();
            let ballot = cast_tiered_ballot(&poll, choices, &mut OsRng).unwrap();
            room.apply(who, ClientMsg::Vote(ballot.clone())).unwrap();
            assert!(room.apply(who, ClientMsg::Vote(ballot)).is_err());
        }
        // Everyone voted, so voting closed by itself.
        assert_eq!(room.view(0).phase, Phase::Tallying);

        // Trustees 1 and 3 decrypt; 2 never shows up.
        for who in [0, 2] {
            let v = room.view(who);
            let c = v.ceremony.as_ref().unwrap();
            let share = v
                .ceremony()
                .unwrap()
                .key_share(&ids[who], &c.dealings, &c.complaints)
                .unwrap();
            let p = partial_decrypt(&share, &v.board().unwrap(), &mut OsRng);
            room.apply(who, ClientMsg::Partial(p)).unwrap();
        }

        // Trustee 2 turns up late: not needed, not an error.
        let v = room.view(1);
        let c = v.ceremony.as_ref().unwrap();
        let share = v
            .ceremony()
            .unwrap()
            .key_share(&ids[1], &c.dealings, &c.complaints)
            .unwrap();
        let late = partial_decrypt(&share, &v.board().unwrap(), &mut OsRng);
        room.apply(1, ClientMsg::Partial(late)).unwrap();

        let v = room.view(1);
        assert_eq!(v.phase, Phase::Done);
        assert_eq!(v.partials.len(), 2);
        let board = v.board().unwrap();
        let tally = v.tally.as_ref().unwrap();
        verify_tally(&board, tally).unwrap();
        let result = TieredResult::from_tally(&board.poll, tally).unwrap();
        assert_eq!(result.available, vec![2, 3, 2]);
        assert_eq!(result.winner(), Some(1));
    }

    #[test]
    fn bad_inputs_are_rejected() {
        let (mut room, _) = Room::new("p1".into(), "Dinner", "alice").unwrap();
        assert!(room
            .apply(
                0,
                ClientMsg::SuggestDate {
                    date: "tomorrow".into()
                }
            )
            .is_err());
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
        assert!(room
            .apply(1, ClientMsg::Register(id.register("other", &mut OsRng)))
            .is_err());
        // Can't start without dates or with no trustees.
        assert!(room
            .apply(0, ClientMsg::StartCeremony { threshold: 1 })
            .is_err());
        room.apply(
            0,
            ClientMsg::SuggestDate {
                date: "2026-10-24".into(),
            },
        )
        .unwrap();
        assert!(room
            .apply(0, ClientMsg::StartCeremony { threshold: 1 })
            .is_err());
        // Can't register someone else's key.
        let reg = id.register("p1", &mut OsRng);
        room.apply(0, ClientMsg::Register(reg.clone())).unwrap();
        assert!(room.apply(1, ClientMsg::Register(reg)).is_err());
    }
}
