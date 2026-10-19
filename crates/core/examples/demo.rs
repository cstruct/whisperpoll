//! cargo run -p whisperpoll-core --example demo

use rand_core::OsRng;
use whisperpoll_core::*;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn main() {
    approval_poll();
    println!("\n────────────────────────────\n");
    ranked_poll();
    println!("\n────────────────────────────\n");
    tiered_poll();
    println!("\n────────────────────────────\n");
    trustee_poll();
}

fn approval_poll() {
    let sk = SecretKey::generate(&mut OsRng);
    let days = ["Fri 24 Oct", "Sat 25 Oct", "Sun 26 Oct"]
        .map(String::from)
        .to_vec();
    let mut board = BulletinBoard::new(Poll::new(
        "team-dinner",
        days,
        BallotKind::Approval,
        sk.public_key(),
    ));

    let voters = [
        ("alice", [true, true, false]),
        ("bob", [false, true, false]),
        ("carol", [true, true, true]),
        ("dave", [false, false, true]),
    ];
    let mut receipts = Vec::new();
    for (name, approvals) in voters {
        let ballot = cast_ballot(&board.poll, &approvals, &mut OsRng).unwrap();
        let receipt = board.submit(ballot).unwrap();
        println!(
            "{name:>6} cast a ballot, receipt {}",
            &hex(&receipt.0)[..16]
        );
        receipts.push(receipt);
    }

    let result = decrypt_tally(&sk, &board, &mut OsRng).unwrap();
    println!();
    for (day, count) in board.poll.options.iter().zip(&result.counts) {
        println!("{day}: {count} {}", "█".repeat(*count as usize));
    }

    println!();
    for (r, (name, _)) in receipts.iter().zip(voters) {
        println!("{name:>6}: ballot on board = {}", board.contains(r));
    }
    match verify_tally(&board, &result) {
        Ok(()) => println!(
            "tally verified against all {} ballots",
            board.ballots().len()
        ),
        Err(e) => println!("tally verification FAILED: {e}"),
    }
}

fn ranked_poll() {
    let sk = SecretKey::generate(&mut OsRng);
    let days = ["Fri 24 Oct", "Sat 25 Oct", "Sun 26 Oct"]
        .map(String::from)
        .to_vec();
    let mut board = BulletinBoard::new(Poll::new(
        "team-dinner-ranked",
        days,
        BallotKind::Ranked,
        sk.public_key(),
    ));

    let voters = [
        ("alice", [1, 0, 2]),
        ("bob", [1, 2, 0]),
        ("carol", [0, 1, 2]),
        ("dave", [2, 1, 0]),
    ];
    for (name, ranking) in voters {
        let ballot = cast_ranked_ballot(&board.poll, &ranking, &mut OsRng).unwrap();
        let receipt = board.submit(ballot).unwrap();
        println!(
            "{name:>6} cast a ranked ballot, receipt {}",
            &hex(&receipt.0)[..16]
        );
    }

    let tally = decrypt_tally(&sk, &board, &mut OsRng).unwrap();
    match verify_tally(&board, &tally) {
        Ok(()) => println!(
            "tally verified against all {} ballots\n",
            board.ballots().len()
        ),
        Err(e) => println!("tally verification FAILED: {e}\n"),
    }

    let result = RankedResult::from_tally(&board.poll, &tally).unwrap();
    println!("{:<12} 1st 2nd 3rd  Borda", "");
    for o in result.order() {
        let p = &result.positions[o];
        println!(
            "{:<12} {:>3} {:>3} {:>3}  {:>5}",
            board.poll.options[o], p[0], p[1], p[2], result.borda[o]
        );
    }
}

fn tiered_poll() {
    const TIERS: [&str; 3] = ["Prefer", "OK", "If needed"];
    let (prefer, ok, if_needed, cant) = (Some(0), Some(1), Some(2), None);

    let sk = SecretKey::generate(&mut OsRng);
    let days = ["Fri 24 Oct", "Sat 25 Oct", "Sun 26 Oct"]
        .map(String::from)
        .to_vec();
    let kind = BallotKind::Tiered { tiers: TIERS.len() };
    let mut board =
        BulletinBoard::new(Poll::new("team-dinner-tiered", days, kind, sk.public_key()));

    let voters = [
        ("alice", [ok, prefer, cant]),
        ("bob", [cant, prefer, prefer]),
        ("carol", [prefer, if_needed, prefer]),
        ("dave", [cant, cant, ok]),
    ];
    for (name, choices) in voters {
        let ballot = cast_tiered_ballot(&board.poll, &choices, &mut OsRng).unwrap();
        let receipt = board.submit(ballot).unwrap();
        println!(
            "{name:>6} cast a tiered ballot, receipt {}",
            &hex(&receipt.0)[..16]
        );
    }

    let tally = decrypt_tally(&sk, &board, &mut OsRng).unwrap();
    match verify_tally(&board, &tally) {
        Ok(()) => println!(
            "tally verified against all {} ballots\n",
            board.ballots().len()
        ),
        Err(e) => println!("tally verification FAILED: {e}\n"),
    }

    let result = TieredResult::from_tally(&board.poll, &tally).unwrap();
    println!(
        "{:<12} {:>6} {:>3} {:>9}  can come  score",
        "", TIERS[0], TIERS[1], TIERS[2]
    );
    for o in result.order() {
        let c = &result.counts[o];
        println!(
            "{:<12} {:>6} {:>3} {:>9}  {:>8}  {:>5}",
            board.poll.options[o], c[0], c[1], c[2], result.available[o], result.score[o]
        );
    }
}

/// No single key holder: alice, bob and carol run a 2-of-3 key ceremony,
/// and any two of them can decrypt the result.
fn trustee_poll() {
    let names = ["alice", "bob", "carol"];
    let ids: Vec<_> = names
        .iter()
        .map(|_| TrusteeIdentity::generate(&mut OsRng))
        .collect();

    // 1. Everyone opens their link and registers a trustee key.
    let regs: Vec<_> = ids
        .iter()
        .map(|i| i.register("team-dinner-trustees", &mut OsRng))
        .collect();
    let ceremony = Ceremony::new("team-dinner-trustees", 2, &regs).unwrap();
    println!("2-of-3 key ceremony between {}", names.join(", "));

    // 2. Everyone deals encrypted shares of their own secret.
    let dealings: Vec<_> = ids
        .iter()
        .map(|i| ceremony.deal(i, &mut OsRng).unwrap())
        .collect();

    // 3. Everyone checks the shares they received.
    let complaints: Vec<_> = ids
        .iter()
        .flat_map(|i| ceremony.check_dealings(i, &dealings, &mut OsRng).unwrap())
        .collect();
    println!("{} complaints", complaints.len());

    // 4. The joint key is public; each trustee keeps their own share.
    let key = ceremony.finalize(&dealings, &complaints).unwrap();
    let shares: Vec<_> = ids
        .iter()
        .map(|i| ceremony.key_share(i, &dealings, &complaints).unwrap())
        .collect();

    let days = ["Fri 24 Oct", "Sat 25 Oct", "Sun 26 Oct"]
        .map(String::from)
        .to_vec();
    let mut board = BulletinBoard::new(Poll::new(
        "team-dinner-trustees",
        days,
        BallotKind::Approval,
        key,
    ));
    for approvals in [[true, true, false], [false, true, true], [true, true, true]] {
        let ballot = cast_ballot(&board.poll, &approvals, &mut OsRng).unwrap();
        board.submit(ballot).unwrap();
    }
    println!("{} ballots cast", board.ballots().len());

    // carol is offline: alice and bob are enough.
    let partials: Vec<_> = shares[..2]
        .iter()
        .map(|s| partial_decrypt(s, &board, &mut OsRng))
        .collect();
    let lonely = combine_tally(&board, &partials[..1]);
    println!("alice alone: {}", lonely.unwrap_err());

    let tally = combine_tally(&board, &partials).unwrap();
    match verify_tally(&board, &tally) {
        Ok(()) => println!("alice + bob decrypted, tally verified\n"),
        Err(e) => println!("tally verification FAILED: {e}\n"),
    }
    for (day, count) in board.poll.options.iter().zip(&tally.counts) {
        println!("{day}: {count} {}", "█".repeat(*count as usize));
    }
}
