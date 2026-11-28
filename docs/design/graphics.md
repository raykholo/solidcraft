# Graphics: never go down because of the GPU

Status: direction approved on 2026-11-27 and 2026-11-28 (section 4). Section 3 says which parts are
on main and which are in progress. This is the place to start for any graphics, GPU or driver work:
it records how other apps on the same stack do it, what we do, why, and what is left.

Related docs: [troubleshooting](../book/src/guide/troubleshooting.md) (what users see),
[crash reports](../crash-reports.md) (how a crash is recorded),
[backdrop blur](backdrop-blur.md) (the one effect that hooks the frame).

## 1. Goal

- **A graphics or driver failure never takes down the app and never loses work.** A driver that
  crashes at start, a device lost mid-session, or a driver that kills the process outright all end
  with the app running, on hardware or on the software renderer, and every design as it was.
- **Mainstream hardware keeps a plain, stock path.** An ordinary iGPU or mid-range card runs
  eframe and wgpu the way they ship. Anything special for one GPU or driver sits behind exact
  detection of that GPU or driver, and nobody else reaches it.

These two pull in different directions. Every special case we added for one machine risked the
path that already worked for everyone else (section 4 has the history). So robustness comes from
*around* the stock path (a probe before it, recovery and a restart after it), not from changing
the path itself.

## 2. How other GPU apps do it

Each fact below links the public source it was checked against on 2026-11-28. Where I could not
check something, it says "we believe" or is left out.

### Firefox (WebGPU on wgpu)

- **WebGPU is wgpu, and on Windows it is D3D12 alone.** Firefox 141 shipped WebGPU on Windows,
  built on wgpu ([Mozilla gfx blog](https://mozillagfx.wordpress.com/2025/07/15/shipping-webgpu-on-windows-in-firefox-141/)).
  The wgpu server asks for `Backends::DX12` on Windows and `Backends::PRIMARY` elsewhere, unless the
  `dom.webgpu.wgpu-backend` pref names others
  ([gfx/wgpu_bindings/src/server.rs](https://searchfox.org/mozilla-central/source/gfx/wgpu_bindings/src/server.rs)).
  So no Vulkan or GL driver is loaded on Windows unless someone asks for it.
- **A separate GPU process.** Compositing (WebRender) and WebGPU run in a GPU process on some
  platforms; "if the GPU process crashes (a bug or a driver issue) we can simply restart it… and the
  browser itself doesn't crash"
  ([Rendering overview](https://firefox-source-docs.mozilla.org/gfx/RenderingOverview.html)).
  After more unstable launches than `layers.gpu-process.max_restarts`, Firefox disables the GPU
  process and falls back from acceleration (`MaybeDisableGPUProcess`,
  `gfxPlatform::FallbackFromAcceleration`), and composites in the parent process
  ([GPUProcessManager.cpp](https://searchfox.org/mozilla-central/source/gfx/ipc/GPUProcessManager.cpp)).
- **A start-up test in a child process (Linux).** Firefox starts a separate probe binary
  (`gfxtest`, the old "glxtest") that creates a GL context and reports the driver back over a pipe,
  with a 4-second timeout. If it fails or times out: "Failed to get GPU info from glxtest. Fallback
  to SW rendering!"
  ([widget/gtk/GfxInfo.cpp](https://searchfox.org/mozilla-central/source/widget/gtk/GfxInfo.cpp)).
- **Crash guards.** Before risky driver set-up, Firefox writes a marker (a pref and a lock file). If
  the marker is still there at the next start, that start disables the accelerated feature "until
  the environment changes" (driver, device or app version)
  ([DriverCrashGuard.cpp](https://searchfox.org/mozilla-central/source/gfx/src/DriverCrashGuard.cpp)).
- **Software rendering as the floor.** Software WebRender renders without the GPU, for bad drivers
  and uncommon set-ups
  ([gfx newsletter 54](https://mozillagfx.wordpress.com/2020/12/17/moz-gfx-newsletter-54-2/)).
- **Blocklists keyed on exact hardware.** Downloadable and built-in blocklists match GPU, driver
  version and OS per feature
  ([wiki](https://wiki.mozilla.org/Blocklisting/Blocked_Graphics_Drivers)).

### Chrome (Dawn)

- **GPU process with a crash budget.** On most platforms, after 3 recent GPU process crashes
  (6 on ChromeOS) Chrome falls back to the next GPU mode, "like disabling hardware acceleration or
  all GL"; one crash is forgiven every 5 minutes. Failing to initialise the GPU also falls back to
  software
  ([gpu_process_host.cc](https://chromium.googlesource.com/chromium/src/+/main/content/browser/gpu/gpu_process_host.cc)).
- **One modern API per platform.** Dawn prefers D3D12 on Windows and Metal on macOS, and Vulkan
  where there is no other modern API
  ([Dawn support](https://dawn.googlesource.com/dawn/+/HEAD/docs/support.md)). WebGPU shipped on
  ChromeOS with Vulkan, Windows with D3D12, and macOS in Chrome 113
  ([Chrome WebGPU overview](https://developer.chrome.com/docs/web-platform/webgpu/overview)).
- **Workarounds keyed on exact hardware.** `gpu_driver_bug_list.json` applies a named workaround
  only to a matching vendor, device and driver version
  ([gpu_driver_bug_list.json](https://chromium.googlesource.com/chromium/src/+/main/gpu/config/gpu_driver_bug_list.json)).
- **Device loss is recoverable by design.** In WebGPU a lost device resolves `GPUDevice.lost`; the
  page asks for a new device and re-creates its resources
  ([MDN](https://developer.mozilla.org/en-US/docs/Web/API/GPUDevice/lost)).

### Bevy and stock eframe (wgpu apps like us)

- **wgpu's default is every backend.** `Backends::default()` is `Backends::all()`
  ([wgpu-types 30.0.2](https://docs.rs/wgpu-types/30.0.2/x86_64-unknown-linux-gnu/src/wgpu_types/backend.rs.html)),
  so a stock instance on Windows initialises Vulkan, D3D12 and GL drivers in one process.
- **Bevy** uses `Backends::all()` on native, overridable with `WGPU_BACKEND`
  ([bevy_render settings.rs](https://github.com/bevyengine/bevy/blob/main/crates/bevy_render/src/settings.rs)).
  On a lost device its default error handler quits the app; an app can install a handler that
  recreates the renderer instead
  ([error_handler.rs](https://github.com/bevyengine/bevy/blob/main/crates/bevy_render/src/error_handler.rs)).
- **Stock eframe** (`WgpuConfiguration::default()`) takes wgpu's backends (or `WGPU_BACKEND`) and
  its adapter choice. Its painter owns the device and surface, so an app on stock eframe can't
  replace a lost device; we believe the only stock answer is to restart the app.

### What we take from each

| From | We take | Why |
|---|---|---|
| Firefox | D3D12 alone on Windows | Same library as us; it keeps the Vulkan and GL drivers out of the process unless chosen |
| Firefox | A probe child before the window | A driver that crashes in the probe can't take the app with it |
| Firefox, Chrome | Software rendering as the floor, after a failed start or repeated losses | It doesn't depend on the driver at all |
| Firefox (crash guard) | A marker before graphics, read at the next start | A crash we can't catch still changes what the next start does |
| Chrome, WebGPU | Recreate the device on loss instead of restarting | The spec and every browser treat loss as recoverable |
| Chrome, Firefox | Workarounds keyed on exact vendor/device/driver | The ordinary path stays stock |
| Bevy, eframe | Stock setup on macOS and Linux | Nothing there needs more yet |
| Firefox, Chrome | A separate GPU process: **not yet** | Large, and we don't yet know it's needed (section 5.1) |

## 3. Our design today

| Part | Where | Status |
|---|---|---|
| Stock eframe and egui-wgpu setup | `apps/solidcraft/src/main.rs` (`wgpu_options`) | on main ([#23352](https://github.com/tangency-labs/solidcraft/pull/23352)) |
| D3D12 alone on Windows in Automatic | `startup::window_backends` | in progress ([T-2962](https://tickets.solidcraft.io/t/2962)) |
| Start-up probe child process | `apps/solidcraft/src/gpu_probe.rs` | in progress |
| Software after one failed start | `startup::SafeGraphics`, `startup::begin` | on main |
| Crash recording before graphics | `solidcraft_engine::crash`, `native_crash` | on main |
| Device recovery on our runner | `crates/window-gpu` | planned (the runner is on main, opt-in) |
| Watchdog restart after a hard crash | | in progress |
| Targeted fixes behind detection | `crates/viewport-gpu` (`window_device_request`) | on main ([#23466](https://github.com/tangency-labs/solidcraft/pull/23466)) |
| vendor/egui freeze | `vendor/egui` | in force |

### 3.1 Stock eframe and egui-wgpu

The window uses `WgpuConfiguration::default()`: wgpu picks the adapter (high performance,
`WGPU_POWER_PREF` aside), with wgpu's default limits, flags and present mode. Only the user's
choices change it: Preferences › Graphics › Graphics API (`graphics.json`'s `backend`) or
`WGPU_BACKEND`, and Preferences' adapter or `WGPU_ADAPTER_NAME`.

The vendored egui-wgpu patches were removed one at a time
([#23362](https://github.com/tangency-labs/solidcraft/pull/23362),
[#23367](https://github.com/tangency-labs/solidcraft/pull/23367),
[#23368](https://github.com/tangency-labs/solidcraft/pull/23368),
[#23370](https://github.com/tangency-labs/solidcraft/pull/23370),
[#23371](https://github.com/tangency-labs/solidcraft/pull/23371),
[#23374](https://github.com/tangency-labs/solidcraft/pull/23374)). The lost-device flag and error
capture now live in our own crates.

### 3.2 D3D12 alone on Windows

Automatic on Windows names D3D12 as the instance's only backend, as Firefox does. wgpu 30
initialises every backend in the instance and enumerates Vulkan adapters first, so a stock
instance loaded Intel's Vulkan driver (`igvk64.dll`) even on machines where D3D12 works, and on
some Intel iGPUs that driver crashed the process before any window
([T-2962](https://tickets.solidcraft.io/t/2962)). Vulkan and OpenGL load only when the user picks
them. macOS and Linux keep eframe's own set.

Known costs, to watch:
- AMD Ryzen (Vega) iGPUs had device losses and freezes on D3D12 before we moved to Vulkan first
  ([#17967](https://github.com/tangency-labs/solidcraft/pull/17967) lists them). Sections 3.3,
  3.6 and 3.7 are what catch that now; Vulkan stays one click away in Preferences.
- D3D12 shader compiles were slow on a Radeon 780M
  ([#4860](https://github.com/tangency-labs/solidcraft/issues/4860)).
- The GPU path tracer runs on Vulkan, so it opens a second device of its own on Windows.

### 3.3 Start-up probe child process

Before the window and its device exist, the app runs itself as `solidcraft --gpu-probe <api>`.
The child makes exactly the device the window would get (the same `wgpu_options`), draws a small
picture off screen and reads it back. If it crashes, hangs past a timeout, or reads back the
wrong colour, this launch opens on the software renderer and says why over the view, with
**Try again**.

- The child opens no window, installs no crash handler and writes nothing of the user's.
- The result is kept (`gpu-probe.json` beside `graphics.json`) for this app version and these
  drivers, so later launches don't wait. An app or driver update, a failed launch, a lost device or
  a different adapter in the window tests again.
- `SOLIDCRAFT_GPU_PROBE=off` skips it, `=always` runs it every launch.
- A probe that can't run at all (no program to start) is not a failure.

### 3.4 Software after one failed start

`startup::begin` counts launches that died before the window opened (a marker file). After **one**
such launch, or a device loss that wasn't out of memory, the next launch is in safe graphics mode
(`startup::SafeGraphics`): the software renderer (WARP on Windows, through D3D12 alone; lavapipe or
llvmpipe elsewhere), no anti-aliasing and no optional effects. **Try full graphics** goes back.
After `FAILED_STARTS_FOR_SAFE_MODE` (2) in a row, full safe mode also uses default settings.

### 3.5 Crash recording before graphics

`solidcraft_engine::crash::install` and the native handlers (`native_crash` on Windows,
`native_crash_macos`) are installed before the window and device are made, and
`crash::phase` names the start-up step. A driver crash at device creation is then a crash report at
the next start, with the faulting module and offset on Windows and macOS, the failed run's GPU and
its log ([#23386](https://github.com/tangency-labs/solidcraft/pull/23386),
[crash reports](../crash-reports.md)).

### 3.6 In-process device recovery on our runner

Today a lost device closes the app, keeping unsaved changes for recovery, and starts it again
(`apps/solidcraft/src/device_lost.rs`); a second loss soon after starts on software. That is the
most stock eframe allows: its painter owns the device and surface.

Our own runner (`crates/window-gpu`: a copy of egui-wgpu 0.37's painter on its public API, plus
`runner`) is on main behind `SOLIDCRAFT_WINDOW_RUNNER=own`. Because we own the painter there, it can
do what browsers do:

1. On loss, request a new device from the same adapter (or the next one).
2. Rebuild egui's renderer, the viewport's pipelines and its uploads.
3. Keep drawing. After 3 losses in one session, switch that session to the software renderer.
4. Restart only if recovery itself fails.

### 3.7 Watchdog restart after a hard crash

A driver that kills the process (an access violation inside the driver) can't be caught in
process. A small watchdog process outlives the app: when the app dies without a clean exit, it
starts it again at once, back into the open designs from recovery, with nothing lost. A repeat
crash within a short window starts on the software renderer.

### 3.8 Targeted fixes behind exact detection

A fix for one GPU or driver is allowed only behind detection of exactly that GPU or driver, in our
own code, never as a patch to wgpu or egui. Examples on main:

- Old drivers whose limits are below wgpu's defaults get the adapter's own limits; everyone else
  gets the stock request.
- 2x and 8x anti-aliasing where the adapter has them
  (`TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES`).
- Pop-out windows present without waiting for vsync where the surface can (our painter).
- The blur behind panels composes in our own painter, not in an egui-wgpu patch
  ([#23469](https://github.com/tangency-labs/solidcraft/pull/23469)).

### 3.9 The vendor/egui freeze

No new graphics changes in `vendor/egui`, and no patched graphics libraries at all (wgpu,
wgpu-hal, gpu-allocator and the like). A real bug in one of them goes upstream, and we take the fix
from a release. The patches still in `vendor/egui` are on their way out (section 5.2).

## 4. Decision log

- **2026-11-24:** Remove the vendored graphics patches; never patch wgpu or its relatives. Fixes go
  upstream.
- **2026-11-24:** Graphics PRs leave the ordinary-hardware path unchanged; a niche fix only behind
  exact detection.
- **2026-11-27:** Use wgpu the way the big users do: D3D12 by default on Windows, stock egui-wgpu.
- **2026-11-27:** Tear it down to stock: `WgpuConfiguration::default()`, no backend ladder, no
  per-vendor table. Kept: the user's API choice and the software fallback.
- **2026-11-27:** Keep the targeted fixes, behind detection in our own code; the blur stays, in our
  painter.
- **2026-11-28:** Make it work for everybody: Windows starts D3D12 alone; one failed start goes to
  software; crash recording before graphics.
- **2026-11-28:** Never crash because of graphics: the probe child, in-process recovery and the
  watchdog restart. A separate GPU process is deferred until the crash data says we need it.

The history behind them:

- **D3D12 first broke AMD Vega iGPUs.** From 0.12.0, almost every device-lost or freeze-on-open
  report was an AMD Ryzen iGPU on D3D12, so
  [#17967](https://github.com/tangency-labs/solidcraft/pull/17967) (2026-11-16) moved Windows to
  Vulkan first.
- **Vulkan first broke Intel.** Intel's Vulkan driver crashed at device creation, before any
  window: about 72 reports from 0.44.1 to 0.50.0, on HD 520 to Iris Xe. When the old API ladder went
  away in [#21833](https://github.com/tangency-labs/solidcraft/pull/21833), those users landed on
  the software renderer instead of hardware D3D12, which
  [#22943](https://github.com/tangency-labs/solidcraft/pull/22943) fixed.
- **An RX 6800 lost its device while idle** on 0.51.1. The render audit found no long submissions
  or unbounded dispatches; the likeliest cause is wgpu marking a device lost whenever a Vulkan call
  returns a code it doesn't expect (section 5.3).
- **[T-2962](https://tickets.solidcraft.io/t/2962):** an Intel Iris Xe crashed at start on 0.51.2,
  before the window opened, because the stock backend set still loaded the Intel Vulkan driver. That
  is what moved Windows to D3D12 alone.

## 5. Deferred work, and how to pick it up

### 5.1 A separate GPU process (Firefox-style)

**When to revisit.** Once the probe, recovery and watchdog ship, count hard driver crashes
*after* the window opened: crash reports whose faulting module is a graphics driver, during a
session, which the watchdog had to restart. Revisit if, over two releases in a row, they reach
**5 distinct reports a week**, or any report says work was lost. Below that, the watchdog restart
is cheaper and good enough.

**Sketch.**
- A `solidcraft --gpu-process` child owns the wgpu instance, adapter and device. The app process
  keeps the model, the UI state and the event loop.
- The window surface: the child renders into a shared texture (DXGI shared handle on Windows,
  IOSurface on macOS, dma-buf on Linux) that the app presents, or the child presents to the app's
  window handle directly.
- egui's primitives and the viewport's draw lists cross by shared memory each frame; meshes upload
  once and are referenced by id.
- When the child dies, the app starts a new one and re-sends the meshes; the same crash budget as
  Chrome's (a few recent crashes, then software) decides when to stop.

**Risks.**
- A frame's latency and copy cost, measured on an ordinary iGPU at 60 Hz before anything else.
- Three shared-surface paths, one per OS, each with its own driver bugs.
- Paint callbacks in `viewport-egui` and the path tracer assume one device in one process; both
  would move to the child.
- The wasm build has no processes; it stays on the in-process path.

### 5.2 R5: our runner as the default

Today eframe runs the window and `SOLIDCRAFT_WINDOW_RUNNER=own` runs ours. Making ours the default:

- **Buys** device recovery in process (3.6), which stock eframe can't do. It also lets the last
  egui-wgpu patches (`winit.rs` clamp, acquire-first, frame compositor hooks; `Callback::downcast_ref`)
  and eframe patches go, so `vendor/egui` becomes stock.
- **Before it ships:** startup runs green on Windows, macOS and Linux on the own runner; window
  handle uses that still need eframe's `Frame` (Windows work-area fit, X11 bring-forward;
  [#23748](https://github.com/tangency-labs/solidcraft/pull/23748),
  [#23766](https://github.com/tangency-labs/solidcraft/pull/23766)); a look at the first frame,
  which was about 2 s slower on WARP in the Windows startup smoke; a before/after crash and frame
  time note on ordinary hardware.
- It changes the default path for everyone, so it goes out through the tester ring first.

### 5.3 Upstream issues to file or track

wgpu:
- **Log the real `VkResult` and the call** when wgpu-hal maps an unexpected Vulkan result to a lost
  device ("Unexpected error variant (driver implementation is at fault)"). Today the code is never
  logged, so we can't tell which call failed.
- **Out of memory outside resource creation** is treated as device loss. We believe this turns
  memory pressure on iGPUs into losses.
- **A swapchain semaphore panic** on close after a lost device.

egui (drafts, checked against egui `main` on 2026-11-27):
- Commands recorded in `CallbackTrait::prepare` are lost when the surface acquire fails (acquire
  first).
- `Renderer::update_buffers` panics when a staging buffer can't be made (skip the frame instead).
- An MSAA count the device can't do panics; clamp it to what the formats support.
- A surface larger than `max_texture_dimension_2d` panics in `configure`; clamp it.
- Renderer creation errors go to the uncaptured-error handler; return them.
- Immediate viewports each wait for vsync in one frame.

Each one we land upstream is one less thing in our code.

### 5.4 Smaller follow-ups

- Integrated GPUs get a lighter default look (ambient occlusion off or fewer samples, no
  supersampling); discrete GPUs unchanged.
- Thumbnails submit on the window's queue from worker threads; give GPU work one owner so they
  don't race the frame.
- A device loss and a failed start are now reportable ([#23386](https://github.com/tangency-labs/solidcraft/pull/23386));
  keep the failed run's log when several starts fail in a row.

## 6. How to contribute a graphics fix

1. **Reproduce first** on a released build, on the machine that fails, with its OS, GPU, driver
   and API ([repro-first](../../.agents/skills/solidcraft-repro-first/SKILL.md)).
2. **The ordinary path stays unchanged.** An iGPU or a mid-range card at 60 Hz gets byte-for-byte
   the same setup as before.
3. **Gate it on exact detection** of the vendor, device or driver you saw, in our code, never in
   `vendor/egui` or a patched wgpu. Name the condition in the code and the PR.
4. **Measure it on ordinary hardware**, not only on yours, and say which tests cover the path
   everyone else takes. GPU test changes pass on our CI runners in the PR's own run.
5. **No new preference** to work around a bug, and a change to a default is a maintainer's call
   ([preferences](../../.agents/skills/solidcraft-preferences/SKILL.md)).
6. **Never crash** ([crash safety](../../.agents/skills/solidcraft-crash-safety/SKILL.md)): if the
   GPU can't do something, say so and carry on with what it can.

A PR that changes the default graphics path without these goes on hold with a note saying which
one is missing. That isn't a rejection: add the evidence or the gate, and it moves.
