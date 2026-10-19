use rand_core::OsRng;
use whisperpoll_core::*;

fn setup() -> (SecretKey, BulletinBoard) {
    let sk = SecretKey::generate(&mut OsRng);
    let days = ["Fri 24 Oct", "Sat 25 Oct", "Sun 26 Oct"]
        .map(String::from)
        .to_vec();
    let poll = Poll::new("team-dinner", days, BallotKind::Approval, sk.public_key());
    (sk, BulletinBoard::new(poll))
}

#[test]
fn end_to_end_tally_is_correct_and_verifiable() {
    let (sk, mut board) = setup();
    let votes = [
        [true, true, false],
        [false, true, false],
        [true, true, true],
        [false, false, true],
    ];
    let receipts: Vec<_> = votes
        .iter()
        .map(|v| {
            board
                .submit(cast_ballot(&board.poll, v, &mut OsRng).unwrap())
                .unwrap()
        })
        .collect();

    let result = decrypt_tally(&sk, &board, &mut OsRng);
    assert_eq!(result.counts, vec![2, 3, 2]);

    // Each voter: my ballot is on the board, and the board's tally checks out.
    for r in &receipts {
        assert!(board.contains(r));
    }
    verify_tally(&board, &result).unwrap();
}

#[test]
fn lying_about_the_result_is_detected() {
    let (sk, mut board) = setup();
    for v in [[true, false, false], [true, true, false]] {
        board
            .submit(cast_ballot(&board.poll, &v, &mut OsRng).unwrap())
            .unwrap();
    }
    let mut result = decrypt_tally(&sk, &board, &mut OsRng);
    result.counts[1] += 1;
    assert_eq!(
        verify_tally(&board, &result),
        Err(Error::InvalidDecryptionProof { cell: 1 })
    );
}

#[test]
fn dropping_a_ballot_after_tally_is_detected() {
    let (sk, mut board) = setup();
    for v in [[true, false, false], [false, true, true]] {
        board
            .submit(cast_ballot(&board.poll, &v, &mut OsRng).unwrap())
            .unwrap();
    }
    let result = decrypt_tally(&sk, &board, &mut OsRng);

    // A board that silently omits the second ballot no longer matches the published result.
    let mut tampered = BulletinBoard::new(board.poll.clone());
    tampered.submit(board.ballots()[0].clone()).unwrap();
    assert!(verify_tally(&tampered, &result).is_err());
}

#[test]
fn ballot_stuffing_is_rejected() {
    let (_, mut board) = setup();
    let pk = board.poll.public_key;
    // Swap in an encryption of 5 "yes" votes, keeping the proof made for a 1.
    let mut stuffed = cast_ballot(&board.poll, &[true, false, false], &mut OsRng).unwrap();
    stuffed.choices[0].ciphertext = pk.encrypt(5, &mut OsRng).0;
    assert_eq!(
        board.submit(stuffed),
        Err(Error::InvalidBallotProof { cell: 0 })
    );
}

#[test]
fn proofs_cannot_be_moved_between_polls() {
    let (_, board) = setup();
    let other_sk = SecretKey::generate(&mut OsRng);
    let other = Poll::new(
        "other",
        board.poll.options.clone(),
        BallotKind::Approval,
        other_sk.public_key(),
    );
    let ballot = cast_ballot(&other, &[true, false, true], &mut OsRng).unwrap();
    assert!(board.poll.verify_ballot(&ballot).is_err());
}

#[test]
fn duplicate_ballot_is_rejected() {
    let (_, mut board) = setup();
    let ballot = cast_ballot(&board.poll, &[true, true, true], &mut OsRng).unwrap();
    board.submit(ballot.clone()).unwrap();
    assert_eq!(board.submit(ballot), Err(Error::DuplicateBallot));
}
