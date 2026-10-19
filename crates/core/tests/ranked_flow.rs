use rand_core::OsRng;
use whisperpoll_core::*;

fn setup() -> (SecretKey, BulletinBoard) {
    let sk = SecretKey::generate(&mut OsRng);
    let days = ["Fri 24 Oct", "Sat 25 Oct", "Sun 26 Oct"]
        .map(String::from)
        .to_vec();
    let poll = Poll::new("team-dinner", days, BallotKind::Ranked, sk.public_key());
    (sk, BulletinBoard::new(poll))
}

#[test]
fn ranked_tally_gives_positions_and_borda() {
    let (sk, mut board) = setup();
    let rankings = [[1, 0, 2], [1, 2, 0], [0, 1, 2], [2, 1, 0]];
    let receipts: Vec<_> = rankings
        .iter()
        .map(|r| {
            board
                .submit(cast_ranked_ballot(&board.poll, r, &mut OsRng).unwrap())
                .unwrap()
        })
        .collect();

    let tally = decrypt_tally(&sk, &board, &mut OsRng).unwrap();
    verify_tally(&board, &tally).unwrap();
    assert!(receipts.iter().all(|r| board.contains(r)));

    let result = RankedResult::from_tally(&board.poll, &tally).unwrap();
    // positions[option][rank]
    assert_eq!(
        result.positions,
        vec![vec![1, 1, 2], vec![2, 2, 0], vec![1, 1, 2]]
    );
    // Borda with 3 options: 2 / 1 / 0 points.
    assert_eq!(result.borda, vec![3, 6, 3]);
    assert_eq!(result.order(), vec![1, 0, 2]);
}

#[test]
fn invalid_rankings_are_refused_when_casting() {
    let (_, board) = setup();
    for bad in [&[0, 0, 1][..], &[0, 1][..], &[0, 1, 3][..]] {
        assert!(cast_ranked_ballot(&board.poll, bad, &mut OsRng).is_err());
    }
}

#[test]
fn ranking_one_day_twice_is_rejected() {
    let (_, mut board) = setup();
    // Ballot A ranks day 0 first. Graft in a validly-proven cell from ballot B,
    // which ranks day 0 second: day 0 now holds two ranks and day 2 none.
    let a = cast_ranked_ballot(&board.poll, &[0, 1, 2], &mut OsRng).unwrap();
    let b = cast_ranked_ballot(&board.poll, &[1, 0, 2], &mut OsRng).unwrap();
    let mut forged = a.clone();
    forged.choices[1] = b.choices[1].clone();
    assert_eq!(
        board.submit(forged),
        Err(Error::InvalidRowProof { option: 0 })
    );
}

#[test]
fn missing_sum_proofs_are_rejected() {
    let (_, mut board) = setup();
    let mut ballot = cast_ranked_ballot(&board.poll, &[2, 0, 1], &mut OsRng).unwrap();
    ballot.sum_proofs.pop();
    assert_eq!(board.submit(ballot), Err(Error::WrongSumProofCount));
}

#[test]
fn ballot_kinds_cannot_be_mixed() {
    let (sk, board) = setup();
    assert_eq!(
        cast_ballot(&board.poll, &[true, false, true], &mut OsRng),
        Err(Error::WrongBallotKind)
    );

    let approval = Poll::new(
        "team-dinner",
        board.poll.options.clone(),
        BallotKind::Approval,
        sk.public_key(),
    );
    let ballot = cast_ballot(&approval, &[true, false, true], &mut OsRng).unwrap();
    assert!(board.poll.verify_ballot(&ballot).is_err());
}

#[test]
fn lying_about_a_position_count_is_detected() {
    let (sk, mut board) = setup();
    for r in [[0, 1, 2], [2, 1, 0]] {
        board
            .submit(cast_ranked_ballot(&board.poll, &r, &mut OsRng).unwrap())
            .unwrap();
    }
    let mut tally = decrypt_tally(&sk, &board, &mut OsRng).unwrap();
    tally.counts.swap(0, 1);
    assert!(verify_tally(&board, &tally).is_err());
}
