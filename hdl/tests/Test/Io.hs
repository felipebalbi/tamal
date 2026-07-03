-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0
{-# LANGUAGE NumericUnderscores #-}

module Test.Io (tests) where

import Clash.Prelude
import qualified Data.List as L
import qualified Hedgehog as H
import qualified Hedgehog.Gen as Gen
import qualified Hedgehog.Range as Range
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)

import Tamal.Bus.Serdes (Lanes, hiZ, serializeX1)
import Tamal.Domain (Dom100)
import Tamal.Io (alertSync, espiPads)

--------------------------------------------------------------------------------
-- alertSync (2-flop synchronizer)
--------------------------------------------------------------------------------

{- | Reference: a 2-flop synchronizer delays @raw@ by exactly two cycles, led by
two idle-high samples (the init-high flops before the input propagates). With
'simAlert' leading the stream by one idle cycle (below), the clean model holds:
@1 : 1 : xs@.
-}
refAlert :: [Bit] -> [Bit]
refAlert xs = L.take (L.length xs) (1 : 1 : xs)

{- | Sample 'alertSync' over a raw ALERT# stream. 'sampleN' asserts the Dom100
async reset on cycle 0, which swallows the first input sample and adds a lead
reset sample (see @hdl/PLAN.md@). So we feed one throwaway idle-high cycle up
front and 'L.drop 1' it — the repo's standard @sampleN@ idiom — leaving the real
stream aligned to the clean 'refAlert' model.
-}
simAlert :: [Bit] -> [Bit]
simAlert xs =
  L.drop 1
    $ sampleN
      (L.length xs + 1)
      (alertSync (fromList (1 : xs <> L.repeat 1)) :: Signal Dom100 Bit)

--------------------------------------------------------------------------------
-- espiPads harnesses
--
-- NB: feeding espiPads's own drive-side 'outs' back into the 'padsIn' it also
-- reads makes the Clash BiSignal loopback diverge (self-drive + self-read knot).
-- The engine never drives and samples one lane in the same cycle, so we mirror
-- that: two SINGLE-driver nets. 'simSample' has espiPads read a DUT-driven net
-- (drivers external, espiPads is a pure consumer); 'simDrive' has espiPads drive
-- a throwaway net and routes its 'outs' to a FRESH reader (linear, no knot).
--------------------------------------------------------------------------------

{- | A never-driven pull-up net (reads idle-high 1); used where espiPads must be
handed a read-side it does not actually depend on.
-}
idleNet :: Vec 4 (BiSignalIn 'PullUp Dom100 1)
idleNet = repeat (veryUnsafeToBiSignalIn (mempty :: BiSignalOut 'PullUp Dom100 1))

{- | Sample direction: all four lanes tri-stated on our side; a per-lane DUT
drives the net; espiPads reads it into @ioIn@. The DUT drivers are bound to the
throwaway 'idleNet' pad (only @seq@'d by 'writeToBiSignal', never affecting the
driven value), so @padsIn@ is not built from a driver that references it — no
self-referential loopback, which otherwise diverges on tri-stated (Nothing) lanes.
-}
simSample :: [Vec 4 (Maybe Bit)] -> [Vec 4 Bit]
simSample dutL = sampleN (L.length dutL) go
 where
  go :: (HiddenClockResetEnable Dom100) => Signal Dom100 (Vec 4 Bit)
  go = ioIn
   where
    dutS = unbundle (fromList (dutL <> L.repeat (repeat Nothing)))
    (_outs, _, _, _, ioIn, _) = espiPads (pure hiZ) (pure 0) (pure 0) (pure 1) (pure 1) padsIn
    dutOuts = zipWith writeToBiSignal idleNet dutS
    padsIn = map veryUnsafeToBiSignalIn dutOuts

{- | Drive direction: espiPads drives per @lanes@; a FRESH reader observes each
pad. espiPads reads a throwaway 'idleNet', so its drive-side never feeds its own
read-side. An @oe=0@ lane is tri-stated, so the reader sees the pull-up (1).
-}
simDrive :: [Lanes] -> [Vec 4 Bit]
simDrive lanesL = sampleN (L.length lanesL) go
 where
  go :: (HiddenClockResetEnable Dom100) => Signal Dom100 (Vec 4 Bit)
  go = bundle (map readFromBiSignal readNet)
   where
    lanesS = fromList (lanesL <> L.repeat hiZ)
    (outs, _, _, _, _ioIn, _) = espiPads lanesS (pure 0) (pure 0) (pure 1) (pure 1) idleNet
    readNet = map veryUnsafeToBiSignalIn outs

{- | Sideband outputs pass straight through espiPads (combinational). espiPads
reads an 'idleNet' (pure consumer), so there is no loopback.
-}
simSide :: [(Bit, Bit, Bit)] -> [(Bit, Bit, Bit)]
simSide sideL = sampleN (L.length sideL) go
 where
  go :: (HiddenClockResetEnable Dom100) => Signal Dom100 (Bit, Bit, Bit)
  go = bundle (csO, sckO, rstO)
   where
    (csS, sckS, rstS) = unbundle (fromList (sideL <> L.repeat (0, 0, 1)))
    (_outs, csO, sckO, rstO, _ioIn, _) = espiPads (pure hiZ) csS sckS rstS (pure 1) idleNet

-- | Sample oracle: each lane reads the DUT value, or the pull-up idle-high (1).
sampleOracle :: Vec 4 (Maybe Bit) -> Vec 4 Bit
sampleOracle = map (maybe 1 id)

-- | Drive oracle: each lane carries @o@ when @oe=1@, else the pull-up (1).
driveOracle :: Lanes -> Vec 4 Bit
driveOracle = map (\(o, oe) -> if oe == 1 then o else 1)

--------------------------------------------------------------------------------
-- generators
--------------------------------------------------------------------------------

-- | A 0/1 'Bit' generator.
genBit :: H.Gen Bit
genBit = Gen.element [0, 1]

-- | Random per-lane drive: four @(value, output-enable)@ pairs.
genLanes :: H.Gen Lanes
genLanes =
  (\a b c d -> a :> b :> c :> d :> Nil) <$> lane <*> lane <*> lane <*> lane
 where
  lane = (,) <$> genBit <*> genBit

-- | Random DUT drive: four @Maybe Bit@ (Nothing = the DUT tri-states that lane).
genDut :: H.Gen (Vec 4 (Maybe Bit))
genDut =
  (\a b c d -> a :> b :> c :> d :> Nil) <$> mb <*> mb <*> mb <*> mb
 where
  mb = Gen.maybe genBit

--------------------------------------------------------------------------------
tests :: TestTree
tests =
  testGroup
    "Io"
    [ -- alertSync
      testCase "alertSync: 2-cycle lag, idle-high lead"
        $ simAlert [1, 1, 0, 0, 0, 1, 1]
        @?= [1, 1, 1, 1, 0, 0, 0]
    , testCase "alertSync: single-cycle assertion survives"
        $ simAlert [1, 1, 0, 1, 1, 1]
        @?= [1, 1, 1, 1, 0, 1]
    , testProperty "alertSync: matches the 2-cycle-delay model" $ H.property $ do
        xs <- H.forAll (Gen.list (Range.linear 0 48) genBit)
        simAlert xs H.=== refAlert xs
    , -- sideband pass-through
      testProperty "sideband: CS#/SCK/RESET# pass through combinationally" $ H.property $ do
        xs <- H.forAll (Gen.list (Range.linear 1 48) ((,,) <$> genBit <*> genBit <*> genBit))
        simSide xs H.=== xs
    , -- sample direction (espiPads reads a DUT-driven net)
      testCase "io: we tri-state, DUT drives -> sample the DUT"
        $ L.head (simSample [Nothing :> Just 0 :> Nothing :> Nothing :> Nil])
        @?= (1 :> 0 :> 1 :> 1 :> Nil)
    , testCase "io: nobody drives -> pull-up idle-high"
        $ L.head (simSample [repeat Nothing])
        @?= (1 :> 1 :> 1 :> 1 :> Nil)
    , testProperty "io: sample reads DUT value or pull-up (per lane)" $ H.property $ do
        dutL <- H.forAll (Gen.list (Range.linear 1 32) genDut)
        simSample dutL H.=== fmap sampleOracle dutL
    , -- drive direction (espiPads drives; fresh reader observes)
      testCase "io: we drive (oe=1) -> pad carries our value; oe=0 -> hi-Z pull-up"
        $ L.head (simDrive [(0, 1) :> (0, 0) :> (0, 0) :> (0, 0) :> Nil])
        @?= (0 :> 1 :> 1 :> 1 :> Nil)
    , testCase "io: x1 beat0 drives IO[0] only, IO[1..3] hi-Z (independent OE)"
        $ let byte = 0b0111_1111 :: BitVector 8 -- MSB (IO[0]) = 0
              lane0 = serializeX1 byte !! (0 :: Index 8)
           in L.head (simDrive [lane0]) @?= (0 :> 1 :> 1 :> 1 :> Nil)
    , testProperty "io: drive puts o on oe=1 lanes, pull-up on oe=0 lanes" $ H.property $ do
        lanesL <- H.forAll (Gen.list (Range.linear 1 32) genLanes)
        simDrive lanesL H.=== fmap driveOracle lanesL
    ]
