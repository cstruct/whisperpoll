# whisperpoll

Find a day that works for everyone, with votes nobody can read (not even
the server) and a result anyone can check.

- `crates/core`: the cryptography. Exponential ElGamal over Ristretto255
  with a homomorphic tally, zero-knowledge ballot proofs, dealerless key
  generation, and threshold decryption.
- `crates/protocol`: messages between server and browser.
- `crates/server`: an axum websocket relay and public bulletin board. It
  never holds a key.
- `crates/web`: the Leptos (CSR) client. All cryptography runs in the
  browser.

## Running

With [mise](https://mise.jdx.dev) (which also installs the pinned trunk):

```sh
mise trust          # once
mise run start      # build, then run server + client watcher in the background
mise run logs       # follow both logs
mise run status
mise run stop
```

Then open <http://localhost:3000>. The web client rebuilds itself when you
change it (just reload the page). After changing the server, `mise run restart`.
Logs and pid files live in `target/dev/`.

Without mise:

```sh
cd crates/web && trunk build && cd ../..
cargo run -p whisperpoll-server
```

To try a poll alone, open each invite link in its own tab. Keys are stored
per invite, so one browser can play every participant.

## Tests

```sh
cargo test
```
