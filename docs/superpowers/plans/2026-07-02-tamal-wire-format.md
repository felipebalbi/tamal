# Tamal Wire Format Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the pure Clash wire-format core — `Tamal.Wire.Cobs` (COBS encode/decode) and `Tamal.Wire` (LE word↔bytes, CRC-8 fold, frame + message layer) — that encodes/decodes the control (`LOAD_PROGRAM`/`TRIGGER`) and result (`TRACE_DRAIN`) byte frames, hedgehog + HUnit tested.

**Architecture:** Three pure layers, each with its own round-trip property (spec §4). `Tamal.Wire.Cobs` is a dependency-free leaf (`cobsEncode`/`cobsDecode :: -> Maybe`). `Tamal.Wire` sits above it: `wordToBytesLE`/`bytesToWordLE`, a `crc8` fold reusing `Tamal.Crc`, a framing layer (`frameEncode`/`frameDecode` = COBS + CRC + `0x00` delimiter), and a message layer (`encodeControl`/`decodeControl`, `encodeResult`/`decodeResult`) over the `ControlMsg`/`WireError` ADTs. Everything is `[BitVector 8]` **list** reference model — like `Trace.encodeRecord`, not synthesizable; the streaming realization is the piece-3 loader.

**Tech Stack:** Clash 1.10 (`clash-prelude`), tasty + tasty-hunit + tasty-hedgehog, `clash-prelude-hedgehog`. Build/test with `cabal` from `hdl/`.

**Collaboration model (TDD ping-pong, per spec §9 / decision D9):** For each slice the **assistant writes the failing test** (the test code in this plan is authoritative and complete), the **author writes the Clash under `src/`** to make it pass (the implementation code blocks are a correct reference target — refine together in the green step), then both **refactor**. This is a Clash learning exercise.

**Spec:** `docs/superpowers/specs/2026-07-02-tamal-wire-format-design.md`. All section (§) references below point there.

**Conventions for every task**
- All commands run from `hdl/` (the Clash project root; use the tool's `workdir`).
- SPDX header on every new `.hs` file:
  ```haskell
  -- SPDX-FileCopyrightText: 2026 Felipe Balbi
  -- SPDX-License-Identifier: CERN-OHL-P-2.0
  ```
- `make format` before each commit; `cabal test` must stay green.
- **List gotcha:** `Clash.Prelude` re-exports `map`/`(++)`/`reverse`/`foldl'` as the **`Vec`** versions. For `[BitVector 8]` list work use `import qualified Data.List as L` (`L.length`, `L.foldl'`, `L.concatMap`, `L.last`, `L.init`, `L.null`, `L.elem`, `L.zip`), the list `(<>)`/`(:)`, and list comprehensions — never the bare `Vec` ones. `reverse`/`unpack`/`pack`/`toList` on a `Vec` come from `Clash.Prelude` unqualified.

---

## File Structure

- **Create** `hdl/src/Tamal/Wire/Cobs.hs` — `cobsEncode`, `cobsDecode` (dependency-free leaf; returns `Maybe`).
- **Create** `hdl/src/Tamal/Wire.hs` — `wordToBytesLE`/`bytesToWordLE`, `crc8`, `WireError`, `ControlMsg`, `frameEncode`/`frameDecode`, `encodeControl`/`decodeControl`, `encodeResult`/`decodeResult`.
- **Create** `hdl/tests/Test/Wire.hs` — all Cobs + Wire properties and HUnit vectors.
- **Modify** `hdl/tamal.cabal` — add `Tamal.Wire.Cobs` and `Tamal.Wire` to `exposed-modules`; add `Test.Wire` to the test-suite `other-modules`.
- **Modify** `hdl/tests/unittests.hs` — import and include `Test.Wire.tests`.
- **Modify** `hdl/PLAN.md` — mark piece 2 (wire protocol) done; loader is next (Task 7).

Leaves reused unchanged: `Tamal.Crc` (`crc8Update`). No `topEntity` change.

---

## Task 1: `Tamal.Wire` scaffold + little-endian word↔bytes

**Files:**
- Create: `hdl/src/Tamal/Wire.hs`
- Modify: `hdl/tamal.cabal` (add `Tamal.Wire` to `exposed-modules`, after `Tamal.Mem`; add `Test.Wire` to test `other-modules`, after `Test.Mem`)
- Create: `hdl/tests/Test/Wire.hs`
- Modify: `hdl/tests/unittests.hs`

- [ ] **Step 1: Create `hdl/src/Tamal/Wire.hs`** (author) with just the LE primitives (the frame/message layers arrive in later tasks):

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
The tamal wire format (design doc 2026-07-02-tamal-wire-format-design.md): the
transport-agnostic control/result byte framing. This module is the frame +
message layer over the 'Tamal.Wire.Cobs' leaf; it is a pure @[BitVector 8]@
reference model (not synthesizable — like 'Tamal.Trace.encodeRecord'). The
streaming realization is the piece-3 loader.
-}
module Tamal.Wire
  ( wordToBytesLE
  , bytesToWordLE
  ) where

import Clash.Prelude

-- | Split a 32-bit word into four little-endian bytes (LSB first; ISA §4).
-- @0xAABBCCDD -> <0xDD, 0xCC, 0xBB, 0xAA>@.
wordToBytesLE :: BitVector 32 -> Vec 4 (BitVector 8)
wordToBytesLE = reverse . unpack

-- | Reassemble a little-endian byte quartet into its 32-bit word (inverse of
-- 'wordToBytesLE').
bytesToWordLE :: Vec 4 (BitVector 8) -> BitVector 32
bytesToWordLE = pack . reverse
```

- [ ] **Step 2: Register the modules** (author). In `hdl/tamal.cabal`, add `Tamal.Wire` to `library`'s `exposed-modules` (after `Tamal.Mem`), and add `Test.Wire` to the `test-suite test-library` `other-modules` (after `Test.Mem`).

- [ ] **Step 3: Create `hdl/tests/Test/Wire.hs`** (assistant) with the LE tests:

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

module Test.Wire (tests) where

import Clash.Prelude
import qualified Data.List as L
import Hedgehog (forAll, property, (===))
import qualified Hedgehog.Gen as Gen
import qualified Hedgehog.Range as Range
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)

import Tamal.Wire
import Test.Gen (genWord)

tests :: TestTree
tests =
  testGroup
    "Wire"
    [ testCase "wordToBytesLE 0xAABBCCDD == [DD,CC,BB,AA]"
        $ toList (wordToBytesLE 0xAABBCCDD) @?= [0xDD, 0xCC, 0xBB, 0xAA]
    , testProperty "bytesToWordLE . wordToBytesLE == id" $ property $ do
        w <- forAll genWord
        bytesToWordLE (wordToBytesLE w) === w
    ]
```

- [ ] **Step 4: Wire into `unittests.hs`** (author). Add `import qualified Test.Wire` (after `import qualified Test.Uart`) and `, Test.Wire.tests` to the `tests` list (after `Test.Mem.tests`).

- [ ] **Step 5: Run the suite** (author).

Run: `cabal test --test-options '-p "/Wire/"'`
Expected: PASS (2 cases). Run `cabal build` too, to confirm the library compiles.

- [ ] **Step 6: Format and commit.**

```bash
make format
git add src/Tamal/Wire.hs tests/Test/Wire.hs tamal.cabal tests/unittests.hs
git commit -m "feat(hdl): Wire scaffold + little-endian word<->bytes"
```

---

## Task 2: `Tamal.Wire.Cobs` — `cobsEncode`

**Files:**
- Create: `hdl/src/Tamal/Wire/Cobs.hs`
- Modify: `hdl/tamal.cabal` (add `Tamal.Wire.Cobs` to `exposed-modules`, before `Tamal.Wire`)
- Test: `hdl/tests/Test/Wire.hs`

- [ ] **Step 1: Write the failing tests** (assistant). Add the `Tamal.Wire.Cobs` import and a zero-dense byte generator, then the encode vectors + no-zero invariant. In `Test/Wire.hs` add the import and widen the `Test.Gen` import to include `genByte`:

```haskell
import Tamal.Wire.Cobs (cobsEncode)
import Test.Gen (genByte, genWord)
```

Add near the top of the module (after `tests`):

```haskell
-- A zero-dense byte generator: stresses COBS group boundaries far harder than
-- the ~1/256 zeros a uniform generator produces. Reuses Test.Gen's genByte
-- (genDefinedBitVector) so no Integral/Bounded instance on BitVector is assumed.
genByteZeros :: Gen.Gen (BitVector 8)
genByteZeros = Gen.frequency [(1, pure 0), (4, genByte)]

-- 254 distinct non-zero bytes (1..254) — the maximal COBS group.
run254 :: [BitVector 8]
run254 = L.map fromIntegral [1 .. 254 :: Int]

-- 255 non-zero bytes — one past the maximal group (forces a second group).
run255 :: [BitVector 8]
run255 = L.map fromIntegral [1 .. 255 :: Int]
```

Add these cases to the `testGroup` list:

```haskell
    , testCase "cobsEncode [0x00] == [0x01,0x01]"
        $ cobsEncode [0x00] @?= [0x01, 0x01]
    , testCase "cobsEncode [11,22,00,33] == [03,11,22,02,33]"
        $ cobsEncode [0x11, 0x22, 0x00, 0x33] @?= [0x03, 0x11, 0x22, 0x02, 0x33]
    , testCase "cobsEncode [11,00,00,00] == [02,11,01,01,01]"
        $ cobsEncode [0x11, 0x00, 0x00, 0x00] @?= [0x02, 0x11, 0x01, 0x01, 0x01]
    , testCase "cobsEncode [] == [0x01]"
        $ cobsEncode [] @?= [0x01]
    , testCase "cobsEncode 254 non-zero bytes == 0xFF-led, no trailing 0x01"
        $ cobsEncode run254 @?= (0xFF : run254)
    , testCase "cobsEncode 255 non-zero bytes == 0xFF group + 0x02 group"
        $ cobsEncode run255 @?= (0xFF : run254) <> [0x02, 255]
    , testProperty "cobsEncode output contains no 0x00" $ property $ do
        xs <- forAll (Gen.list (Range.linear 0 300) genByteZeros)
        L.elem 0 (cobsEncode xs) === False
```

> Mentoring note: folding `pure 0` into `genByte` at 1:4 makes zeros common, exercising group boundaries. `run254` is built via `Int` then `fromIntegral` because `BitVector` has no convenient `Enum`.

- [ ] **Step 2: Run to verify it fails** (assistant).

Run: `cabal test --test-options '-p "/Wire/"'`
Expected: FAIL — `Tamal.Wire.Cobs` does not exist / `cobsEncode` not in scope (module won't compile).

- [ ] **Step 3: Create `hdl/src/Tamal/Wire/Cobs.hs`** (author) with `cobsEncode`:

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
Consistent Overhead Byte Stuffing (spec §5): removes every @0x00@ from a byte
sequence so a single @0x00@ can delimit frames. A dependency-free leaf — it
imports no other @Tamal.Wire.*@ module, and 'cobsDecode' reports malformed input
as 'Nothing' (the frame layer lifts that to @BadCobs@). This is a pure
@[BitVector 8]@ reference model; the loader implements the streaming form.
-}
module Tamal.Wire.Cobs
  ( cobsEncode
  ) where

import Clash.Prelude
import qualified Data.List as L

{- | COBS-encode a byte sequence into groups: each group is a code byte
@(len+1)@ followed by up to 254 non-zero data bytes. A @0x00@ closes a group
(consumed); a group full at 254 bytes is flushed (code @0xFF@). The output never
contains @0x00@ and does NOT include the frame delimiter (the frame layer appends
it).
-}
cobsEncode :: [BitVector 8] -> [BitVector 8]
cobsEncode = go []
 where
  go grp [] = emit grp
  go grp (b : bs)
    | b == 0 = emit grp <> go [] bs -- zero terminates the group (implied)
    | L.length grp == 254 = emit grp <> go [b] bs -- group full: flush, restart with b
    | otherwise = go (grp <> [b]) bs
  emit grp = fromIntegral (L.length grp + 1) : grp
```

(`cobsDecode` is added in Task 3, which widens this export list to `( cobsEncode, cobsDecode )`. The test in this task imports only `cobsEncode`.)

- [ ] **Step 4: Register the module** (author). In `hdl/tamal.cabal`, add `Tamal.Wire.Cobs` to `exposed-modules`, immediately **before** `Tamal.Wire`.

- [ ] **Step 5: Run to verify it passes** (author).

Run: `cabal test --test-options '-p "/Wire/"'`
Expected: PASS (LE cases + 5 encode vectors + no-zero property).

- [ ] **Step 6: Format and commit.**

```bash
make format
git add src/Tamal/Wire/Cobs.hs tests/Test/Wire.hs tamal.cabal
git commit -m "feat(hdl): COBS encode (Tamal.Wire.Cobs)"
```

---

## Task 3: `cobsDecode` + the round-trip law

**Files:**
- Modify: `hdl/src/Tamal/Wire/Cobs.hs` (add `cobsDecode`, widen exports)
- Test: `hdl/tests/Test/Wire.hs`

- [ ] **Step 1: Write the failing tests** (assistant). Extend the import and add the round-trip + malformed cases. Change the import in `Test/Wire.hs` to:

```haskell
import Tamal.Wire.Cobs (cobsDecode, cobsEncode)
```

Add to the `testGroup` list:

```haskell
    , testProperty "cobsDecode . cobsEncode == Just" $ property $ do
        xs <- forAll (Gen.list (Range.linear 0 300) genByteZeros)
        cobsDecode (cobsEncode xs) === Just xs
    , testCase "cobsDecode truncated group -> Nothing"
        $ cobsDecode [0x05, 0x11] @?= Nothing
    , testCase "cobsDecode interior zero -> Nothing"
        $ cobsDecode [0x03, 0x11, 0x00] @?= Nothing
    , testCase "cobsDecode [] -> Nothing"
        $ cobsDecode [] @?= (Nothing :: Maybe [BitVector 8])
```

- [ ] **Step 2: Run to verify it fails** (assistant).

Run: `cabal test --test-options '-p "/Wire/"'`
Expected: FAIL — `cobsDecode` not in scope (not yet exported/defined).

- [ ] **Step 3: Add `cobsDecode`** (author). Widen the export list to `( cobsEncode, cobsDecode )` and add:

```haskell
{- | COBS-decode a delimiter-stripped byte sequence, or 'Nothing' if malformed
(a code byte demanding more bytes than remain, any @0x00@ in the input, or an
empty list). Inverse of 'cobsEncode': @cobsDecode (cobsEncode x) == Just x@.
-}
cobsDecode :: [BitVector 8] -> Maybe [BitVector 8]
cobsDecode bytes
  | L.null bytes = Nothing -- empty is not a valid frame
  | L.elem 0 bytes = Nothing -- a valid (delimiter-stripped) frame has no 0x00
  | otherwise = go bytes
 where
  go [] = Just [] -- unreachable given the guards, but keeps `go` total
  go (code : rest) =
    let n = fromIntegral code - 1 -- data bytes in this group
     in do
          (grp, more) <- takeExactly n rest
          if L.null more
            then Just grp -- final group: no trailing zero
            else do
              decoded <- go more
              -- a non-full group (<255) had a zero the encoder consumed
              Just (grp <> (if code == 255 then [] else [0]) <> decoded)

-- | Split off exactly @k@ elements, or 'Nothing' if fewer than @k@ remain.
takeExactly :: Int -> [a] -> Maybe ([a], [a])
takeExactly 0 xs = Just ([], xs)
takeExactly _ [] = Nothing
takeExactly k (x : xs) = do
  (ys, zs) <- takeExactly (k - 1) xs
  Just (x : ys, zs)
```

> Mentoring note: the `L.elem 0 bytes` guard makes the interior-zero rejection explicit and matches the spec (§5.1) — since `cobsEncode` never emits `0x00`, any zero in the decode input is malformed. `takeExactly` is where truncated groups become `Nothing`.

- [ ] **Step 4: Run to verify it passes** (author).

Run: `cabal test --test-options '-p "/Wire/"'`
Expected: PASS (round-trip over zero-dense inputs + 3 malformed cases).

- [ ] **Step 5: Format and commit.**

```bash
make format
git add src/Tamal/Wire/Cobs.hs tests/Test/Wire.hs
git commit -m "feat(hdl): COBS decode + round-trip law (Tamal.Wire.Cobs)"
```

---

## Task 4: `crc8` fold, `WireError`, and the framing layer

**Files:**
- Modify: `hdl/src/Tamal/Wire.hs` (add `WireError`, `crc8`, `frameEncode`, `frameDecode`; import `Cobs` + `Crc`)
- Test: `hdl/tests/Test/Wire.hs`

- [ ] **Step 1: Write the failing tests** (assistant). `Tamal.Wire` is imported unqualified, so it already brings in `crc8`/`frameEncode`/`frameDecode`/`WireError (..)` once this task's step 3 defines them — no import change needed there. `genByte`/`genWord` are already imported (Task 2). Only add `(/==)` to the Hedgehog import:

```haskell
import Hedgehog (forAll, property, (/==), (===))
```

Add to the `testGroup` list:

```haskell
    , testCase "crc8 matches CRC-8/SMBUS check vector (0xF4)"
        $ crc8 [fromIntegral (fromEnum c) | c <- "123456789"] @?= 0xF4
    , testProperty "frameDecode . frameEncode == Right" $ property $ do
        xs <- forAll (Gen.list (Range.linear 0 64) genByte)
        frameDecode (frameEncode xs) === Right xs
    , testCase "frame ends in exactly one 0x00, none interior" $ do
        let f = frameEncode [0x01, 0x00, 0x02]
        L.last f @?= 0
        L.length (L.filter (== 0) f) @?= 1
    , testProperty "single-byte corruption is never a silent success" $ property $ do
        xs <- forAll (Gen.list (Range.linear 1 32) genByte)
        let f = frameEncode xs
        i <- forAll (Gen.int (Range.linear 0 (L.length f - 1)))
        let f' = [if j == i then x `xor` 1 else x | (j, x) <- L.zip [0 ..] f]
        frameDecode f' /== Right xs
```

> Mentoring note (why the corruption property is deterministic): flipping any one byte either breaks the trailing `0x00` delimiter (→ `Left`), changes the COBS structure (→ `Left` or different content), or changes a payload/CRC byte (→ different content or `BadCrc`). None can reproduce the *exact* original logical bytes, so `/== Right xs` holds every time — no flaky CRC-collision edge.

- [ ] **Step 2: Run to verify it fails** (assistant).

Run: `cabal test --test-options '-p "/Wire/"'`
Expected: FAIL — `crc8`/`frameEncode`/`frameDecode`/`WireError` not in scope.

- [ ] **Step 3: Implement the framing layer** (author). Update `Tamal.Wire`'s imports, module export list, and add the definitions:

```haskell
module Tamal.Wire
  ( WireError (..)
  , wordToBytesLE
  , bytesToWordLE
  , crc8
  , frameEncode
  , frameDecode
  ) where

import Clash.Prelude
import qualified Data.List as L

import Tamal.Crc (crc8Update)
import Tamal.Wire.Cobs (cobsDecode, cobsEncode)
```

```haskell
-- | Why a frame failed to decode.
data WireError
  = BadCrc -- ^ CRC byte did not match the recomputed CRC
  | BadCobs -- ^ malformed COBS or missing delimiter
  | UnknownOpcode (BitVector 8) -- ^ opcode not in the v1 set
  | ShortFrame -- ^ decoded frame lacks an opcode and/or CRC byte
  | BadPayloadLen -- ^ payload length invalid for the opcode
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

-- | Fold the CRC-8 (poly 0x07, init 0x00, MSB-first — reuses 'Tamal.Crc') over a
-- whole byte sequence.
crc8 :: [BitVector 8] -> BitVector 8
crc8 = L.foldl' crc8Update 0

-- | Wrap a logical frame (opcode ++ payload) for the wire: append its CRC,
-- COBS-encode, then append the 0x00 delimiter (§4, §8).
frameEncode :: [BitVector 8] -> [BitVector 8]
frameEncode logical = cobsEncode (logical <> [crc8 logical]) <> [0]

-- | Unwrap a wire frame back to its logical bytes: strip the delimiter,
-- COBS-decode, split off the trailing CRC byte, and verify it.
frameDecode :: [BitVector 8] -> Either WireError [BitVector 8]
frameDecode wire = do
  stripped <- stripDelim wire
  content <- maybe (Left BadCobs) Right (cobsDecode stripped)
  (logical, crc) <- splitLastByte content
  if crc8 logical == crc then Right logical else Left BadCrc

-- | Remove the trailing 0x00 delimiter (or fail).
stripDelim :: [BitVector 8] -> Either WireError [BitVector 8]
stripDelim xs
  | L.null xs = Left ShortFrame
  | L.last xs == 0 = Right (L.init xs)
  | otherwise = Left BadCobs

-- | Separate a non-empty byte list into its leading bytes and its last byte.
splitLastByte :: [BitVector 8] -> Either WireError ([BitVector 8], BitVector 8)
splitLastByte xs
  | L.null xs = Left ShortFrame
  | otherwise = Right (L.init xs, L.last xs)
```

> Mentoring note: `frameDecode (frameEncode xs) == Right xs` for any `xs`, including `[]` (there `logical = []`, `crc8 [] = 0`, and the round-trip still holds). The `NFDataX` derivation on `WireError` matches the codebase convention (`Trace.Record`); a `BitVector 8` field is fine for it.

- [ ] **Step 4: Run to verify it passes** (author).

Run: `cabal test --test-options '-p "/Wire/"'`
Expected: PASS (CRC vector, frame round-trip, delimiter invariant, corruption).

- [ ] **Step 5: Format and commit.**

```bash
make format
git add src/Tamal/Wire.hs tests/Test/Wire.hs
git commit -m "feat(hdl): Wire framing layer (CRC-8 + COBS + delimiter) and WireError"
```

---

## Task 5: Control messages — `encodeControl` / `decodeControl`

**Files:**
- Modify: `hdl/src/Tamal/Wire.hs` (add `ControlMsg`, opcodes, `bytesToWords`, `encodeControl`, `decodeControl`)
- Test: `hdl/tests/Test/Wire.hs`

- [ ] **Step 1: Write the failing tests** (assistant). Add to the `testGroup` list:

```haskell
    , testProperty "decodeControl . encodeControl (LoadProgram) == Right" $ property $ do
        ws <- forAll (Gen.list (Range.linear 0 32) genWord)
        decodeControl (encodeControl (LoadProgram ws)) === Right (LoadProgram ws)
    , testCase "decodeControl . encodeControl Trigger == Right Trigger"
        $ decodeControl (encodeControl Trigger) @?= Right Trigger
    , testCase "unknown opcode -> UnknownOpcode"
        $ decodeControl (frameEncode [0x7E, 0xAA]) @?= Left (UnknownOpcode 0x7E)
    , testCase "LOAD payload not a multiple of 4 -> BadPayloadLen"
        $ decodeControl (frameEncode [0x01, 0xAA, 0xBB]) @?= Left BadPayloadLen
    , testCase "empty logical frame -> ShortFrame"
        $ decodeControl (frameEncode []) @?= Left ShortFrame
```

- [ ] **Step 2: Run to verify it fails** (assistant).

Run: `cabal test --test-options '-p "/Wire/"'`
Expected: FAIL — `ControlMsg`/`encodeControl`/`decodeControl` not in scope.

- [ ] **Step 3: Implement the control message layer** (author). Add `ControlMsg (..)`, `encodeControl`, `decodeControl` to the export list, and:

```haskell
-- | Control-plane messages (host -> FPGA).
data ControlMsg
  = LoadProgram [BitVector 32] -- ^ instruction words to load into the instr BRAM
  | Trigger -- ^ start-of-run pulse
  deriving stock (Generic, Show, Eq)
  deriving anyclass (NFDataX)

-- Frame opcodes (§8.1). 0x00 is the delimiter, never an opcode.
opLoadProgram, opTrigger, opTraceDrain :: BitVector 8
opLoadProgram = 0x01
opTrigger = 0x02
opTraceDrain = 0x81

-- | Encode a control message to its wire frame.
encodeControl :: ControlMsg -> [BitVector 8]
encodeControl (LoadProgram ws) =
  frameEncode (opLoadProgram : L.concatMap (toList . wordToBytesLE) ws)
encodeControl Trigger = frameEncode [opTrigger]

-- | Decode a wire frame into a control message.
decodeControl :: [BitVector 8] -> Either WireError ControlMsg
decodeControl wire = do
  logical <- frameDecode wire
  case logical of
    [] -> Left ShortFrame
    (op : payload)
      | op == opLoadProgram -> LoadProgram <$> bytesToWords payload
      | op == opTrigger -> if L.null payload then Right Trigger else Left BadPayloadLen
      | otherwise -> Left (UnknownOpcode op)

-- | Regroup a byte payload into little-endian 32-bit words; fail unless the
-- length is a multiple of four.
bytesToWords :: [BitVector 8] -> Either WireError [BitVector 32]
bytesToWords [] = Right []
bytesToWords (a : b : c : d : rest) =
  (bytesToWordLE (a :> b :> c :> d :> Nil) :) <$> bytesToWords rest
bytesToWords _ = Left BadPayloadLen
```

> Mentoring note: `toList . wordToBytesLE` turns each word into its 4 LE bytes; `L.concatMap` flattens the program. `bytesToWords` consumes the payload four bytes at a time — a 1–3-byte remainder hits the `_` clause → `BadPayloadLen`. `Trigger` with a non-empty payload is also `BadPayloadLen` (the payload length is wrong for the opcode).

- [ ] **Step 4: Run to verify it passes** (author).

Run: `cabal test --test-options '-p "/Wire/"'`
Expected: PASS (control round-trips + error taxonomy).

- [ ] **Step 5: Format and commit.**

```bash
make format
git add src/Tamal/Wire.hs tests/Test/Wire.hs
git commit -m "feat(hdl): Wire control messages (LOAD_PROGRAM/TRIGGER) + error taxonomy"
```

---

## Task 6: Result frames — `encodeResult` / `decodeResult`

**Files:**
- Modify: `hdl/src/Tamal/Wire.hs` (add `encodeResult`, `decodeResult`)
- Test: `hdl/tests/Test/Wire.hs`

- [ ] **Step 1: Write the failing tests** (assistant). Add to the `testGroup` list:

```haskell
    , testProperty "decodeResult . encodeResult == Right" $ property $ do
        ws <- forAll (Gen.list (Range.linear 0 64) genWord)
        decodeResult (encodeResult ws) === Right ws
    , testCase "result frame round-trips a REVISION-led word stream"
        $ decodeResult (encodeResult [0x0001_0000, 0xAABBCCDD, 0xC000_0011])
        @?= Right [0x0001_0000, 0xAABBCCDD, 0xC000_0011]
    , testCase "control opcode is rejected by decodeResult"
        $ decodeResult (encodeControl Trigger) @?= Left (UnknownOpcode 0x02)
```

- [ ] **Step 2: Run to verify it fails** (assistant).

Run: `cabal test --test-options '-p "/Wire/"'`
Expected: FAIL — `encodeResult`/`decodeResult` not in scope.

- [ ] **Step 3: Implement the result layer** (author). Add `encodeResult`, `decodeResult` to the export list, and:

```haskell
-- | Encode a drained ring word-stream (REVISION ++ records ++ HALT terminator,
-- §8.3) as a TRACE_DRAIN wire frame.
encodeResult :: [BitVector 32] -> [BitVector 8]
encodeResult ws = frameEncode (opTraceDrain : L.concatMap (toList . wordToBytesLE) ws)

-- | Decode a TRACE_DRAIN wire frame back to its ring word-stream.
decodeResult :: [BitVector 8] -> Either WireError [BitVector 32]
decodeResult wire = do
  logical <- frameDecode wire
  case logical of
    [] -> Left ShortFrame
    (op : payload)
      | op == opTraceDrain -> bytesToWords payload
      | otherwise -> Left (UnknownOpcode op)
```

> Mentoring note: `encodeResult`/`decodeResult` reuse `bytesToWords` and the `0x81` opcode; the wire layer is word-agnostic (it neither inspects the REVISION word nor the HALT tag — that is the host's job). Feeding a control frame to `decodeResult` yields `UnknownOpcode 0x02`, confirming the direction opcodes are disjoint.

- [ ] **Step 4: Run to verify it passes** (author).

Run: `cabal test --test-options '-p "/Wire/"'`
Expected: PASS (result round-trips + cross-direction rejection).

- [ ] **Step 5: Format and commit.**

```bash
make format
git add src/Tamal/Wire.hs tests/Test/Wire.hs
git commit -m "feat(hdl): Wire result frames (TRACE_DRAIN word stream)"
```

---

## Task 7: Full suite green + Clash codegen smoke + PLAN update

**Files:**
- Modify: `hdl/PLAN.md`

- [ ] **Step 1: Run the whole test suite** (author/assistant).

Run: `cabal test`
Expected: PASS — all groups (`Crc`, `Isa`, `Config`, `Serdes`, `Trace`, `Branch`, `Alu`, `RegFile`, `Uart`, `Engine`, `Mem`, `Wire`).

- [ ] **Step 2: Clash codegen smoke** — the wire core is a pure `[BitVector 8]` reference model (not in `topEntity`, and list-returning like `Tamal.Trace`), so this only confirms the library still compiles under the Clash executable path.

Run: `cabal run clash -- Tamal --verilog`
Expected: succeeds (generates `verilog/Tamal.topEntity/`; the placeholder heartbeat top is still the entity — the loader wires the wire core in later).

- [ ] **Step 3: Format check.**

Run: `make format-check`
Expected: exit 0 (no diffs).

- [ ] **Step 4: Update `hdl/PLAN.md`** (author). Reflect that piece 2 is done:
  - In the status table, add rows: `Tamal.Wire.Cobs` — COBS encode/decode (delimiter framing) — **done, tested**; and `Tamal.Wire` — LE word↔bytes, CRC-8 fold, control/result frame + message layer — **done, tested**.
  - In "What remains" and the ordering list, mark item 6 (**Wire protocol**) done and set item 7 (**Loader**) as the next `← next`.
  - Change the section-2 heading from `### 2. Wire protocol — fill the tamal-abi placeholder (loader prerequisite)` to note it is now done in HDL (`Tamal.Wire`), and record that the Rust `tamal-abi` mirror is deferred (post-silicon) per the wire-format spec's scope.

- [ ] **Step 5: Commit.**

```bash
make format
git add PLAN.md
git commit -m "docs(hdl): wire format (Tamal.Wire) done + tested; loader is next"
```

---

## Notes for the implementer

- **`Tamal.Wire.Cobs` is dependency-free.** It must not import `Tamal.Wire` (or `WireError`); that would create an import cycle. Its only failure signal is `Nothing`; the frame layer maps it to `BadCobs`. This is deliberate (spec D7/§9).
- **Lists vs `Vec`.** Everything in these modules is a `[BitVector 8]`/`[BitVector 32]` **reference model**, mirroring `Tamal.Trace.encodeRecord`. `Clash.Prelude` shadows `map`/`(++)`/`reverse`/`foldl'` with `Vec` versions, so qualify list ops as `L.*` (`Data.List`). The only `Vec` uses are inside `wordToBytesLE`/`bytesToWordLE` (`reverse`/`unpack`/`pack`) and the `a :> b :> c :> d :> Nil` quartet in `bytesToWords` — those want the `Clash.Prelude` versions.
- **Not synthesizable, and that's fine.** Like `Trace`, these list functions are never instantiated in `topEntity`; the codegen smoke (Task 7) just compiles the library. The loader (piece 3) implements the streaming, synthesizable form that reproduces this model byte-for-byte.
- **COBS boundary cases.** The 254/255-byte group boundary is the subtle part (Task 2's `run254` vector pins it): exactly 254 non-zero bytes ⇒ a single `0xFF`-led group with **no** trailing `0x01`; 255 non-zero bytes ⇒ `0xFF`-group of 254 then a `0x02`-group of the last byte. If the round-trip property fails only on long inputs, this boundary is where to look.
- **`fromIntegral` widths.** In `cobsEncode`, `L.length grp + 1 ∈ 1..255` fits `BitVector 8`; in `cobsDecode`, `fromIntegral code - 1 ∈ 0..254` is an `Int` count for `takeExactly`.

---

## Execution note

Each task is self-contained (its own red → green → refactor → commit) and leaves `cabal test` green. Tasks are strictly ordered by dependency (`Cobs` before the framing layer; the framing layer before the message layers), so they are **not** parallelizable — run them in sequence.
