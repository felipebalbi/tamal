# Tamal Loader Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the impure loader — `Tamal.Loader` (the `RxControl → Run → Drain` lifecycle FSM) over `Tamal.Loader.Cobs` (streaming COBS decode/encode) — that parses control frames into instruction-BRAM writes + a `startIn` pulse, and on HALT streams the trace ring out as one `TRACE_DRAIN` frame. Plus the one enabling engine change: expose `ringPtr`.

**Architecture:** The loader is `mealy loaderStep initLoader` over a pure step function (matching the engine's lift). The "isolated streaming-COBS codec" is realized as two **pure step functions** in `Tamal.Loader.Cobs` (`cobsDecodeStep`, `cobsEncodeStep`) — cleaner than separate Signal blocks (no feedback plumbing) and property-tested by pure iteration against the pure `Tamal.Wire.Cobs` oracle. `loaderStep` embeds their state (`DecSt`/`EncSt`) and calls them; the frame/message layer (one-byte holdback, CRC-8, opcode dispatch, LE word (dis)assembly, the drain generator) lives in `loaderStep`, exactly as `Tamal.Wire` sits above `Tamal.Wire.Cobs`. The loader reproduces the pure `Tamal.Wire` model byte-for-byte.

**Tech Stack:** Clash 1.10 (`clash-prelude`), tasty + tasty-hunit + tasty-hedgehog. Build/test with `stack` from `hdl/`.

**Collaboration model (TDD ping-pong, per spec §11.2 / decision D6):** For each slice the **assistant writes the failing test** (the test code in this plan is authoritative and complete), the **author writes the Clash under `src/`** to make it pass (the implementation code blocks are a correct reference target — the streaming state machines were validated out-of-band against the pure `Tamal.Wire` reference before this plan; refine together in the green step), then both **refactor**. This is a Clash learning exercise.

**Spec:** `docs/superpowers/specs/2026-07-02-tamal-loader-design.md`. All section (§) references below point there unless prefixed (Wire §…, Engine §…).

**Conventions for every task**
- All commands run from `hdl/` (the Clash project root; use the tool's `workdir`).
- SPDX header on every new `.hs` file:
  ```haskell
  -- SPDX-FileCopyrightText: 2026 Felipe Balbi
  -- SPDX-License-Identifier: CERN-OHL-P-2.0
  ```
- `make format` before each commit; `stack test` must stay green.
- **List gotcha:** `Clash.Prelude` re-exports `map`/`(++)`/`reverse`/`foldl'` as the **`Vec`** versions. In `tests/`, for `[BitVector 8]` list work use `import qualified Data.List as L` (`L.length`, `L.filter`, `L.zip`, `L.repeat`, `L.replicate`, `L.take`, `L.drop`, `(L.!!)`), the list `(<>)`/`(:)`, and list comprehensions. `Data.Maybe (mapMaybe, fromMaybe, isJust)` for the stream collectors.
- **Signal-level test idiom** (from `Test.Mem`/`Test.Uart`): `sampleN n (block (fromList (xs <> L.repeat pad)) :: Signal Dom100 _)`, with an inline `:: Signal Dom100 _` annotation so `sampleN` can solve `KnownDomain`. `mealy` has no output register, so (unlike `blockRam`) there is **no** cycle-0 value to drop.

---

## File Structure

- **Modify** `hdl/src/Tamal/Engine.hs` — add `ringPtrOut :: Unsigned 12` to `BusOut` + one line in `busOut` (Task 1).
- **Create** `hdl/src/Tamal/Loader/Cobs.hs` — `DecSt`/`initDec`/`cobsDecodeStep`, `EncSt`/`initEnc`/`cobsEncodeStep` (pure streaming COBS step functions).
- **Create** `hdl/src/Tamal/Loader.hs` — `LoaderIn`/`LoaderOut`, the lifecycle FSM `loader`, and the pure `loaderStep` + helpers.
- **Create** `hdl/tests/Test/Loader.hs` — all loader + codec properties/vectors (grows across tasks).
- **Modify** `hdl/tamal.cabal` — `exposed-modules += Tamal.Loader.Cobs, Tamal.Loader`; test `other-modules += Test.Loader`.
- **Modify** `hdl/tests/unittests.hs` — import + include `Test.Loader.tests`.
- **Modify** `hdl/PLAN.md` — mark piece 3 (loader) done; next = IOBUF.

Leaves reused unchanged: `Tamal.Wire` / `Tamal.Wire.Cobs` (the pure oracle + `encodeControl`/`encodeResult`/`ControlMsg`), `Tamal.Crc` (`crc8Update`), `Tamal.Mem` (the BRAMs, wired by the topEntity later). No `topEntity` change here.

---

## Task 1: Engine `ringPtrOut` projection + `Test.Loader` scaffold

**Files:**
- Modify: `hdl/src/Tamal/Engine.hs` (add `ringPtrOut` to `BusOut` + `busOut`)
- Create: `hdl/tests/Test/Loader.hs`
- Modify: `hdl/tamal.cabal`, `hdl/tests/unittests.hs`

- [ ] **Step 1: Add the `ringPtrOut` field** (author). In `hdl/src/Tamal/Engine.hs`, add a field to `BusOut` (after `haltedOut`) and to its haddock:

```haskell
data BusOut = BusOut
  { pcOut :: Unsigned AW
  , csOut :: Bit
  , sckOut :: Bit
  , rstOut :: Bit
  , lanesOut :: Lanes
  , haltedOut :: Bool
  , ringPtrOut :: Unsigned 12
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)
```

- [ ] **Step 2: Project it in `busOut`** (author). Add the one line (after `haltedOut = ...`):

```haskell
busOut :: State -> BusOut
busOut s =
  BusOut
    { pcOut = pc s
    , csOut = csN s
    , sckOut = sck s
    , rstOut = rstN s
    , lanesOut = lanes s
    , haltedOut = phase s == Halted
    , ringPtrOut = ringPtr s
    }
```

- [ ] **Step 3: Register the test module** (author). In `hdl/tamal.cabal`, add `Test.Loader` to the `test-suite` `other-modules` (after `Test.Wire`). Do **not** add the `Tamal.Loader*` library modules yet — they are created and registered in Task 2 (`Tamal.Loader.Cobs`) and Task 4 (`Tamal.Loader`), so registering them now would break the build.

- [ ] **Step 4: Create `hdl/tests/Test/Loader.hs`** (assistant) with the `ringPtrOut` projection tests:

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
{-# LANGUAGE NumericUnderscores #-}

module Test.Loader (tests) where

import Clash.Prelude
import Test.Tasty
import Test.Tasty.HUnit

import Tamal.Engine (State (..), busOut, initState, ringPtrOut)

tests :: TestTree
tests =
  testGroup
    "Loader"
    [ testGroup
        "engine ringPtrOut projection"
        [ testCase "initState projects ringPtr = 1"
            $ ringPtrOut (busOut initState) @?= 1
        , testCase "busOut projects the State ringPtr verbatim"
            $ ringPtrOut (busOut initState{ringPtr = 42}) @?= 42
        , testCase "projects the top-of-ring value"
            $ ringPtrOut (busOut initState{ringPtr = maxBound}) @?= (maxBound :: Unsigned 12)
        ]
    ]
```

- [ ] **Step 5: Wire into `unittests.hs`** (author). Add `import qualified Test.Loader` (after `import qualified Test.Wire`) and `, Test.Loader.tests` to the `tests` list (after `Test.Wire.tests`).

- [ ] **Step 6: Run** (author).

Run: `stack test --test-arguments '-p "/Loader/"'`
Expected: PASS (3 cases). Run `stack test` too — all existing groups (incl. `Engine`) stay green (adding a `BusOut` field does not touch any accessor-only test).

- [ ] **Step 7: Format and commit.**

```bash
make format
git add src/Tamal/Engine.hs tests/Test/Loader.hs tamal.cabal tests/unittests.hs
git commit -m "feat(hdl): expose engine ringPtrOut (drain bound) + Test.Loader scaffold"
```

---

## Task 2: `Tamal.Loader.Cobs` — streaming decode

**Files:**
- Create: `hdl/src/Tamal/Loader/Cobs.hs`
- Modify: `hdl/tamal.cabal` (add `Tamal.Loader.Cobs` to `exposed-modules`, before `Tamal.Loader`… which is added in Task 4; for now add just `Tamal.Loader.Cobs` after `Tamal.Wire`)
- Test: `hdl/tests/Test/Loader.hs`

- [ ] **Step 1: Write the failing tests** (assistant). Extend `Test/Loader.hs` imports and add the decode group. New imports:

```haskell
import qualified Data.List as L
import Hedgehog (Gen, forAll, property, (===))
import qualified Hedgehog.Gen as Gen
import qualified Hedgehog.Range as Range
import Test.Tasty.Hedgehog (testProperty)

import Tamal.Loader.Cobs (DecSt, cobsDecodeStep, initDec)
import Tamal.Wire.Cobs (cobsEncode)
```

Add these helpers to the module (after `tests`):

```haskell
-- A zero-dense byte generator: stresses COBS group boundaries (~1/4 zeros).
genByteZeros :: Gen (BitVector 8)
genByteZeros = Gen.frequency [(1, pure 0), (4, genB)]
 where
  genB = fromIntegral <$> Gen.int (Range.linear 0 255)

-- Pure driver for the decode step: feed the COBS bytes (delimiter stripped),
-- then a frame-end pulse; collect the decoded bytes and the malformed flag.
decDrive :: [BitVector 8] -> ([BitVector 8], Bool)
decDrive enc = go initDec ([(Just b, False) | b <- enc] <> [(Nothing, True)])
 where
  go :: DecSt -> [(Maybe (BitVector 8), Bool)] -> ([BitVector 8], Bool)
  go _ [] = ([], False)
  go st (i : is) =
    let (st', (mo, done, bad)) = cobsDecodeStep st i
     in if done
          then ([], bad)
          else let (os, b) = go st' is in (maybe os (: os) mo, b)
```

Add a `decodeTests` group and include it in the top-level list (`, decodeTests`):

```haskell
decodeTests :: TestTree
decodeTests =
  testGroup
    "Loader.Cobs decode"
    [ testProperty "streaming decode of cobsEncode x reconstructs x" $ property $ do
        x <- forAll (Gen.list (Range.linear 0 300) genByteZeros)
        let (dec, bad) = decDrive (cobsEncode x)
        dec === x
        bad === False
    , testCase "decode reconstructs the [0x00] vector"
        $ decDrive (cobsEncode [0x00]) @?= ([0x00], False)
    , testCase "decode reconstructs the 254/255 boundary vectors" $ do
        decDrive (cobsEncode (L.map fromIntegral [1 .. 254 :: Int]))
          @?= (L.map fromIntegral [1 .. 254 :: Int], False)
        decDrive (cobsEncode (L.map fromIntegral [1 .. 255 :: Int]))
          @?= (L.map fromIntegral [1 .. 255 :: Int], False)
    , testCase "truncated group -> malformed"
        $ snd (decDrive [0x05, 0x11]) @?= True
    , testCase "empty frame (delimiter only) -> malformed"
        $ snd (decDrive []) @?= True
    , testCase "code byte with no data -> malformed"
        $ snd (decDrive [0x03]) @?= True
    ]
```

Also add `import Test.Tasty.HUnit` items already present; ensure `genByte` isn't needed (we use `genByteZeros`). Add `genByteZeros`'s `Gen` import is covered.

- [ ] **Step 2: Run to verify it fails** (assistant).

Run: `stack test --test-arguments '-p "/Loader/"'`
Expected: FAIL — `Tamal.Loader.Cobs` / `cobsDecodeStep` not in scope (module missing).

- [ ] **Step 3: Create `hdl/src/Tamal/Loader/Cobs.hs`** (author) with the decode step (encode added in Task 3). This state machine was validated against the pure `cobsDecode` reference:

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
Streaming COBS codec (spec §9): the per-cycle step functions the loader embeds.
Pure — no CRC, no delimiter (those live in the loader's frame layer, exactly as
'Tamal.Wire' sits above 'Tamal.Wire.Cobs'). Each step's iterated output equals
the pure 'Tamal.Wire.Cobs' reference.
-}
module Tamal.Loader.Cobs
  ( DecSt
  , initDec
  , cobsDecodeStep
  ) where

import Clash.Prelude

{- | Decode state: data bytes remaining in the current group (@0@ = expecting a
code byte), whether that group's code was @255@ (a full group injects no zero),
whether we owe an injected zero before the next code byte, and whether any byte
has arrived this frame.
-}
data DecSt = DecSt
  { dCnt :: Unsigned 8
  , dFull :: Bool
  , dPend :: Bool
  , dGot :: Bool
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

initDec :: DecSt
initDec = DecSt 0 False False False

{- | One decode step. Input @(maybe-COBS-byte, frame-end)@; the loader pulses
frame-end when it sees the @0x00@ delimiter. Output @(maybe-decoded-byte, done,
malformed)@. On frame-end the state resets and @done@ pulses; @malformed@ is set
if the frame was empty or a group was left truncated.
-}
cobsDecodeStep ::
  DecSt ->
  (Maybe (BitVector 8), Bool) ->
  (DecSt, (Maybe (BitVector 8), Bool, Bool))
cobsDecodeStep s (mIn, frameEnd)
  | frameEnd = (initDec, (Nothing, True, not (dGot s) || dCnt s /= 0))
  | otherwise = case mIn of
      Nothing -> (s, (Nothing, False, False))
      Just b
        | dCnt s == 0 ->
            let (out, s1) =
                  if dPend s
                    then (Just 0, s{dPend = False, dGot = True})
                    else (Nothing, s{dGot = True})
                s2 = startGroup s1 b
             in (s2, (out, False, False))
        | otherwise ->
            let cnt' = dCnt s - 1
                s' =
                  s
                    { dCnt = cnt'
                    , dGot = True
                    , dPend = if cnt' == 0 then not (dFull s) else dPend s
                    }
             in (s', (Just b, False, False))
 where
  startGroup st c =
    let full = c == 255
        n = (unpack c :: Unsigned 8) - 1 -- c is 1..255 (never 0), so n is 0..254
     in if n == 0
          then st{dCnt = 0, dFull = full, dPend = not full}
          else st{dCnt = n, dFull = full}
```

- [ ] **Step 4: Register the module** (author). In `hdl/tamal.cabal`, add `Tamal.Loader.Cobs` to `exposed-modules` (after `Tamal.Wire`).

- [ ] **Step 5: Run to verify it passes** (author).

Run: `stack test --test-arguments '-p "/Loader.Cobs decode/"'`
Expected: PASS (round-trip over zero-dense inputs + boundary + malformed vectors).

- [ ] **Step 6: Format and commit.**

```bash
make format
git add src/Tamal/Loader/Cobs.hs tests/Test/Loader.hs tamal.cabal
git commit -m "feat(hdl): streaming COBS decode step (Tamal.Loader.Cobs)"
```

> Mentoring note: the injected zero is deferred — a non-full group's zero is emitted on the cycle the *next* code byte arrives (`dPend`), which keeps output at ≤1 byte per input byte. `not (dGot s) || dCnt s /= 0` at frame-end catches the empty frame and any truncated group.

---

## Task 3: `Tamal.Loader.Cobs` — streaming encode

**Files:**
- Modify: `hdl/src/Tamal/Loader/Cobs.hs` (add `EncSt`/`initEnc`/`cobsEncodeStep`, widen exports)
- Test: `hdl/tests/Test/Loader.hs`

- [ ] **Step 1: Write the failing tests** (assistant). Extend the `Tamal.Loader.Cobs` import and add the encode group. Change the import to:

```haskell
import Tamal.Loader.Cobs (DecSt, EncSt, cobsDecodeStep, cobsEncodeStep, initDec, initEnc)
```

Add the pure encode driver (after `decDrive`):

```haskell
-- Pure driver for the encode step: feed logical bytes (last one flagged),
-- downstream always ready; advance the input only when readyIn (byte consumed).
-- Collects the emitted COBS bytes (no delimiter).
encDrive :: [BitVector 8] -> [BitVector 8]
encDrive [] = []
encDrive xs = go initEnc [(x, i == L.length xs - 1) | (i, x) <- L.zip [0 ..] xs]
 where
  go :: EncSt -> [(BitVector 8, Bool)] -> [BitVector 8]
  go st inp =
    let (feed, rest0) = case inp of
          (t : ts) -> (Just t, ts)
          [] -> (Nothing, [])
        (st', (readyIn, mo, done)) = cobsEncodeStep st (feed, True)
        inp' = if readyIn then rest0 else inp
     in if done then [] else maybe (go st' inp') (: go st' inp') mo
```

Add an `encodeTests` group and include it (`, encodeTests`):

```haskell
encodeTests :: TestTree
encodeTests =
  testGroup
    "Loader.Cobs encode"
    [ testProperty "streaming encode equals cobsEncode (non-empty)" $ property $ do
        x <- forAll (Gen.list (Range.linear 1 300) genByteZeros)
        encDrive x === cobsEncode x
    , testCase "encode the [11,00,00,00] vector"
        $ encDrive [0x11, 0x00, 0x00, 0x00] @?= [0x02, 0x11, 0x01, 0x01, 0x01]
    , testCase "encode the 254/255 boundary" $ do
        encDrive (L.map fromIntegral [1 .. 254 :: Int])
          @?= (0xFF : L.map fromIntegral [1 .. 254 :: Int])
        encDrive (L.map fromIntegral [1 .. 255 :: Int])
          @?= (0xFF : L.map fromIntegral [1 .. 254 :: Int]) <> [0x02, 255]
    , testProperty "encode then decode round-trips (both streaming)" $ property $ do
        x <- forAll (Gen.list (Range.linear 1 300) genByteZeros)
        decDrive (encDrive x) === (x, False)
    ]
```

- [ ] **Step 2: Run to verify it fails** (assistant).

Run: `stack test --test-arguments '-p "/Loader.Cobs encode/"'`
Expected: FAIL — `cobsEncodeStep`/`initEnc`/`EncSt` not in scope.

- [ ] **Step 3: Add the encode step** (author). Widen the export list to include `EncSt`, `initEnc`, `cobsEncodeStep`, and add (this machine was validated against the pure `cobsEncode`; `readyIn` is `True` iff the byte is consumed this cycle — i.e. only in `EFilling`):

```haskell
{- | Encode mode: filling the current group, or emitting @code ++ group@. -}
data EncMode = EFilling | EEmitting
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

{- | Encode state: the ≤254-byte group buffer + fill count, the emit index
(@0@ = code byte, @1..fill@ = data), a byte stashed when a full 254-group is
flushed (it starts the next group), whether a final empty group is still owed
(the last input byte was @0x00@), and whether the input stream has ended.
-}
data EncSt = EncSt
  { eMode :: EncMode
  , eBuf :: Vec 254 (BitVector 8)
  , eFill :: Unsigned 8
  , eIx :: Unsigned 8
  , ePend :: Maybe (BitVector 8, Bool)
  , eFinal :: Bool
  , eLast :: Bool
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

initEnc :: EncSt
initEnc = EncSt EFilling (repeat 0) 0 0 Nothing False False

{- | One encode step. Input @(maybe-(byte,is-last), downstream-ready)@; output
@(ready-in, maybe-COBS-byte, done)@. @ready-in@ is high only while filling (the
one state that consumes input). An output byte appears only when
downstream-ready. @done@ pulses when the whole frame's COBS output is emitted.
-}
cobsEncodeStep ::
  EncSt ->
  (Maybe (BitVector 8, Bool), Bool) ->
  (EncSt, (Bool, Maybe (BitVector 8), Bool))
cobsEncodeStep s (mIn, dsReady) = case eMode s of
  EFilling -> case mIn of
    Nothing -> (s, (True, Nothing, False))
    Just (b, lst)
      | b == 0 ->
          (s{eMode = EEmitting, eIx = 0, eFinal = lst, eLast = lst}, (True, Nothing, False))
      | eFill s == 254 ->
          (s{eMode = EEmitting, eIx = 0, ePend = Just (b, lst)}, (True, Nothing, False))
      | lst ->
          ((store s b){eMode = EEmitting, eIx = 0, eLast = True, eFinal = False}, (True, Nothing, False))
      | otherwise ->
          (store s b, (True, Nothing, False))
  EEmitting
    | not dsReady -> (s, (False, Nothing, False))
    | eIx s <= eFill s ->
        let out :: BitVector 8
            out = if eIx s == 0 then fromIntegral (eFill s) + 1 else eBuf s !! (eIx s - 1)
         in (s{eIx = eIx s + 1}, (False, Just out, False))
    | eFinal s -> (s{eFill = 0, eIx = 0, eFinal = False}, (False, Nothing, False))
    | otherwise -> case ePend s of
        Just (pb, pl)
          | pl -> ((store s{eFill = 0} pb){eMode = EEmitting, eIx = 0, ePend = Nothing, eLast = True}, (False, Nothing, False))
          | otherwise -> ((store s{eFill = 0} pb){eMode = EFilling, ePend = Nothing}, (False, Nothing, False))
        Nothing
          | eLast s -> (initEnc, (False, Nothing, True))
          | otherwise -> (s{eMode = EFilling, eFill = 0}, (False, Nothing, False))
 where
  store st b = st{eBuf = replace (eFill st) b (eBuf st), eFill = eFill st + 1}
```

- [ ] **Step 4: Run to verify it passes** (author).

Run: `stack test --test-arguments '-p "/Loader.Cobs/"'`
Expected: PASS (encode == oracle over zero-dense inputs, boundary vectors, and the streaming round-trip).

- [ ] **Step 5: Format and commit.**

```bash
make format
git add src/Tamal/Loader/Cobs.hs tests/Test/Loader.hs
git commit -m "feat(hdl): streaming COBS encode step (Tamal.Loader.Cobs)"
```

> Mentoring note: the code byte precedes its data, so a whole group must be buffered before the code is known (`eBuf`/`eFill`). The full-group case (`eFill == 254`) flushes with `code = 255` and stashes the incoming byte (`ePend`) to start the next group; a `0x00` closes a group and, if it was the last byte, still owes one final empty group (`eFinal`). `replace`/`(!!)` index the `Vec 254` with the `Unsigned 8` counters — safe because writes happen only at `eFill < 254` and reads at `eIx-1 ∈ 0..253`.

---

## Task 4: `Tamal.Loader` — types + RX load path (`RxControl`)

**Files:**
- Create: `hdl/src/Tamal/Loader.hs`
- Modify: `hdl/tamal.cabal` (add `Tamal.Loader` to `exposed-modules`, after `Tamal.Loader.Cobs`)
- Test: `hdl/tests/Test/Loader.hs`

- [ ] **Step 1: Write the failing tests** (assistant). Add imports and the RX group. New imports in `Test/Loader.hs`:

```haskell
import Data.Maybe (mapMaybe)
import Tamal.Domain (Dom100)
import Tamal.Loader (LoaderIn (..), LoaderOut (..), loader)
import Tamal.Wire (ControlMsg (..), encodeControl)
import Test.Gen (genWord)
```

Add a signal-level harness (after `encDrive`):

```haskell
-- Feed an rxByte stream (idle otherwise), collecting the instr-BRAM writes.
-- The engine/ring inputs are quiescent (halted low, ring empty).
simInstrWr :: [Maybe (BitVector 8)] -> [(Unsigned 10, BitVector 32)]
simInstrWr rxs =
  mapMaybe instrWr
    $ sampleN
      (L.length rxs + 8)
      (loader (fromList (fmap mkIn (rxs <> L.repeat Nothing))) :: Signal Dom100 LoaderOut)
 where
  mkIn r = LoaderIn{rxByte = r, txReady = True, halted = False, ringPtrIn = 0, ringData = 0}

-- Collect the startOut pulses over an rxByte stream.
simStartOut :: [Maybe (BitVector 8)] -> [Bool]
simStartOut rxs =
  fmap startOut
    $ sampleN
      (L.length rxs + 8)
      (loader (fromList (fmap mkIn (rxs <> L.repeat Nothing))) :: Signal Dom100 LoaderOut)
 where
  mkIn r = LoaderIn{rxByte = r, txReady = True, halted = False, ringPtrIn = 0, ringData = 0}
```

Add an `rxTests` group and include it (`, rxTests`):

```haskell
rxTests :: TestTree
rxTests =
  testGroup
    "Loader RX / load"
    [ testProperty "LOAD_PROGRAM writes the exact words at 0,1,2,.." $ property $ do
        ws <- forAll (Gen.list (Range.linear 0 20) genWord)
        let bytes = encodeControl (LoadProgram ws)
        simInstrWr (fmap Just bytes) === [(fromIntegral i, w) | (i, w) <- L.zip [0 :: Int ..] ws]
    , testCase "LOAD_PROGRAM does not pulse startOut" $ do
        let bytes = encodeControl (LoadProgram [0xDEAD_BEEF, 0x0000_0001])
        L.filter id (simStartOut (fmap Just bytes)) @?= []
    , testCase "TRIGGER pulses startOut exactly once, after the frame" $ do
        let bytes = encodeControl Trigger
        L.length (L.filter id (simStartOut (fmap Just bytes))) @?= 1
    ]
```

- [ ] **Step 2: Run to verify it fails** (assistant).

Run: `stack test --test-arguments '-p "/Loader RX/"'`
Expected: FAIL — `Tamal.Loader` / `LoaderIn` / `loader` not in scope.

- [ ] **Step 3: Create `hdl/src/Tamal/Loader.hs`** (author) with the types, the `mealy` lift, and the `RxControl`/`Run` branches of `loaderStep` (the `Drain` branch is a stub here, completed in Task 5). The RX holdback+CRC+word-assembly was validated against `decodeControl`:

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
The impure loader (design doc 2026-07-02-tamal-loader-design.md): the
@RxControl -> Run -> Drain@ lifecycle FSM bridging the UART, the two BRAMs, and
the engine's @startIn@/@haltedOut@. It is the streaming realization of the pure
'Tamal.Wire' model; the streaming COBS lives in 'Tamal.Loader.Cobs'.
-}
module Tamal.Loader
  ( LoaderIn (..)
  , LoaderOut (..)
  , loader
  ) where

import Clash.Prelude
import Data.Maybe (fromMaybe, isJust)

import Tamal.Crc (crc8Update)
import Tamal.Loader.Cobs

-- | What the top feeds the loader each cycle.
data LoaderIn = LoaderIn
  { rxByte :: Maybe (BitVector 8)
  , txReady :: Bool
  , halted :: Bool
  , ringPtrIn :: Unsigned 12
  , ringData :: BitVector 32
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

-- | The loader's outputs: UART TX, instr-BRAM write, ring-BRAM read address,
-- and the engine start pulse.
data LoaderOut = LoaderOut
  { txByte :: Maybe (BitVector 8)
  , instrWr :: Maybe (Unsigned 10, BitVector 32)
  , ringAddr :: Unsigned 12
  , startOut :: Bool
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

data Lifecycle = RxControl | Run | Drain
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

data DrainPhase = DrOpcode | DrFetch | DrLatch | DrWordByte | DrCrcByte | DrDrainOut | DrDelim
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

data LoaderSt = LoaderSt
  { lPhase :: Lifecycle
  , lDec :: DecSt
  , lEnc :: EncSt
  , lHeld :: Maybe (BitVector 8) -- one-byte holdback (separates the trailing CRC)
  , lCrcRx :: BitVector 8 -- running CRC over confirmed bytes
  , lHaveOp :: Bool -- opcode confirmed yet?
  , lOpcode :: BitVector 8
  , lByteIx :: Unsigned 2 -- payload byte within the current word (0..3)
  , lWordAcc :: BitVector 32 -- LE word being assembled
  , lHadPay :: Bool -- any payload byte seen (TRIGGER must have none)
  , lAddr :: Unsigned 10 -- next instr write slot
  , lFull :: Bool -- instr store overflowed (>1024 words)
  , lDrn :: DrainPhase
  , lWord :: BitVector 32 -- ring word being emitted
  , lWIx :: Unsigned 2 -- LE byte of lWord (0..3)
  , lCrcTx :: BitVector 8 -- running CRC over the drain
  , lDrCnt :: Unsigned 12 -- ring record index being fetched
  , lTerm :: Bool -- fetching/emitting the terminator word
  }
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

initLoader :: LoaderSt
initLoader =
  LoaderSt
    { lPhase = RxControl
    , lDec = initDec
    , lEnc = initEnc
    , lHeld = Nothing
    , lCrcRx = 0
    , lHaveOp = False
    , lOpcode = 0
    , lByteIx = 0
    , lWordAcc = 0
    , lHadPay = False
    , lAddr = 0
    , lFull = False
    , lDrn = DrOpcode
    , lWord = 0
    , lWIx = 0
    , lCrcTx = 0
    , lDrCnt = 0
    , lTerm = False
    }

idleOut :: LoaderOut
idleOut = LoaderOut{txByte = Nothing, instrWr = Nothing, ringAddr = 0, startOut = False}

-- | The loader: a plain 'mealy' over the pure 'loaderStep' (matching the engine lift).
loader :: (HiddenClockResetEnable dom) => Signal dom LoaderIn -> Signal dom LoaderOut
loader = mealy loaderStep initLoader

loaderStep :: LoaderSt -> LoaderIn -> (LoaderSt, LoaderOut)
loaderStep s inp = case lPhase s of
  RxControl -> rxStep s inp
  Run -> runStep s inp
  Drain -> drainStep s inp

-- | Run: idle until the engine halts, then start the drain.
runStep :: LoaderSt -> LoaderIn -> (LoaderSt, LoaderOut)
runStep s inp
  | halted inp =
      ( s{lPhase = Drain, lEnc = initEnc, lDrn = DrOpcode, lCrcTx = 0, lDrCnt = 0, lWIx = 0, lTerm = False}
      , idleOut
      )
  | otherwise = (s, idleOut)

-- | RxControl: stream-decode a control frame; on the delimiter, verify + dispatch.
rxStep :: LoaderSt -> LoaderIn -> (LoaderSt, LoaderOut)
rxStep s inp =
  let din = case rxByte inp of
        Just 0 -> (Nothing, True) -- delimiter => frame end
        Just b -> (Just b, False)
        Nothing -> (Nothing, False)
      (dec', (mDec, done, bad)) = cobsDecodeStep (lDec s) din
      s1 = s{lDec = dec'}
   in if done
        then finalize s1 bad
        else case mDec of
          Nothing -> (s1, idleOut)
          Just d ->
            let (s2, mw) = case lHeld s1 of
                  Just h -> confirm s1 h
                  Nothing -> (s1, Nothing)
             in (s2{lHeld = Just d}, idleOut{instrWr = mw})

-- | Confirm a held (definitely-not-CRC) byte: fold CRC, route as opcode or a
-- payload byte, assembling LE words and writing them (write-through) for LOAD.
confirm :: LoaderSt -> BitVector 8 -> (LoaderSt, Maybe (Unsigned 10, BitVector 32))
confirm s h
  | not (lHaveOp s) =
      ( s
          { lHaveOp = True
          , lOpcode = h
          , lCrcRx = crc8Update (lCrcRx s) h
          , lAddr = if h == 0x01 then 0 else lAddr s
          , lFull = if h == 0x01 then False else lFull s
          }
      , Nothing
      )
  | otherwise =
      let crc' = crc8Update (lCrcRx s) h
          acc' = lWordAcc s .|. (zeroExtend h `shiftL` (8 * fromIntegral (lByteIx s)))
          isLoad = lOpcode s == 0x01
       in if lByteIx s == 3
            then
              let doWrite = isLoad && not (lFull s)
                  (addr', full') = if lAddr s == maxBound then (lAddr s, True) else (lAddr s + 1, lFull s)
               in ( s
                      { lCrcRx = crc'
                      , lWordAcc = 0
                      , lByteIx = 0
                      , lHadPay = True
                      , lAddr = if isLoad then addr' else lAddr s
                      , lFull = if isLoad then full' else lFull s
                      }
                  , if doWrite then Just (lAddr s, acc') else Nothing
                  )
            else
              ( s{lCrcRx = crc', lWordAcc = acc', lByteIx = lByteIx s + 1, lHadPay = True}
              , Nothing
              )

-- | Frame end: the held byte is the CRC candidate. A good TRIGGER pulses
-- startOut and enters Run; a good LOAD's words are already written; anything
-- else is discarded (D4/D5). Reset the frame-parse state either way.
finalize :: LoaderSt -> Bool -> (LoaderSt, LoaderOut)
finalize s bad =
  let crcCand = fromMaybe 0 (lHeld s)
      crcGood = not bad && isJust (lHeld s) && lHaveOp s && lCrcRx s == crcCand
      trigOk = crcGood && lOpcode s == 0x02 && not (lHadPay s)
      s0 = resetFrame s
   in if trigOk
        then (s0{lPhase = Run}, idleOut{startOut = True})
        else (s0, idleOut)

resetFrame :: LoaderSt -> LoaderSt
resetFrame s =
  s
    { lDec = initDec
    , lHeld = Nothing
    , lCrcRx = 0
    , lHaveOp = False
    , lOpcode = 0
    , lByteIx = 0
    , lWordAcc = 0
    , lHadPay = False
    }

-- | Drain: completed in Task 5.
drainStep :: LoaderSt -> LoaderIn -> (LoaderSt, LoaderOut)
drainStep s _ = (resetFrame s{lPhase = RxControl}, idleOut)
```

- [ ] **Step 4: Register the module** (author). In `hdl/tamal.cabal`, add `Tamal.Loader` to `exposed-modules` (after `Tamal.Loader.Cobs`).

- [ ] **Step 5: Run to verify it passes** (author).

Run: `stack test --test-arguments '-p "/Loader RX/"'`
Expected: PASS (exact-words property, no-pulse for LOAD, one-pulse for TRIGGER).

- [ ] **Step 6: Format and commit.**

```bash
make format
git add src/Tamal/Loader.hs tests/Test/Loader.hs tamal.cabal
git commit -m "feat(hdl): loader RxControl load path (decode+holdback+CRC+word writes)"
```

> Mentoring note: the one-byte holdback (`lHeld`) is how a stream separates the trailing CRC from payload — every decoded byte is confirmed only once the *next* one arrives, so the byte still held at the delimiter is the CRC. Word writes are write-through (a bad frame is simply overwritten by the retry); the commit is the separate `TRIGGER`.

---

## Task 5: `Tamal.Loader` — drain path (`Run → Drain → RxControl`)

**Files:**
- Modify: `hdl/src/Tamal/Loader.hs` (replace the `drainStep` stub with the real generator+encoder; add `feedByte`, `afterWordByte`, `leByte`)
- Test: `hdl/tests/Test/Loader.hs`

- [ ] **Step 1: Write the failing tests** (assistant). Add imports and the drain group. New imports:

```haskell
import Tamal.Wire (encodeResult)
```

Add the drain harness (after `simStartOut`). It closes the ring-BRAM read loop (1-cycle latency) with a `register`, feeds a `TRIGGER`, then asserts `halted` so the loader drains a modeled ring:

```haskell
-- The loader with the ring-BRAM read loop closed (1-cycle latency via register).
-- Feedback harnesses MUST be a function carrying (HiddenClockResetEnable dom) so
-- sampleN can supply the hidden clock/reset/enable (the Test.Uart fastLoop idiom);
-- a `where`-bound signal at the test level would have no clock in scope.
drainRig ::
  (HiddenClockResetEnable dom) =>
  (Unsigned 12 -> BitVector 32) -> -- ring lookup (simulation-only, non-synthesizable)
  Unsigned 12 -> -- ringPtr
  Signal dom (Maybe (BitVector 8)) -> -- rxByte
  Signal dom Bool -> -- txReady
  Signal dom Bool -> -- halted
  Signal dom (Maybe (BitVector 8)) -- txByte
drainRig lookupRing ringPtrV rxs txr hlt = txByte <$> loaderOut
 where
  loaderOut = loader loaderIn
  ringDataS = register 0 (lookupRing <$> (ringAddr <$> loaderOut))
  loaderIn = LoaderIn <$> rxs <*> txr <*> hlt <*> pure ringPtrV <*> ringDataS

-- Ring model: word[0..ringPtr-1] = records (word0 = REVISION); word[termAddr] = term.
ringModel :: [BitVector 32] -> BitVector 32 -> Unsigned 12 -> BitVector 32
ringModel records term a
  | a == maxBound = term
  | fromIntegral a < L.length records = records L.!! fromIntegral a
  | otherwise = 0

-- Drive TRIGGER -> Run -> (halted) -> Drain; collect the drained byte stream.
-- @records@ are word[0..ringPtr-1] (word0 = REVISION); @term@ is the terminator.
simDrain :: [BitVector 32] -> BitVector 32 -> [Bool] -> [BitVector 8]
simDrain records term txReadyPat =
  mapMaybe id
    $ sampleN
      2500
      ( drainRig
          (ringModel records term)
          (fromIntegral (L.length records))
          (fromList rxs)
          (fromList (L.cycle txReadyPat))
          (fromList halteds) ::
          Signal Dom100 (Maybe (BitVector 8))
      )
 where
  trig = encodeControl Trigger
  rxs = fmap Just trig <> L.repeat Nothing
  halteds = L.replicate (L.length trig + 6) False <> L.repeat True
```

Add a `drainTests` group and include it (`, drainTests`):

```haskell
drainTests :: TestTree
drainTests =
  testGroup
    "Loader TX / drain"
    [ testProperty "drain stream == encodeResult (records ++ terminator)" $ property $ do
        records <- forAll (Gen.list (Range.linear 1 24) genWord)
        term <- forAll genWord
        simDrain records term [True] === encodeResult (records <> [term])
    , testCase "minimal drain: REVISION + terminator only"
        $ simDrain [0x0001_0000] 0xC000_0000 [True]
        @?= encodeResult [0x0001_0000, 0xC000_0000]
    , testProperty "drain is byte-identical under txReady backpressure" $ property $ do
        records <- forAll (Gen.list (Range.linear 1 16) genWord)
        term <- forAll genWord
        simDrain records term [True, False, True, True, False]
          === encodeResult (records <> [term])
    ]
```

- [ ] **Step 2: Run to verify it fails** (assistant).

Run: `stack test --test-arguments '-p "/Loader TX/"'`
Expected: FAIL — the `drainStep` stub emits nothing, so the drained stream is empty (`[] /= encodeResult …`).

- [ ] **Step 3: Replace `drainStep`** (author) with the real drain generator and its helpers. This produces the logical stream `opcode ++ ring-words-LE ++ CRC`, feeds it through `cobsEncodeStep` (gated by `readyIn`), and appends the `0x00` delimiter — validated to equal `encodeResult`:

```haskell
-- | Drain: emit one TRACE_DRAIN frame from the ring, then return to RxControl.
drainStep :: LoaderSt -> LoaderIn -> (LoaderSt, LoaderOut)
drainStep s inp = case lDrn s of
  DrOpcode ->
    feedByte s inp 0x81 False (\s' -> s'{lDrn = DrFetch, lDrCnt = 0, lTerm = False})
  DrWordByte ->
    feedByte s inp (leByte (lWord s) (lWIx s)) False (afterWordByte inp)
  DrCrcByte ->
    feedByte s inp (lCrcTx s) True (\s' -> s'{lDrn = DrDrainOut})
  DrFetch ->
    let addr = if lTerm s then maxBound else lDrCnt s
        (enc', (_, mOut, _)) = cobsEncodeStep (lEnc s) (Nothing, txReady inp)
     in (s{lEnc = enc', lDrn = DrLatch}, idleOut{txByte = mOut, ringAddr = addr})
  DrLatch ->
    let addr = if lTerm s then maxBound else lDrCnt s
        (enc', (_, mOut, _)) = cobsEncodeStep (lEnc s) (Nothing, txReady inp)
     in ( s{lEnc = enc', lWord = ringData inp, lWIx = 0, lDrn = DrWordByte}
        , idleOut{txByte = mOut, ringAddr = addr}
        )
  DrDrainOut ->
    let (enc', (_, mOut, encDone)) = cobsEncodeStep (lEnc s) (Nothing, txReady inp)
     in (s{lEnc = enc', lDrn = if encDone then DrDelim else DrDrainOut}, idleOut{txByte = mOut})
  DrDelim ->
    if txReady inp
      then (resetFrame s{lPhase = RxControl}, idleOut{txByte = Just 0})
      else (s, idleOut{txByte = Nothing})
 where
  afterWordByte i s'
    | lWIx s' /= 3 = s'{lWIx = lWIx s' + 1}
    | lTerm s' = s'{lDrn = DrCrcByte}
    | lDrCnt s' + 1 >= ringPtrIn i = s'{lTerm = True, lDrn = DrFetch}
    | otherwise = s'{lDrCnt = lDrCnt s' + 1, lDrn = DrFetch}

-- | Present a logical byte to the encoder; when consumed (readyIn), fold it into
-- the drain CRC (except the CRC byte itself, flagged @lst@) and advance the
-- generator. Route the encoder's output to txByte.
feedByte ::
  LoaderSt -> LoaderIn -> BitVector 8 -> Bool -> (LoaderSt -> LoaderSt) -> (LoaderSt, LoaderOut)
feedByte s inp b lst advance =
  let (enc', (readyIn, mOut, _)) = cobsEncodeStep (lEnc s) (Just (b, lst), txReady inp)
      s1 = s{lEnc = enc'}
      s2 =
        if readyIn
          then advance s1{lCrcTx = if lst then lCrcTx s1 else crc8Update (lCrcTx s1) b}
          else s1
   in (s2, idleOut{txByte = mOut})

-- | The little-endian byte @i@ (0..3) of a 32-bit word.
leByte :: BitVector 32 -> Unsigned 2 -> BitVector 8
leByte w i = case i of
  0 -> slice d7 d0 w
  1 -> slice d15 d8 w
  2 -> slice d23 d16 w
  _ -> slice d31 d24 w
```

- [ ] **Step 4: Run to verify it passes** (author).

Run: `stack test --test-arguments '-p "/Loader TX/"'`
Expected: PASS (drain == `encodeResult`, minimal drain, and byte-identical under backpressure).

- [ ] **Step 5: Format and commit.**

```bash
make format
git add src/Tamal/Loader.hs tests/Test/Loader.hs
git commit -m "feat(hdl): loader drain path (ring sweep + streaming COBS encode + delimiter)"
```

> Mentoring note: the drain is two-rate — the generator fills the encoder (BRAM-paced) while the encoder emits to the UART (paced by `txReady`). Because `readyIn` is high only while the encoder is filling, `feedByte` advances (and folds CRC) exactly when a byte is consumed, so no byte is dropped or double-counted under backpressure — that is what the `txReady` pattern test proves. The ring read is `DrFetch` (drive `ringAddr`) then `DrLatch` (capture `ringData` one cycle later), matching the BRAM's 1-cycle latency.

---

## Task 6: Robustness + integration + PLAN update

**Files:**
- Test: `hdl/tests/Test/Loader.hs`
- Modify: `hdl/PLAN.md`

- [ ] **Step 1: Write the robustness + end-to-end tests** (assistant). Add a `robustTests` group and include it (`, robustTests`). These reuse the harnesses already in the module:

```haskell
robustTests :: TestTree
robustTests =
  testGroup
    "Loader robustness + lifecycle"
    [ testProperty "single-byte corruption of a TRIGGER never triggers a run" $ property $ do
        -- TRIGGER is the frame that *would* pulse startOut; corrupting any byte
        -- (bad COBS/CRC, or an early 0x00) must be discarded (D4/D5) -> no pulse.
        let frame = encodeControl Trigger
        i <- forAll (Gen.int (Range.linear 0 (L.length frame - 1)))
        let frame' = [if j == i then b `xor` 1 else b | (j, b) <- L.zip [0 :: Int ..] frame]
        L.filter id (simStartOut (fmap Just frame')) === []
    , testCase "a clean TRIGGER still pulses exactly once (control)" $
        L.length (L.filter id (simStartOut (fmap Just (encodeControl Trigger)))) @?= 1
    , testCase "over-long LOAD saturates the write address at 1023" $ do
        let ws = L.map fromIntegral [1 .. 1100 :: Int] :: [BitVector 32]
            writes = simInstrWr (fmap Just (encodeControl (LoadProgram ws)))
        L.length writes @?= 1024
        fmap fst writes @?= [0 .. 1023]
    , testCase "two LOADs each write from address 0 (overwrite)" $ do
        let ws1 = [0x1111_1111, 0x2222_2222] :: [BitVector 32]
            ws2 = [0xAAAA_AAAA] :: [BitVector 32]
            bytes = encodeControl (LoadProgram ws1) <> encodeControl (LoadProgram ws2)
        simInstrWr (fmap Just bytes)
          @?= [(0, 0x1111_1111), (1, 0x2222_2222), (0, 0xAAAA_AAAA)]
    , testCase "re-runnable: two TRIGGER/halt cycles drain twice" $ do
        -- trig at 0..k, then (after drain1) trig again at cycle 1000; halted high
        -- from cycle 30. Drains are identical => stream is encodeResult twice.
        let recs = [0x0001_0000, 0x00AB_00CD] :: [BitVector 32]
            term = 0xC000_0000 :: BitVector 32
        simDrainTwice recs term @?= (encodeResult (recs <> [term]) <> encodeResult (recs <> [term]))
    ]

-- Like simDrain, but feeds a second TRIGGER at cycle 1000 (well after drain1
-- completes) with halted held high, so the loader drains twice. Reuses drainRig
-- and ringModel from Task 5.
simDrainTwice :: [BitVector 32] -> BitVector 32 -> [BitVector 8]
simDrainTwice records term =
  mapMaybe id
    $ sampleN
      2500
      ( drainRig
          (ringModel records term)
          (fromIntegral (L.length records))
          (fromList rxs)
          (pure True)
          (fromList halteds) ::
          Signal Dom100 (Maybe (BitVector 8))
      )
 where
  trig = encodeControl Trigger
  rxs =
    fmap Just trig
      <> L.replicate (1000 - L.length trig) Nothing
      <> fmap Just trig
      <> L.repeat Nothing
  halteds = L.replicate 30 False <> L.repeat True
```

- [ ] **Step 2: Run to verify** (assistant/author).

Run: `stack test --test-arguments '-p "/Loader robustness/"'`
Expected: PASS (corruption never triggers; overflow caps at 1023; overwrite; two drains).

- [ ] **Step 3: Run the whole suite** (author).

Run: `stack test`
Expected: PASS — all groups (`Crc`, `Isa`, `Config`, `Serdes`, `Trace`, `Branch`, `Alu`, `RegFile`, `Uart`, `Engine`, `Mem`, `Wire`, `Loader`).

- [ ] **Step 4: Clash codegen smoke** — `Tamal.Loader` is synthesizable but not yet in `topEntity`; this confirms it compiles under the Clash executable path.

Run: `stack run clash -- Tamal --verilog`
Expected: succeeds (generates `verilog/Tamal.topEntity/`; the placeholder heartbeat top is still the entity — the loader is wired to pins in piece 5).

- [ ] **Step 5: Format check.**

Run: `make format-check`
Expected: exit 0 (no diffs).

- [ ] **Step 6: Update `hdl/PLAN.md`** (author):
  - In the status table, add rows: `Tamal.Loader.Cobs` — streaming COBS decode/encode step functions — **done, tested**; `Tamal.Loader` — UART load/drain lifecycle FSM (`RxControl→Run→Drain`) — **done, tested**. Update the `Tamal` (top) row's note to mention the loader now exists (still placeholder heartbeat until piece 5).
  - In "What remains" and the ordering list, mark item 7 (**Loader**) done and set item 8 (**IOBUF**) as `← next`.
  - Note the one engine change: `BusOut.ringPtrOut` now exposes the ring depth for the drain (spec §4).

- [ ] **Step 7: Commit.**

```bash
make format
git add tests/Test/Loader.hs PLAN.md
git commit -m "docs(hdl): loader (Tamal.Loader) done + tested; IOBUF is next"
```

> Mentoring note: the overflow test feeds 1100 words and expects exactly 1024 writes at addresses `[0..1023]` — the `lFull` latch drops the rest without wrapping the `Unsigned 10` counter. The re-run test spaces the second `TRIGGER` far past drain 1 (cycle 1000) so the schedule is deterministic without needing to detect drain completion.

---

## Notes for the implementer

- **The codec is pure step functions, not Signal blocks.** `Tamal.Loader.Cobs` exports `cobsDecodeStep`/`cobsEncodeStep :: st -> i -> (st, o)`. This is the spec's "isolated streaming codec" realized without Signal-feedback plumbing: the loader's `mealy` calls them inside `loaderStep`, and the tests iterate them purely against the `Tamal.Wire.Cobs` oracle. Both were validated out-of-band (edge vectors, 400 randomized zero-dense inputs each, the 254/255 boundary, malformed detection, and a full streaming round-trip) before this plan.
- **`readyIn` means "consumed this cycle."** The encoder returns `readyIn = True` only in `EFilling` (the one mode that consumes input). `feedByte` relies on this to fold CRC + advance the generator exactly once per byte — do not "optimize" `readyIn` to be high on the Emitting→Filling transition cycle (it does not consume there).
- **Port ownership (no BRAM collisions).** The loader drives the instr-BRAM *write* port (`instrWr`) and the ring-BRAM *read* address (`ringAddr`); the engine drives the instr *read* (`pcOut`) and ring *write* (`Maybe Ring`). Loads happen only in `RxControl`, drains only in `Drain` — never overlapping the engine's run-time accesses.
- **`mealy` has no output register.** Unlike the `blockRam` tests (`Test.Mem`), there is no undefined cycle-0 output to `L.drop 1`; `sampleN` from cycle 0 is valid.
- **Lists vs `Vec`.** In `Test/Loader.hs` qualify list ops as `L.*`. In `src`, the only `Vec` is the encoder's `eBuf :: Vec 254` (with `replace`/`(!!)`), and `slice d7 d0` in `leByte`; everything else is `Maybe`/tuples/`Unsigned`/`BitVector`.
- **Not in `topEntity` yet.** The codegen smoke (Task 6) only compiles the library; the loader is wired to the UART, BRAMs, engine, and IOBUFs in piece 5.

---

## Execution note

Tasks are strictly ordered by dependency: the engine projection (1) and the two codec steps (2, 3) before the loader that embeds them (4, 5), and robustness/integration last (6). Each task is self-contained (its own red → green → refactor → commit) and leaves `stack test` green, so they are **not** parallelizable — run them in sequence.
