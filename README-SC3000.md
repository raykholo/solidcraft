# SolidCraft 3000

SolidCraft 3000 is a parody of generic cloud software, built on SolidCraft. The CAD app still works underneath its
fictional maintenance notices, updates, expired sessions, cloud sync conflicts and outages; there is no cloud and no
account, and saving works as usual. The parody uses original wording and the existing SolidCraft icon, and it does not
imitate or name another company or product. The real SolidCraft is at https://github.com/tangency-labs/solidcraft.

The `sc3000` Cargo feature is **off by default**; a normal build keeps SolidCraft's behaviour.

Phone number verification gag requested by: **oursland** (cadmpeg). Community Support chat idea: **Luke** (Discord).

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

## Phone verification

Three seconds after the first design opens, a fictional phone verification form queues once per browser session.
It waits for other modals, file pickers and issue-report forms to close before appearing.
“Send code” discards the number without sending anything. Any six ASCII digits verify.
Skip (including Escape or clicking outside) clears both fields and asks again after three minutes.
Verification clears the fields and shows “Phone verified ✅ Expect a call between 2 AM and 4 AM”.
The country and surprise-call checkbox are temporary props; no texts or calls happen.
The footer says: “SolidCraft 3000 is a parody. Nothing you type here is sent or stored.”
Phone and code input never enters network requests, browser storage, design data or logs.

## Drawing PDF incident

In the Drawing workspace, create a drawing and right-click its sheet in the browser tree
(or use the sheet card’s three-dot menu when the Sheets panel is visible).
“PDF (cloud-rendered)” opens the full-window fictional incident desk. “Send report”
shows a local acknowledgement; “Relaunch” dismisses the screen and returns to the
unchanged drawing. The same menu’s SVG and PNG choices export real files locally.
The menu additions and incident state compile only with `sc3000`. The engine’s
`drawing.export` command stays unchanged, including for programmatic callers.

The incident desk uses original code-drawn paper/service artwork and theme tokens;
it sends no report, launches no process and writes no PDF. Each PDF choice repeats
the gag, while SVG and PNG never trigger it.

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

## Usage statistics you can't turn off

SolidCraft's real usage-statistics toast is replaced under `sc3000`: it asks you to [Accept] or [Accept All]
(both only close it), its x says "Remind me in 5 minutes" (it only closes it), and it comes back every run.
The Privacy switch can be turned off, then flips back on by itself with "Your preference has been saved."

None of this is real. `usage_stats::active()` and `may_send()` return false under `sc3000`, `tick_at` counts, keeps
and sends nothing, and the web host hands over no usage HTTP. The switch is UI state and never writes the real setting.

## Community Support

Every Discord link (the title bar pill, Help › Join the Discord…, About and the start page) and Help › Documentation
open “SolidCraft 3000 Community Support”, an in-app chat, instead of a browser tab. Sunny the Support Bot greets you
by asking whether your computer is switched on, then answers whatever you type with escalating non-answers: sign out
of the cloud and back in, attach your designs’ designs and a photo of your monitor, forwarded to the Enterprise Plus
team, marked Solved. Two community members chime in with “same problem here, any update?” and “bump”.

It is all local. The chat makes no request and stores nothing; what you type is gone when the window closes. The top
line says “Parody. Not a real support channel. The real Discord is discord.gg/yTcseBEdeu”, and that link is the only
way out, opened only when clicked. Ko-fi still opens its real page. The tests are in
`crates/ui-egui/src/sc3000_support_tests.rs`.

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
cargo test -p solidcraft-ui-egui --features sc3000 --lib sc3000
cargo clippy -p solidcraft-web --target wasm32-unknown-unknown --features sc3000 -- -D warnings
```
