# Tamal topEntity (integration) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the whole tamal design to the Arty A7-100T pins — a cosim-testable `Tamal.Top.system` (BRAMs + loader + UART + engine `mealy stepM initState` + status LED) plus a thin `Tamal.topEntity` shell (clock + `espiPads` + named ports) — and extend the XDC, retiring the heartbeat.

**Architecture:** All BiSignal lives in the shell so `system` is plain-`Signal` and testable end-to-end via a UART line-level cosim (serialize a `Tamal.Wire` control frame → run → decode `txLine` → assert the drained trace). Pure helpers (`stepM`, `ringWrite`, `rigState`, `ledPattern`) are hedgehog-tested; the shell is validated by the `--verilog` codegen gate.

**Tech Stack:** Clash 1.10, Haskell, `cabal`, tasty + tasty-hedgehog + tasty-hunit, fourmolu, Vivado (XDC).

**Division of labor (ping-pong TDD):** steps tagged **(assistant)** are the failing tests, cabal, and docs; steps tagged **(author)** are the synthesizable Clash under `src/`.

**Spec:** `docs/superpowers/specs/2026-07-03-tamal-topentity-design.md`. All commands run from `hdl/`.

---

## Task 1: `Tamal.Top` skeleton + `Test.Top` wiring

Establishes the module, cabal exposure, and an empty `Test.Top` so later tasks are clean red→green loops.

**Files:**
- Create: `hdl/src/Tamal/Top.hs`
- Create: `hdl/tests/Test/Top.hs`
- Modify: `hdl/tamal.cabal`, `hdl/tests/unittests.hs`

- [ ] **Step 1 (author): create the `Tamal.Top` skeleton with `undefined` bodies**

`hdl/src/Tamal/Top.hs`:

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
{-# LANGUAGE NumericUnderscores #-}

{- |
The tamal design minus pin binding (design doc 2026-07-03-tamal-topentity-design.md):
'system' wires the BRAMs, loader, UART, and engine (@mealy stepM initState@) over
plain 'Signal's — no 'BiSignal', so the whole integration is cosim-testable. The
'Tamal' shell binds the clock + 'espiPads' + pins around it. Pure helpers 'stepM',
'ringWrite', 'rigState', and 'ledPattern' are hedgehog-tested.
-}
module Tamal.Top
  ( system
  , stepM
  , ringWrite
  , RigState (..)
  , rigState
  , ledPattern
  ) where

import Clash.Prelude

import Tamal.Bus.Serdes (Lanes)
import Tamal.Engine (BusIn (..), BusOut (..), Ring (..), State, initState, step)

-- | The mealy adapter: re-associates 'step' so it lifts with 'mealy'.
stepM :: State -> BusIn -> (State, (BusOut, Maybe Ring))
stepM = undefined

-- | Project the engine's ring write to the BRAM write-port tuple.
ringWrite :: Maybe Ring -> Maybe (Unsigned 12, BitVector 32)
ringWrite = undefined

-- | Lifecycle state shown on the status LED.
data RigState = Waiting | Running | Done
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

-- | Pure status -> LED level over a free-running counter.
ledPattern :: RigState -> Unsigned 26 -> Bit
ledPattern = undefined

-- | Derive the LED state from the running latch + halted flag.
rigState :: Bool -> Bool -> RigState
rigState = undefined

-- | The whole design minus pin binding.
system ::
  (HiddenClockResetEnable dom) =>
  Signal dom Bit ->          -- uart RX line
  Signal dom (Vec 4 Bit) ->  -- ioIn
  Signal dom Bit ->          -- alertIn
  ( Signal dom Bit           -- uart TX line
  , Signal dom Lanes         -- lanesOut
  , Signal dom Bit           -- csOut
  , Signal dom Bit           -- sckOut
  , Signal dom Bit           -- rstOut
  , Signal dom Bit           -- led
  )
system = undefined
```

- [ ] **Step 2 (assistant): create the `Test.Top` stub**

`hdl/tests/Test/Top.hs`:

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

module Test.Top (tests) where

import Test.Tasty

tests :: TestTree
tests = testGroup "Top" []
```

- [ ] **Step 3 (assistant): cabal wiring**

In `hdl/tamal.cabal`, add to the library `exposed-modules` (after `Tamal.Io`):

```
    Tamal.Io
    Tamal.Top
```

and to the test-suite `other-modules` (after `Test.Io`):

```
    Test.Io
    Test.Top
```

- [ ] **Step 4 (assistant): runner wiring**

In `hdl/tests/unittests.hs`, add the import (alphabetical, after `import qualified Test.Mem`):

```haskell
import qualified Test.Top
```

and add it to the `tests` list (after `Test.Io.tests`):

```haskell
    , Test.Io.tests
    , Test.Top.tests
```

- [ ] **Step 5: build the suite**

Run: `cabal test 2>&1 | tail -5`
Expected: PASS — suite compiles, empty `"Top"` group present.

- [ ] **Step 6: commit**

```bash
git add hdl/src/Tamal/Top.hs hdl/tests/Test/Top.hs hdl/tamal.cabal hdl/tests/unittests.hs
git commit -m "feat(hdl): Tamal.Top skeleton + Test.Top wiring (topEntity piece)"
```

---

## Task 2: pure helpers (`stepM`, `ringWrite`, `rigState`, `ledPattern`)

**Files:**
- Modify: `hdl/tests/Test/Top.hs`, `hdl/src/Tamal/Top.hs`

- [ ] **Step 1 (assistant): write the pure-helper tests**

Overwrite `hdl/tests/Test/Top.hs`:

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
{-# LANGUAGE NumericUnderscores #-}

module Test.Top (tests) where

import Clash.Prelude
import qualified Hedgehog as H
import qualified Hedgehog.Gen as Gen
import qualified Hedgehog.Range as Range
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)

import Tamal.Engine (BusIn (..), Ring (..), initState, step)
import Tamal.Top (RigState (..), ledPattern, rigState, ringWrite, stepM)
import Test.Gen (genBit, genWord)

-- | A random BusIn: instr word, four sampled IO bits, ALERT#, start.
genBusIn :: H.Gen BusIn
genBusIn =
  BusIn
    <$> genWord
    <*> ((\a b c d -> a :> b :> c :> d :> Nil) <$> genBit <*> genBit <*> genBit <*> genBit)
    <*> genBit
    <*> Gen.bool

tests :: TestTree
tests =
  testGroup
    "Top"
    [ -- stepM: re-associates step, nothing more
      testProperty "stepM = step re-associated" $ H.property $ do
        i <- H.forAll genBusIn
        let (s', bo, mr) = step initState i
        stepM initState i H.=== (s', (bo, mr))
    , -- ringWrite: unwrap the Ring record to the BRAM tuple
      testCase "ringWrite Nothing = Nothing" $
        ringWrite Nothing @?= Nothing
    , testProperty "ringWrite (Just Ring) = Just (addr,data)" $ H.property $ do
        a <- H.forAll (fromIntegral <$> Gen.int (Range.linear 0 4095))
        d <- H.forAll genWord
        ringWrite (Just (Ring a d)) H.=== Just (a, d)
    , -- rigState truth table
      testCase "rigState: halted -> Done (regardless of running)" $ do
        rigState False True @?= Done
        rigState True True @?= Done
    , testCase "rigState: running & not halted -> Running" $
        rigState True False @?= Running
    , testCase "rigState: idle -> Waiting" $
        rigState False False @?= Waiting
    , -- ledPattern: Done solid; Running faster than Waiting
      testCase "ledPattern Done is solid on" $ do
        ledPattern Done 0 @?= high
        ledPattern Done maxBound @?= high
    , testCase "ledPattern Waiting toggles on bit 25 (slow)" $ do
        ledPattern Waiting 0 @?= low
        ledPattern Waiting 0x2000000 @?= high -- 2^25
    , testCase "ledPattern Running toggles on bit 22 (faster than Waiting)" $ do
        ledPattern Running 0 @?= low
        ledPattern Running 0x400000 @?= high -- 2^22
        ledPattern Waiting 0x400000 @?= low -- same count: Waiting still low => Running is faster
    ]
```

- [ ] **Step 2: run to verify red**

Run: `cabal test 2>&1 | tail -30`
Expected: FAIL — the `Top` cases error on `undefined` (`stepM`/`ringWrite`/`rigState`/`ledPattern`).

- [ ] **Step 3 (author): implement the pure helpers**

In `hdl/src/Tamal/Top.hs`, replace the four `undefined` helper bodies:

```haskell
stepM s i = let (s', bo, mr) = step s i in (s', (bo, mr))

ringWrite = fmap (\(Ring a d) -> (a, d))

ledPattern Waiting c = msb c
ledPattern Running c = msb (shiftL c 3)
ledPattern Done _ = high

rigState _ True = Done
rigState running False = if running then Running else Waiting
```

- [ ] **Step 4: run to verify green**

Run: `cabal test 2>&1 | tail -20`
Expected: PASS — all `Top` pure-helper cases green (`system` still `undefined`, but no test forces it yet).

- [ ] **Step 5: format and commit**

```bash
make format
git add hdl/tests/Test/Top.hs hdl/src/Tamal/Top.hs
git commit -m "feat(hdl): Tamal.Top pure helpers (stepM, ringWrite, rigState, ledPattern)"
```

---

## Task 3: `system` + whole-system cosim

The centerpiece. The cosim serializes a `Tamal.Wire` control frame onto `rxLine`, runs load→run→drain, and decodes `txLine`.

**Files:**
- Modify: `hdl/tests/Test/Top.hs`, `hdl/src/Tamal/Top.hs`

- [ ] **Step 1 (assistant): add the cosim harness + tests**

Extend the imports in `hdl/tests/Test/Top.hs` to add:

```haskell
import qualified Data.List as L
import Tamal.Bus.Serdes (Lanes)
import Tamal.Domain (Dom100)
import Tamal.Isa (Instr (..), encode)
import Tamal.Top (system)
import Tamal.Uart (uart)
import Tamal.Uart.BaudGen (oversampleTick)
import Tamal.Uart.Rx (uartRx)
import Tamal.Wire (ControlMsg (..), decodeResult, encodeControl)
```

Insert this harness block above `tests`:

```haskell
-- | 100 MHz / 2 Mbaud = 50 system cycles per UART bit.
cyclesPerBit :: Int
cyclesPerBit = 50

-- | Serialize bytes to a UART line waveform: 8N1, LSB-first, 50 cycles/bit,
-- idle-high between/after. The honest inverse of the UART's own framing.
serialize :: [BitVector 8] -> [Bit]
serialize = L.concatMap serByte
 where
  serByte b =
    L.replicate cyclesPerBit low
      <> L.concatMap (\i -> L.replicate cyclesPerBit (if testBit b i then high else low)) [0 .. 7]
      <> L.replicate cyclesPerBit high

-- | Decode a captured UART line back to bytes by running it through the real
-- 'uartRx' (reuses tested framing rather than reimplementing it).
deserialize :: [Bit] -> [BitVector 8]
deserialize samples =
  [ b
  | Just b <-
      sampleN
        (L.length samples)
        ( fst (uartRx (oversampleTick (SNat @2_000_000)) (fromList (samples <> L.repeat high)))
            :: Signal Dom100 (Maybe (BitVector 8))
        )
  ]

-- | Drive 'system': feed a serialized control stream on rxLine (idle-high after),
-- hold ioIn at 0 and ALERT# idle-high, run for @length rx + nExtra@ cycles, and
-- return the sampled (txLine, cs_n, sck, lanesOut).
runSystem :: [Bit] -> Int -> ([Bit], [Bit], [Bit], [Lanes])
runSystem rxSamples nExtra =
  L.unzip4 $
    sampleN
      (leadN + L.length rxSamples + nExtra)
      ( let (txLine, lanesOut, csO, sckO, _rstO, _led) =
              system
                (fromList (L.replicate leadN high <> rxSamples <> L.repeat high))
                (pure (repeat 0))
                (pure 1)
         in bundle (txLine, csO, sckO, lanesOut) :: Signal Dom100 (Bit, Bit, Bit, Lanes)
      )
 where
  -- one idle bit-time up front so the Dom100 cycle-0 reset settles during idle,
  -- not on the first start bit (the sampleN reset idiom, cf. hdl/PLAN.md).
  leadN = cyclesPerBit

-- | Load a program + trigger, run, and return the decoded drain word-stream.
loadRunDrain :: [BitVector 32] -> Int -> Either String [BitVector 32]
loadRunDrain prog nExtra =
  case decodeResult (deserialize tx) of
    Right ws -> Right ws
    Left e -> Left (show e)
 where
  ctrl = encodeControl (LoadProgram prog) <> encodeControl Trigger
  (tx, _cs, _sck, _lanes) = runSystem (serialize ctrl) nExtra
```

Append these cases to the `Top` `testGroup` list:

```haskell
    , -- whole-system cosim: minimal program
      testCase "cosim: load [HALT], trigger -> drain = REVISION + HALT terminator" $
        loadRunDrain [encode (Halt 0)] 20000
          @?= Right [0x0001_0000, 0xC000_0000]
    , -- whole-system cosim: eSPI pin activity + still drains
      testCase "cosim: PUT program asserts CS# and toggles SCK" $ do
        let prog =
              [ encode CsAssert
              , encode (PutByteImm 0xA5)
              , encode CsDeassert
              , encode (Halt 0)
              ]
            ctrl = encodeControl (LoadProgram prog) <> encodeControl Trigger
            (tx, cs, sck, _lanes) = runSystem (serialize ctrl) 20000
        assertBool "cs_n asserts low" (low `L.elem` cs)
        assertBool "sck toggles" (low `L.elem` sck && high `L.elem` sck)
        decodeResult (deserialize tx) @?= Right [0x0001_0000, 0xC000_0000]
    ]
```

- [ ] **Step 2: run to verify red**

Run: `cabal test 2>&1 | tail -30`
Expected: FAIL — the two `cosim:` cases error on `system = undefined`; the pure-helper cases still pass.

- [ ] **Step 3 (author): implement `system`**

In `hdl/src/Tamal/Top.hs`, add imports and replace `system = undefined`.

Add to the import list:

```haskell
import Tamal.Loader (LoaderIn (..), LoaderOut (..), loader)
import Tamal.Mem (instrRam, ringRam)
import Tamal.Uart (uart)
```

Body (note: `LoaderOut`'s field accessors are `txByte`/`instrWr`/`ringAddr`/`startOut`, so the local bindings use distinct names to avoid recursive shadowing):

```haskell
system rxLine ioIn alertIn = (txLine, lanesO, csO, sckO, rstO, ledOut)
 where
  -- UART @ 2 Mbaud
  (rxByte, _rxErr, txLine, txReady) = uart (SNat @2_000_000) rxLine txByteL

  -- Loader FSM (LoaderOut fields projected to distinct local names)
  lOut = loader (LoaderIn <$> rxByte <*> txReady <*> halted <*> ringPtrO <*> ringData)
  txByteL = txByte <$> lOut
  instrWrL = instrWr <$> lOut
  ringAddrL = ringAddr <$> lOut
  startO = startOut <$> lOut

  -- Memories
  instrWord = instrRam pcO instrWrL
  ringData = ringRam ringAddrL (ringWrite <$> maybeRing)

  -- Engine
  (busOut, maybeRing) = unbundle (mealy stepM initState busInS)
  busInS = BusIn <$> instrWord <*> ioIn <*> alertIn <*> startO
  pcO = pcOut <$> busOut
  lanesO = lanesOut <$> busOut
  csO = csOut <$> busOut
  sckO = sckOut <$> busOut
  rstO = rstOut <$> busOut
  halted = haltedOut <$> busOut
  ringPtrO = ringPtrOut <$> busOut

  -- Status LED
  running = register False (mux startO (pure True) (mux halted (pure False) running))
  ledCnt = register (0 :: Unsigned 26) (ledCnt + 1)
  ledOut = ledPattern <$> (rigState <$> running <*> halted) <*> ledCnt
```

The `BusOut` projections (`pcO`/`lanesO`/`csO`/…) already use distinct local names from the `BusOut` fields (`pcOut`/`lanesOut`/`csOut`/…), so no shadowing there. `BusOut (..)` (imported in Task 1) brings the field accessors into scope.


- [ ] **Step 4: run to verify green**

Run: `cabal test 2>&1 | tail -25`
Expected: PASS — both `cosim:` cases green (`Right [0x00010000, 0xC0000000]`, CS#/SCK activity), all `Top` + prior tests pass.

If the drain decode fails or is empty, bump `nExtra` (the run needs load + a ~2-word UART drain ≈ 6–8k cycles; 20000 is generous) and confirm on a longer window; the expected words are fixed by `revisionWord = 0x00010000` and the `Halt 0` terminator `0xC0000000` (§ engine).

- [ ] **Step 5: format and commit**

```bash
make format
git add hdl/tests/Test/Top.hs hdl/src/Tamal/Top.hs
git commit -m "feat(hdl): Tamal.Top system + whole-system UART/eSPI cosim"
```

---

## Task 4: `topEntity` shell + codegen gate

**Files:**
- Modify: `hdl/src/Tamal.hs`

- [ ] **Step 1 (author): rewrite `Tamal.hs` as the shell**

Replace the whole body of `hdl/src/Tamal.hs` (keep the SPDX header):

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
Top entity for the tamal gateware: the thin pin-binding shell. It ties the 100 MHz
clock, binds the tri-state @IO[3:0]@ pads via 'Tamal.Io.espiPads', and wires the
UART / sideband / LED pins around 'Tamal.Top.system'. No reset port (power-up
@init@, per AGENTS.md).
-}
module Tamal where

import Clash.Annotations.TH
import Clash.Prelude

import Tamal.Domain (Dom100)
import Tamal.Io (espiPads)
import Tamal.Top (system)

topEntity ::
  "clk" ::: Clock Dom100 ->
  "uart_rx" ::: Signal Dom100 Bit ->
  "io" ::: Vec 4 (BiSignalIn 'PullUp Dom100 1) ->
  "alert_n" ::: Signal Dom100 Bit ->
  ( "io" ::: Vec 4 (BiSignalOut 'PullUp Dom100 1)
  , "uart_tx" ::: Signal Dom100 Bit
  , "cs_n" ::: Signal Dom100 Bit
  , "sck" ::: Signal Dom100 Bit
  , "reset_n" ::: Signal Dom100 Bit
  , "led" ::: Signal Dom100 Bit
  )
topEntity clk uartRx ioPads alertN =
  withClockResetEnable clk noReset enableGen $
    let (txLine, lanesO, csO, sckO, rstO, ledOut) = system uartRx ioIn alertIn
        (ioDrive, csPin, sckPin, rstPin, ioIn, alertIn) =
          espiPads lanesO csO sckO rstO alertN ioPads
     in (ioDrive, txLine, csPin, sckPin, rstPin, ledOut)
 where
  noReset = unsafeFromActiveHigh (pure False)

makeTopEntity 'topEntity
```

- [ ] **Step 2: build**

Run: `cabal build 2>&1 | tail -5`
Expected: success (library compiles; `Tamal.hs` type-checks against `system`/`espiPads`).

- [ ] **Step 3 (assistant): codegen gate — confirm the inout port shape**

Run: `cabal run clash -- Tamal --verilog 2>&1 | tail -10`
Expected: `Clash: Total compilation took …` (success).

Then inspect the emitted top's ports:

Run: `rg -n "input|output|inout" verilog/Tamal.topEntity/*.v | head -40`
Expected: an `inout` for the IO lanes (`inout [3:0] io` **or** `io_0..io_3`), plus `input clk`, `input uart_rx`, `input alert_n`, `output uart_tx`, `output cs_n`, `output sck`, `output reset_n`, `output led`. **Record the exact `io` port name(s)** — the XDC (Task 5) binds them.

If the `Vec 4 (BiSignalIn …)` does not lower to a usable `inout` (decision 9 fallback): change the shell to four scalar inout pairs — args `"io0".."io3" ::: BiSignalIn 'PullUp Dom100 1` and results `"io0".."io3" ::: BiSignalOut 'PullUp Dom100 1` — reassembling `ioPads = io0 :> io1 :> io2 :> io3 :> Nil` and splitting `ioDrive` back to scalars. Re-run the gate.

- [ ] **Step 4: format and commit**

```bash
make format
git add hdl/src/Tamal.hs
git commit -m "feat(hdl): topEntity shell — system + espiPads + named pins (retire heartbeat)"
```

---

## Task 5: XDC + close-out

**Files:**
- Modify: `hdl/constraints/arty_a7.xdc`, `hdl/PLAN.md`

- [ ] **Step 1 (author): extend the XDC**

Append to `hdl/constraints/arty_a7.xdc` (use the exact `io` port name(s) recorded in Task 4 Step 3 — shown here as the `io[i]` vector form; if scalars, use `io0`..`io3`). Keep the existing `clk`/`led` lines (LED reused as the status LED):

```
## ---- eSPI data lanes IO[3:0] — Pmod JA (bank 15), PULLUP (eSPI idle-high) ----
set_property -dict { PACKAGE_PIN G13 IOSTANDARD LVCMOS33 PULLUP TRUE } [get_ports { io[0] }]
set_property -dict { PACKAGE_PIN B11 IOSTANDARD LVCMOS33 PULLUP TRUE } [get_ports { io[1] }]
set_property -dict { PACKAGE_PIN A11 IOSTANDARD LVCMOS33 PULLUP TRUE } [get_ports { io[2] }]
set_property -dict { PACKAGE_PIN D12 IOSTANDARD LVCMOS33 PULLUP TRUE } [get_ports { io[3] }]

## ---- eSPI control/sideband — Pmod JB (bank 15) ----
set_property -dict { PACKAGE_PIN E15 IOSTANDARD LVCMOS33 } [get_ports { sck }]
set_property -dict { PACKAGE_PIN E16 IOSTANDARD LVCMOS33 } [get_ports { cs_n }]
set_property -dict { PACKAGE_PIN D15 IOSTANDARD LVCMOS33 } [get_ports { reset_n }]
set_property -dict { PACKAGE_PIN C15 IOSTANDARD LVCMOS33 PULLUP TRUE } [get_ports { alert_n }]

## ---- USB-UART (FTDI) — FPGA RX in / TX out ----
set_property -dict { PACKAGE_PIN D10 IOSTANDARD LVCMOS33 } [get_ports { uart_rx }]
set_property -dict { PACKAGE_PIN A9  IOSTANDARD LVCMOS33 } [get_ports { uart_tx }]
```

Update the file's header comment to note it now constrains the full eSPI/UART/LED pinout (not just clk+led).

- [ ] **Step 2: codegen still clean + format**

Run: `cabal run clash -- Tamal --verilog 2>&1 | tail -3 && make format-check 2>&1 | tail -2`
Expected: codegen success; fourmolu clean (run `make format` if not).

Note: `cd hdl && make` (full Vivado → `tamal.bit`) is the ultimate gate but is toolchain-dependent; run it where Vivado is available. It is not required to pass in a Vivado-less environment.

- [ ] **Step 3 (assistant): mark topEntity done in `hdl/PLAN.md`**

In `hdl/PLAN.md`: change the `Tamal` (top) table row status from `**placeholder heartbeat** (LED blink)` to `done (system + shell)`; add a `Tamal.Top` row (`system + pure helpers` | `done, tested`); update the "Where things stand" / "Confirmed absent" prose to state the shell is complete; in the Ordering list change `9. **topEntity** … ← next (last)` to `— done`; retire the "impure shell remains" framing.

- [ ] **Step 4: full test run + commit**

Run: `cabal test 2>&1 | tail -5`
Expected: PASS (all tests, `Top` included).

```bash
git add hdl/constraints/arty_a7.xdc hdl/PLAN.md
git commit -m "feat(hdl): XDC eSPI/UART/LED pinout + PLAN close-out (topEntity done)"
```

---

## Self-review notes

- **Spec coverage:** §4.1 helpers → Task 2; §4.2 `system` + §6.2 cosim → Task 3; §4.3 shell + §6.3 codegen gate → Task 4; §5 pin map → Task 5; cabal/runner wiring → Task 1; PLAN close-out → Task 5.
- **Type consistency:** `system` 6-tuple `(txLine, lanesO, csO, sckO, rstO, ledOut)` identical across skeleton (Task 1), impl (Task 3), and shell consumer (Task 4); `stepM`/`ringWrite`/`rigState`/`ledPattern`/`RigState` signatures identical between Task 1 skeleton and Task 2 impl; `encodeControl`/`decodeResult`/`LoadProgram`/`Trigger` from `Tamal.Wire`; `encode`/`Halt`/`CsAssert`/`PutByteImm`/`CsDeassert` from `Tamal.Isa`.
- **Known empirical knobs:** the cosim `nExtra` (run+drain window) and the confirmed `io` inout port name(s) (Task 4 Step 3) — both resolved by observation during execution; the drain oracle `[0x00010000, 0xC0000000]` is fixed by the engine.
```
