module Test.Isa (tests) where

import Clash.Prelude
import Test.Tasty
import Test.Tasty.HUnit
import Test.Tasty.Hedgehog (testProperty)
import Hedgehog (property, forAll, (===), success)

import Tamal.Isa
import Test.Gen (genBusInstr, genCtrlInstr, genDataInstr, genInstr, genWord)

tests :: TestTree
tests =
  testGroup "Isa"
    [ testProperty "BUS: decode . encode == Right" $ property $ do
        i <- forAll genBusInstr
        decode (encode i) === Right i
    , testProperty "any 32-bit word: decode is canonical or traps" $ property $ do
        -- Over ALL words (every group), not just the image of encode: any word
        -- decode accepts must re-encode to itself. This catches a too-lax
        -- reserved-field check in any group (spec §11.1).
        w <- forAll genWord
        case decode w of
          Right i -> encode i === w
          Left _  -> success
    , testProperty "CTRL: decode . encode == Right" $ property $ do
        i <- forAll genCtrlInstr
        decode (encode i) === Right i
    , testProperty "DATA: decode . encode == Right" $ property $ do
        i <- forAll genDataInstr
        decode (encode i) === Right i
    , testProperty "any valid instr: encode . decode == id" $ property $ do
        i <- forAll genInstr
        (encode <$> decode (encode i)) === Right (encode i)
    , testCase "reserved non-zero field traps (CS_ASSERT with junk imm)" $
        -- CS_ASSERT = group 00, sub 0x0, all operand bits reserved; set imm bit 0.
        decode (busWord 0b00 0x0 + 1) @?= Left ReservedFieldNonZero
    ]
  where
    -- helper: build a word from (group, sub) with all-zero operands
    busWord :: BitVector 2 -> BitVector 4 -> BitVector 32
    busWord g s = bitCoerce (g, s, 0 :: BitVector 5, 0 :: BitVector 5, 0 :: BitVector 5, 0 :: BitVector 11)
