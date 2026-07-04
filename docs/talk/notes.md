# Talk notes — "Tiny imperative shell, large functional core" (raw material)

> **Status: uncommitted scratch. NOT the presentation.** This is a collection of
> learnings, evidence, metrics, code, and story beats to draw on when we build
> the actual talk later in **typst + cetz + polylux**. Nothing here is slide
> copy yet — it's the quarry, not the statue. Reorganize freely.

## The talk's three threads (as stated by Felipe)

1. **Tiny imperative shell + large functional core** — the architecture.
2. **The power of Haskell** (and Clash) — the tools.
3. **The benefits of engaging AI in the process** — the method.

The through-line that ties them: *purity is what buys you everything else* —
speed, testability, debuggability, and a small enough impure surface that an
AI + human can build a whole SoC, in the open, with confidence.

## The project in one paragraph (audience context)

**tamal** is an FPGA-based eSPI (Intel Enhanced Serial Peripheral Interface)
exerciser rig that can also be used for compliance and conformance testing — a
programmable controller/target that turns compliance testing into a
reproducible, byte-for-byte deterministic exercise. The gateware is written in
**Clash** (Haskell-to-hardware) targeting a Digilent Arty A7-100T. A host loads
a compiled program over UART; an on-FPGA cycle engine executes it,
drives/samples the eSPI bus with deterministic timing, records every transaction
into a trace ring, and streams it back. Spiritual sibling to `mole` (I2C/I3C).

Key domain insight (worth a slide): **this is not a throughput problem, it's an
external-timing-alignment problem.** eSPI tops out at 66 MHz — slow for an
Artix-7.  The dangerous part is at the pins (turnaround, tri-state,
setup/hold). That maps *perfectly* onto functional-core / imperative-shell:
everything up to the pins can be pure; the timing-critical impurity is a thin
skin.

---

## Thread 1 — Tiny imperative shell, large functional core

**Thesis:** Gary Bernhardt's "Functional Core, Imperative Shell" — usually
pitched for software — applies to *hardware description*, and the payoff is even
bigger because the "shell" is where synthesis, tri-state, and timing live.

### The concrete shape

- The engine is a **pure state transition**:

  ```haskell
  step :: State -> BusIn -> (State, BusOut, Maybe Ring)
  ```

  No clock, no I/O, no signals — just `oldState -> input -> (newState,
  outputs)`.

- A one-line adapter re-associates it so Clash's `mealy` can lift it, leaving
  `step` untouched:

  ```haskell
  stepM s i = let (s', bo, mr) = step s i in (s', (bo, mr))
  engine = mealy stepM initState        -- Signal dom BusIn -> Signal dom (BusOut, Maybe Ring)
  ```

- `Tamal.Top.system` is **pure signal-plumbing** — BRAMs + loader + UART +
  `engine` + a status-LED — over plain `Signal`s, *no BiSignal*. It's fully
  simulatable.

- `Tamal.topEntity` is the **entire impure surface**: bind the 100 MHz clock,
  bind the tri-state `IO[3:0]` pads (`espiPads`, the only `BiSignal` in the
  design), name the ports the XDC binds. That's it.

### The metric that sells it

| | LOC | note |
|---|---|---|
| Impure shell (`src/Tamal.hs`) | **52** | ~15 lines of actual logic; rest is the port type-sig + comments |
| `Tamal.Top` (system + pure helpers) | 113 | pure, cosim-tested |
| **Total gateware `src/`** | **2,438** across 22 files | all pure & property-tested except the shell |
| Tests (`tests/`) | 2,036 | ~1:1 test-to-code |

**The imperative shell is ~2% of the codebase**, and its logic is a dozen
lines. The other 98% is referentially transparent.

### Why it mattered here specifically

- We deliberately pushed **`espiPads` (BiSignal) into the shell** so `system`
  stays plain-`Signal` → the *whole integration* is testable in simulation
  without touching BiSignal's fragile bits. On hardware, `espiPads` lowers to
  real Xilinx IOBUFs; the simulation-only fragility never reaches synthesis.

- The impure shell is the *only* thing "validated by reading the emitted
  Verilog" rather than by a property test.

### Diagram ideas (cetz)

- Concentric rings: big pure core (engine + leaves), thin ring = `system`
  plumbing, hairline outer skin = `topEntity` shell touching the pins. Annotate
  each ring with its LOC and "how it's tested."

- The pipeline as a horizontal dataflow: `host —UART→ loader → instr BRAM →
  engine → eSPI pads`, with `ring BRAM → drain → UART → host` on the
  return. Shade the only impure box (pads/clock binding).

- A "what's testable" overlay: 98% green (property-tested), 2% amber (codegen +
  hardware).

---

## Thread 2 — The power of Haskell / Clash

**Thesis:** the same properties (purity, laziness, types) that make the design
clean make the *tests* absurdly fast and the *bugs* trivially isolable. One
source both **simulates** (pure Haskell) and **synthesizes** (to Verilog).

### Speed — the headline demo

- The full tasty suite — **171 tests**, incl. **~86 hedgehog properties at 100
  generated cases each (≈ 8,600 generated cases)** plus ~99 HUnit assertions,
  *and* a whole-SoC cosim that simulates UART + loader + engine + two BRAMs for
  **~26,000 clock cycles** — runs in **< 700 ms** (`All 171 tests passed
  (0.75s)` typical).

- *Why:* it's pure evaluation. `sampleN n sig` is morally `take n` on an
  infinite lazy list. No process spawn, no VCD, no simulator handshake, no
  I/O. A "hardware simulation" is a list traversal.

- Live-demo candidate: run `cabal test`, let the wall-clock land under a second,
  then point out how many hardware cycles were just simulated.

### Property-based testing (hedgehog)

- Every pure leaf + the engine is tested against a **reference model**, not just
  examples. 100 cases/property, automatic **shrinking** to a minimal
  counterexample.

- Concrete: the `alertSync` bug shrank to the input `[0,0,0]` — the smallest
  case that distinguished "reset artifact" from "real sample."

### Types as the hardware contract

- `BusIn`/`BusOut` records; `Lanes = Vec 4 (Bit, Bit)` (per-lane value+output-enable);
  `Unsigned 10` PC vs `Unsigned 12` ring address (sizes checked at compile time);
  phantom-typed `BiSignalIn 'PullUp dom 1`. The compiler enforces widths and pin
  semantics.

### One source, two targets

- The exact same `system`/`step` code that the test suite *evaluates* is what Clash
  *compiles to Verilog*. `cabal run clash -- Tamal --verilog` emits a synthesizable
  top; `cd hdl && make` → `tamal.bit`.

- `clashi` (the Clash REPL) simulates and can `:verilog <expr>` on demand — you can
  poke a circuit and emit its HDL in the same session.

### Clash "war-story" gotchas (great slide fodder — real, specific, teachable)

1. **`sampleN` cycle-0 reset.** The test harness's `sampleN` asserts the Dom100
   async reset on cycle 0, which *swallows the first input sample* and adds a
   lead reset sample. A 2-flop synchronizer looked "wrong" until we saw it: the
   hardware was correct; the *test* needed to lead with one idle cycle.

2. **BiSignal simulation loopback divergence.** Feeding a component's own
   tri-state drive back into the net it also reads makes Clash's
   `veryUnsafeToBiSignalIn` loopback *diverge* (infinite loop) — and worse, on a
   `Nothing`-driven (idle) net.  Fix: model drive and sample as two
   single-driver nets, and bind test drivers to a throwaway pad (the pad arg is
   only `seq`'d, so it breaks the self-reference). This also *mirrors the
   hardware*: the engine drives XOR samples a lane, never both.

3. **Clash inout lowering.** A `Vec 4` of BiSignals lowers to a plain `input
   [3:0]` — it does **not** fuse into an `inout`. Only *scalar* `BiSignalIn`
   arg + `BiSignalOut` result pairs fuse, one `inout` each → `io0..io3`. Found
   by reading the emitted Verilog port list.

4. **UART back-to-back bytes.** The RX drops truly zero-gap bytes on the
   falling-edge resync; a realistic transmitter leaves inter-byte idle. Found by
   systematic boundary probing (see war stories).

### Diagram ideas (cetz)

- A bar/marker on a log time-axis: "8,600 property cases + 26,000 simulated
  cycles → 0.7 s". Contrast with a typical HDL simulator startup time.

- "`sampleN n` = `take n`": a lazy infinite stream with a window of `n`
  highlighted.

---

## Thread 3 — Engaging AI in the process

**Thesis:** a structured skill-driven workflow (design → plan → TDD) plus a
**human/AI division of labor** produced a whole SoC in the open — with a
committed design spec and plan for *every* piece — and the discipline made the
AI's mistakes surface where they're cheap (in tests) and get caught by TDD.

### The workflow (per piece, fresh session each time)

`brainstorming` → `writing-plans` → `TDD execution` → `finish branch`. Each
piece (BRAM, wire, loader, IOBUF, topEntity) got:

- a committed **design spec** (`docs/superpowers/specs/*-design.md`),

- a committed **TDD implementation plan** (`docs/superpowers/plans/*.md`),

- small, conventional commits (one per red→green slice),

- a feature branch, cosim/codegen gates, then a fast-forward merge.

~92 commits touching the gateware + design docs — every step legible in history.

### Ping-pong TDD (the human/AI contract)

- **AI writes the failing test** and mentors the Clash idioms; **human writes
  the synthesizable `src/`** to green. Refactor together.

- Keeps the human authoring the "real" hardware while the AI handles test
  scaffolding, API research, and codegen mechanics.

- **The striking pattern: the bug was in the AI's *tests* three times running,
  never in the human's Clash.** alertSync (harness reset), BiSignal loopback
  (harness divergence), UART cosim (harness back-to-back bytes) — each time the
  human's synthesizable code was correct, and TDD surfaced the AI's test-harness
  mistake.  Nice inversion of the usual "AI writes buggy code" story.

### AI as a verification/research partner (not a guess machine)

- Plans contained **concrete, compile-ready code**, not placeholders, because
  the AI verified exact APIs against the *pinned* `clash-prelude-1.10.0` source
  (BiSignal semantics, `sampleN`'s documented single-cycle reset, `hasX`/`isX`,
  `Bundle` instances) before writing them.

- When the design raised a "do we need FIFOs?" question, the AI answered with
  evidence from the specs (single clock domain, UART ≪ fabric,
  ring-BRAM-as-buffer) rather than hand-waving — and pushed back where a naive
  answer would've been wrong.

### Systematic debugging with AI

- Used a **root-cause-before-fixes** discipline: read the error, reproduce,
  gather evidence at *component boundaries*, form one hypothesis, test
  minimally.

- The UART bug is the poster child: symptom = "engine never runs." Instead of
  guessing, probed the uart→loader boundary in `clashi` and got hard numbers —
  **12 bytes sent, 10 received, 0 trigger pulses, 1 program write** — which
  pinpointed "trailing TRIGGER frame loses bytes." One more minimal probe (add
  inter-byte idle → 12/12, 1 trigger) confirmed the fix before changing a line
  of the harness.

- This is only cheap *because of purity*: lifting `uart → loader` out into a
  15-line REPL block requires no reproduction harness — every signal is a
  deterministic function of its inputs.

### Bidirectional learning (nice human moment)

- The human learned Clash idioms through the AI's mentoring; the **human taught the AI**
  `:m` (module context) in `clashi`. Genuinely collaborative, not one-directional.

### Honesty / correction beats (keeps it credible)

- The AI **refused to implement on `main` without consent**, flagged a conflict
  between the chosen "subagent-driven" execution and the human's "I write the Clash"
  intent, and corrected its own wrong analysis when a probe contradicted it. Worth
  showing: AI as a disciplined collaborator, not an oracle.

### Diagram ideas (cetz / polylux)

- A ping-pong swimlane: AI lane (red test) ↔ Human lane (green `src/`) ↔ shared
  (refactor/commit), looping.

- A "boundary probe" panel: the pipeline with a magnifying glass on uart→loader
  and the 12/10/0/1 evidence numbers.

- Timeline of a piece: brainstorm → spec (commit) → plan (commit) → red/green×N
  → cosim/codegen gate → merge.

---

## War stories (the narrative engine of the talk)

Each is a self-contained 60–90s story with a setup, a probe, and a punchline. All are
real, from this build.

1. **"The synchronizer that wasn't wrong."** alertSync's property fails on
   `[0,0,0]`.  Reading the model vs the sampled output (via `clashi`) reveals
   `sampleN` swallows the cycle-0 sample under async reset. *Punchline:* the
   hardware was textbook; the test harness had to lead with an idle
   cycle. Bonus: this taught the `clashi` probing workflow.

2. **"The test that ran forever."** The `espiPads` tri-state tests hang — not
   slowly, *forever*. Bisecting the BiSignal loopback (probes A–H) shows the
   divergence appears only when a component's drive feeds back into the net it
   reads. *Punchline:* fix by modeling drive/sample as separate single-driver
   nets — which is exactly what the real engine does (drive XOR sample).

3. **"Engine never runs."** The whole-system cosim: HALT drains nothing, PUT
   never asserts CS#. A boundary probe prints **12 sent / 10 received / 0
   triggers / 1 write** — the trailing TRIGGER frame is losing bytes to
   back-to-back UART framing.  *Punchline:* the RX is fine for a realistic
   transmitter; the harness was sending zero-gap bytes. One idle bit-time
   between bytes → green.

4. **"Reading the Verilog."** The `Vec 4 (BiSignalIn)` top port silently lowers
   to a plain `input [3:0] io` — the tri-state drive vanishes. Switching to four
   scalar BiSignal pairs emits four real `inout` ports. *Punchline:* trust, but
   verify the generated HDL.

Common moral across all four: **purity made the bug reproducible and the
boundary probe cheap; TDD made sure a bug couldn't hide.**

---

## Metrics & artifacts to pull from (verify/refresh when building slides)

- **Test suite:** 171 tests, < 700 ms (`cabal test`). ~86 `testProperty`, ~99
  `testCase` in source (grep overcounts vs the 171 the runner reports — recount
  the exact split for the slide; each property = 100 cases).

- **LOC:** shell 52 (`src/Tamal.hs`); `Tamal.Top` 113; total `src/` 2,438 (22
  files); tests 2,036.

- **Commits:** ~92 touching `hdl/` + design docs — clean conventional-commit
  history, small slices.

- **Design artifacts:** one spec + one plan per piece under
  `docs/superpowers/{specs,plans}/` — good "we designed before we coded"
  evidence, and a source of already-written prose/diagrams.

- **Emitted HDL:** `verilog/Tamal.topEntity/topEntity.v` — the port list (4
  `inout` lanes) is a concrete "one source, real hardware" artifact; consider a
  synth utilization/timing snippet from `make` for a hardware-cred slide.

- **Hardware demo (future):** the status LED encodes Waiting → Running → Done —
  film it on the board as the closer.

## Possible narrative arcs (pick later)

- **A) Problem → shape → payoff.** eSPI = timing-alignment problem → push
  impurity to the pins → reap speed/testability/debuggability → the AI method
  that got us there.

- **B) Three demos.** (1) sub-second full-SoC test run; (2) a `clashi` boundary
  probe live; (3) the LED on real hardware. Weave the three threads through the
  demos.

- **C) War-story spine.** Open with "the bug was never in the hardware code,"
  tell the four stories, and let each one surface one thread (purity, types,
  tooling, method).

## Quotable lines / candidate takeaways (draft, sharpen later)

- "It's not a throughput problem, it's a timing-alignment problem — so put all
  the danger in a 2% skin and make the other 98% pure."

- "A hardware simulation of 26,000 cycles is a `take` on a lazy list. That's why
  it's under a second."

- "Same source: the test suite *evaluates* it, Vivado *synthesizes* it."

- "Purity isn't academic — it's what let us lift `uart → loader` into a 15-line
  REPL probe and read off exactly where the bytes died."

- "Three bugs, three times the fault was in the tests, not the hardware — and
  TDD caught all three."

- "The AI taught me `clashi`; I taught it `:m`."

## TODO before writing the talk

- [ ] Recount exact property/unit split and total generated-case count.
- [ ] Grab a real `cabal test` timing line + a `make` utilization/timing snippet.
- [ ] Pull 2–3 tight code snippets (`step`, `stepM`+`mealy`, the `topEntity` shell,
      one cosim assertion) sized for slides.
- [ ] Decide scope: internal team talk vs. conference; adjust Clash-vs-general-FP depth.
- [ ] Film the LED lifecycle on hardware for the closer (needs on-board bring-up first).
- [ ] Set up the `typst + cetz + polylux` skeleton (own dir under `docs/talk/`).
