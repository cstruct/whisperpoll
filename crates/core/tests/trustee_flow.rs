use rand_core::OsRng;
use whisperpoll_core::*;

struct Setup {
    shares: Vec<KeyShare>,
    board: BulletinBoard,
}

/// Runs the whole key ceremony for `n` trustees with threshold `t`, then
/// opens a 3-day tiered poll under the resulting key.
fn setup(n: usize, t: usize) -> Setup {
    let ids: Vec<_> = (0..n)
        .map(|_| TrusteeIdentity::generate(&mut OsRng))
        .collect();
    let regs: Vec<_> = ids
        .iter()
        .map(|i| i.register("team-dinner", &mut OsRng))
        .collect();
    let ceremony = Ceremony::new("team-dinner", t, &regs).unwrap();

    let dealings: Vec<_> = ids
        .iter()
        .map(|i| ceremony.deal(i, &mut OsRng).unwrap())
        .collect();
    let complaints: Vec<_> = ids
        .iter()
        .flat_map(|i| ceremony.check_dealings(i, &dealings, &mut OsRng).unwrap())
        .collect();
    assert!(complaints.is_empty());

    let key = ceremony.finalize(&dealings, &complaints).unwrap();
    let shares = ids
        .iter()
        .map(|i| ceremony.key_share(i, &dealings, &complaints).unwrap())
        .collect();

    let days = ["Fri 24 Oct", "Sat 25 Oct", "Sun 26 Oct"]
        .map(String::from)
        .to_vec();
    let kind = BallotKind::Tiered { tiers: 2 };
    let mut board = BulletinBoard::new(Poll::new("team-dinner", days, kind, key));
    for choices in [
        [Some(0), Some(1), None],
        [None, Some(0), Some(0)],
        [Some(1), Some(0), None],
    ] {
        let ballot = cast_tiered_ballot(&board.poll, &choices, &mut OsRng).unwrap();
        board.submit(ballot).unwrap();
    }
    Setup { shares, board }
}

fn partials(s: &Setup, trustees: &[usize]) -> Vec<PartialDecryption> {
    trustees
        .iter()
        .map(|&j| partial_decrypt(&s.shares[j - 1], &s.board, &mut OsRng))
        .collect()
}

// (Fri, Sat, Sun) × (tier 0, tier 1).
const EXPECTED: [u64; 6] = [1, 1, 2, 1, 1, 0];

#[test]
fn any_two_of_three_trustees_decrypt_the_same_result() {
    let s = setup(3, 2);
    for subset in [[1, 2], [1, 3], [3, 2]] {
        let tally = combine_tally(&s.board, &partials(&s, &subset)).unwrap();
        assert_eq!(tally.counts, EXPECTED);
        verify_tally(&s.board, &tally).unwrap();
    }
}

#[test]
fn one_trustee_alone_cannot_decrypt() {
    let s = setup(3, 2);
    assert_eq!(
        combine_tally(&s.board, &partials(&s, &[2])),
        Err(Error::NotEnoughPartials { have: 1, need: 2 })
    );
}

#[test]
fn everyone_required_when_threshold_is_n() {
    let s = setup(4, 4);
    assert!(combine_tally(&s.board, &partials(&s, &[1, 2, 4])).is_err());
    let tally = combine_tally(&s.board, &partials(&s, &[1, 2, 3, 4])).unwrap();
    assert_eq!(tally.counts, EXPECTED);
}

#[test]
fn bogus_partials_are_skipped() {
    let s = setup(3, 2);
    // Someone posing as trustee 1 with a key that isn't theirs.
    let impostor = SecretKey::generate(&mut OsRng);
    let fake = partial_decrypt(&KeyShare::from(&impostor), &s.board, &mut OsRng);
    let mut ps = vec![fake];
    ps.extend(partials(&s, &[2]));
    assert_eq!(
        combine_tally(&s.board, &ps),
        Err(Error::NotEnoughPartials { have: 1, need: 2 })
    );
    ps.extend(partials(&s, &[3]));
    assert_eq!(combine_tally(&s.board, &ps).unwrap().counts, EXPECTED);
}

#[test]
fn a_trustee_counted_twice_does_not_meet_the_threshold() {
    let s = setup(3, 2);
    let tally = combine_tally(&s.board, &partials(&s, &[1, 2])).unwrap();
    let mut doubled = tally.clone();
    doubled.partials[1] = doubled.partials[0].clone();
    assert_eq!(
        verify_tally(&s.board, &doubled),
        Err(Error::DuplicatePartial { trustee: 1 })
    );
    let mut short = tally.clone();
    short.partials.pop();
    assert!(verify_tally(&s.board, &short).is_err());
}

#[test]
fn lying_about_counts_is_detected() {
    let s = setup(3, 2);
    let mut tally = combine_tally(&s.board, &partials(&s, &[1, 3])).unwrap();
    tally.counts[2] -= 1;
    assert_eq!(
        verify_tally(&s.board, &tally),
        Err(Error::CountMismatch { cell: 2 })
    );
}

#[test]
fn partials_are_bound_to_the_board_they_decrypt() {
    let s = setup(3, 2);
    let stale = partials(&s, &[1, 2]);
    let mut board = s.board.clone();
    let ballot = cast_tiered_ballot(&board.poll, &[Some(0), None, None], &mut OsRng).unwrap();
    board.submit(ballot).unwrap();
    // Partials made before the last ballot don't decrypt the new aggregate.
    assert!(combine_tally(&board, &stale).is_err());
}
