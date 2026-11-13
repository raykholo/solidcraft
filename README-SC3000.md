# SolidCraft 3000

SolidCraft 3000 is a parody of generic cloud software, built on SolidCraft. The CAD app still works underneath its
fictional maintenance notices, updates, expired sessions, cloud sync conflicts and outages; there is no cloud and no
account, and saving works as usual. The parody uses original wording and the existing SolidCraft icon, and it does not
imitate or name another company or product. The real SolidCraft is at https://github.com/tangency-labs/solidcraft.

The `sc3000` Cargo feature is **off by default**; a normal build keeps SolidCraft's behaviour.

## Build

Install the prerequisites in [docs/building.md](docs/building.md), including Trunk and the `wasm32-unknown-unknown`
Rust target, then:

```sh
cd apps/solidcraft-web
trunk build --release --features sc3000 --public-url ./
bash ../../packaging/sc3000-note.sh ../../dist/web/index.html   # the parody note with a link to SolidCraft
```

The site lands in `dist/web/`. The live site, https://raykholo.github.io/solidcraft3000/, is built from this branch by
[raykholo/solidcraft3000](https://github.com/raykholo/solidcraft3000). Browser graphics use WebGPU, with `?webgl` for WebGL2; `?sample` opens the sample part.

## What appears

- The title and splash read “SolidCraft 3000” and “Now 100% in the cloud”.
- A slowly orbiting maintenance dot sits below the help button.
- The dated maintenance banner returns 60 seconds after dismissal, with a little
  more padding each time (bounded so it cannot grow over the whole workspace).
- The daily update reaches 99% after two seconds, holds there for eight seconds,
  then shows its fictional changelog. Continue skips it at any time.
- After ten minutes with the tab focused, the pretend session expires. Any text,
  including no text, signs in; dismissing also returns to CAD.
- A successful Save or Save As triggers three seconds of pretend syncing and
  two identical conflict choices. The joke does not delay or alter the real save.
- On load and at ten-minute intervals there is a one-in-twenty chance of a
  five-second outage screen. Continue dismisses it early.
- The fourth successful fillet within a rolling hour queues a dismissible outage
  notice with an original, code-drawn friendly salmon. Fillets still succeed;
  the notice repeats at most once an hour and stores no history on disk.
- Preferences has a “SolidCraft 3000 Cloud” page. Its toggles affect only their
  own checkmarks for the current session.

## Easter eggs (spoilers)

<details>
<summary>Spoilers: seven small, harmless, dismissible jokes</summary>

- **The friendly service salmon** swims across the scheduled-outage screen, swims by as a toast when you type
  `salmon` or `fish` into Search commands, and cameos on the loading splash about one load in twenty (and still
  announces the fourth fillet within an hour).
- **Cloud credits**: the Konami code (up, up, down, down, left, right, left, right, B, A) shows
  “you have 3 credits. Extrudes cost 1. (Just kidding, they're free.)”
- **Uptime**: Help > About says “99.9% (rounded to the nearest 99.9)”.
- **Version**: about one load in eight, About reads “3000.0.0-cloud (build ∞)”.
- **Fake upload**: about one save in four shows a two-second “Uploading to the cloud… 100%… 99%… 100%” bar that does nothing.
- **Tip of the cloud**: the first load each day shows one of four tips, such as “Your files are stored on your own computer.”
- **Offline mode**: going offline shows “Offline mode detected: everything works perfectly”.

Every egg is a bottom-strip note or an About line; none blocks CAD or touches the design. The daily tip adds one more
`sc3000_` `localStorage` day marker.

</details>

## The cloud is fictional

The parody adds no telemetry, network calls, remote account or cloud storage.
The sign-in box accepts anything, including an empty identity. Its temporary
text is cleared on dismissal and is never sent or persisted.
The fake cloud sync choices refer to the same local design. Timed jokes can be
dismissed, and normal CAD work continues underneath.

The once-per-day update and tip markers use browser `localStorage` with an `sc3000_`
prefix. Clearing that marker allows the joke to appear again. These are the only
persistent state added by the parody; SolidCraft's existing browser design
storage continues to work as usual.

## Checks

```sh
cargo test -p solidcraft-web --features sc3000
cargo clippy -p solidcraft-web --target wasm32-unknown-unknown --features sc3000 -- -D warnings
```
