-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-W-2.0

module Test.Trace (tests) where

import Clash.Prelude
import qualified Data.List as L
import Test.Tasty
import Test.Tasty.HUnit

import Tamal.Trace

tests :: TestTree
tests =
  testGroup
    "Trace"
    [ testCase "CAPTURE encodes tag 00, nbits, byte"
        $ encodeRecord (Capture 8 0xA5)
        @?= [0b00 `shiftL` 30 .|. (8 `shiftL` 8) .|. 0xA5]
    , testCase "MARK encodes 2 words: tag 10 + label, then payload"
        $ encodeRecord (Mark 0x1234 0xDEADBEEF)
        @?= [(0b10 `shiftL` 30) .|. 0x1234, 0xDEADBEEF]
    , testCase "HALT (no trap) encodes tag 11, reason 0, trap 0, overflow, status"
        $ encodeRecord (Halt False 0 True 0x11)
        @?= [(0b11 `shiftL` 30) .|. (1 `shiftL` 8) .|. 0x11]
    , testCase "HALT (trap) encodes reason in [12:10] and trap bit [9]"
        $ encodeRecord (Halt True 3 False 0x00)
        @?= [(0b11 `shiftL` 30) .|. (3 `shiftL` 10) .|. (1 `shiftL` 9)]
    , testCase "ringPush past limit sets sticky overflow and drops" $ do
        -- limit 3: slots 0..3 usable, slot 4 = reserved HALT terminator.
        let step (ptr, ovf, acc) ws =
              let (ptr', ovf', wrote) = ringPush ptr 3 ovf ws
               in (ptr', ovf', acc <> wrote)
            recs = L.replicate 10 [0xC0DE :: BitVector 32] -- ten 1-word records
            (finalPtr, finalOvf, written) = L.foldl' step (0, False, []) recs
        assertBool "ptr never past limit+1" (finalPtr <= 4)
        finalOvf @?= True
        assertBool "at most 4 words written" (L.length written <= 4)
    ]
