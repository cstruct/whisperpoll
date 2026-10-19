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

## iOS app

`app/` is a Flutter app with the same flow as the web client. Its
cryptography is the same Rust code (`crates/core`, `crates/protocol`), called
through [flutter_rust_bridge](https://cjycode.com/flutter_rust_bridge/) from
`app/rust`. Invite links work in both: open the web link in the app's "Join"
field, or use `whisperpoll://p/<poll>?t=<token>&server=<https://host>`.

```sh
cd app
flutter run                              # on a booted iOS simulator
flutter_rust_bridge_codegen generate     # after changing app/rust/src/api
```

Push notifications: the server pushes to participants who are offline when
they're needed (to vote, or for their phone to do its key or decryption step,
which wakes the app in the background). Without a paid Apple developer
account there is no real push, but the simulator can be driven locally:

```sh
WHISPERPOLL_PUSH=simulator:dev.whisperpoll.whisperpoll mise run start
```

With that, the server delivers pushes to the booted simulator with
`xcrun simctl push` instead of logging them. Real delivery needs an APNs key
and the app's push entitlement.

## Deploying to Fly.io

The repo has a `Dockerfile` and `fly.toml`. `fly launch`'s source scanner
can't handle the Cargo workspace, so create the app and deploy directly:

```sh
fly apps create <your-app-name>   # then set `app` in fly.toml to match
fly deploy --ha=false
```

`--ha=false` keeps it to a single machine. Polls are kept in memory only, so
a second machine would split polls between them, and every deploy or restart
clears all polls.

## Tests

```sh
cargo test
```
