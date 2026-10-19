//! cargo run -p whisperpoll-core --example demo

use rand_core::OsRng;
use whisperpoll_core::*;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn main() {
    let sk = SecretKey::generate(&mut OsRng);
    let days = ["Fri 24 Oct", "Sat 25 Oct", "Sun 26 Oct"].map(String::from).to_vec();
    let mut board = BulletinBoard::new(Poll::new("team-dinner", days, sk.public_key()));

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
        println!("{name:>6} cast a ballot, receipt {}", &hex(&receipt.0)[..16]);
        receipts.push(receipt);
    }

    let result = decrypt_tally(&sk, &board, &mut OsRng);
    println!();
    for (day, count) in board.poll.options.iter().zip(&result.counts) {
        println!("{day}: {count} {}", "█".repeat(*count as usize));
    }

    println!();
    for (r, (name, _)) in receipts.iter().zip(voters) {
        println!("{name:>6}: ballot on board = {}", board.contains(r));
    }
    match verify_tally(&board, &result) {
        Ok(()) => println!("tally verified against all {} ballots", board.ballots().len()),
        Err(e) => println!("tally verification FAILED: {e}"),
    }
}
