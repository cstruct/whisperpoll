use rand_core::OsRng;
use whisperpoll_core::*;

// Tiers: 0 = Prefer, 1 = OK, 2 = If needed; None = can't make it.
const PREFER: Option<usize> = Some(0);
const OK: Option<usize> = Some(1);
const IF_NEEDED: Option<usize> = Some(2);
const CANT: Option<usize> = None;

fn setup() -> (SecretKey, BulletinBoard) {
    let sk = SecretKey::generate(&mut OsRng);
    let days = ["Fri 24 Oct", "Sat 25 Oct", "Sun 26 Oct"]
        .map(String::from)
        .to_vec();
    let poll = Poll::new(
        "team-dinner",
        days,
        BallotKind::Tiered { tiers: 3 },
        sk.public_key(),
    );
    (sk, BulletinBoard::new(poll))
}

fn submit(board: &mut BulletinBoard, choices: &[Option<usize>]) -> Receipt {
    let ballot = cast_tiered_ballot(&board.poll, choices, &mut OsRng).unwrap();
    board.submit(ballot).unwrap()
}

#[test]
fn tiered_tally_gives_availability_and_preference() {
    let (sk, mut board) = setup();
    let receipts = [
        submit(&mut board, &[OK, PREFER, CANT]),
        submit(&mut board, &[CANT, PREFER, PREFER]), // tie between Sat and Sun
        submit(&mut board, &[PREFER, IF_NEEDED, PREFER]),
        submit(&mut board, &[CANT, CANT, OK]), // partial ballot
    ];

    let tally = decrypt_tally(&sk, &board, &mut OsRng);
    verify_tally(&board, &tally).unwrap();
    assert!(receipts.iter().all(|r| board.contains(r)));

    let result = TieredResult::from_tally(&board.poll, &tally).unwrap();
    assert_eq!(
        result.counts,
        vec![vec![1, 1, 0], vec![2, 0, 1], vec![2, 1, 0]]
    );
    assert_eq!(result.available, vec![2, 3, 3]);
    // 3 / 2 / 1 points per tier.
    assert_eq!(result.score, vec![5, 7, 8]);
    // Sat and Sun are tied on availability; Sun wins on preference. Fri trails.
    assert_eq!(result.order(), vec![2, 1, 0]);
}

#[test]
fn availability_outranks_preference() {
    let (sk, mut board) = setup();
    submit(&mut board, &[PREFER, IF_NEEDED, CANT]);
    submit(&mut board, &[PREFER, IF_NEEDED, CANT]);
    submit(&mut board, &[CANT, IF_NEEDED, CANT]);
    let tally = decrypt_tally(&sk, &board, &mut OsRng);
    let result = TieredResult::from_tally(&board.poll, &tally).unwrap();
    // Fri is everyone's favourite that can come, but only Sat works for all three.
    assert_eq!(result.score[0], 6);
    assert_eq!(result.score[1], 3);
    assert_eq!(result.order()[0], 1);
}

#[test]
fn empty_ballot_is_valid_and_counts_nothing() {
    let (sk, mut board) = setup();
    submit(&mut board, &[CANT, CANT, CANT]);
    let tally = decrypt_tally(&sk, &board, &mut OsRng);
    verify_tally(&board, &tally).unwrap();
    assert!(tally.counts.iter().all(|&c| c == 0));
}

#[test]
fn nonexistent_tier_is_refused_when_casting() {
    let (_, board) = setup();
    assert_eq!(
        cast_tiered_ballot(&board.poll, &[OK, Some(3), CANT], &mut OsRng),
        Err(Error::InvalidTier { option: 1 })
    );
    assert!(cast_tiered_ballot(&board.poll, &[OK, CANT], &mut OsRng).is_err());
}

#[test]
fn putting_a_day_in_two_tiers_is_rejected() {
    let (_, mut board) = setup();
    // Ballot A puts Fri in tier 0. Graft in B's validly-proven cell that puts
    // Fri in tier 1: Fri now counts twice.
    let a = cast_tiered_ballot(&board.poll, &[PREFER, CANT, CANT], &mut OsRng).unwrap();
    let b = cast_tiered_ballot(&board.poll, &[OK, CANT, CANT], &mut OsRng).unwrap();
    let mut forged = a.clone();
    forged.choices[1] = b.choices[1].clone();
    assert_eq!(
        board.submit(forged),
        Err(Error::InvalidRowProof { option: 0 })
    );
}

#[test]
fn wrong_kind_of_sum_proof_is_rejected() {
    let (_, mut board) = setup();
    let ranked = Poll::new(
        "team-dinner",
        board.poll.options.clone(),
        BallotKind::Ranked,
        board.poll.public_key,
    );
    let r = cast_ranked_ballot(&ranked, &[0, 1, 2], &mut OsRng).unwrap();
    let mut ballot = cast_tiered_ballot(&board.poll, &[PREFER, OK, OK], &mut OsRng).unwrap();
    ballot.sum_proofs[0] = r.sum_proofs[0];
    assert_eq!(
        board.submit(ballot),
        Err(Error::InvalidRowProof { option: 0 })
    );
}

#[test]
fn lying_about_a_tier_count_is_detected() {
    let (sk, mut board) = setup();
    submit(&mut board, &[PREFER, OK, CANT]);
    submit(&mut board, &[CANT, PREFER, IF_NEEDED]);
    let mut tally = decrypt_tally(&sk, &board, &mut OsRng);
    tally.counts[0] += 1;
    assert!(verify_tally(&board, &tally).is_err());
}
