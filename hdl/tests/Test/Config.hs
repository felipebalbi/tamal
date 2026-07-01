module Test.Config (tests) where

import Clash.Prelude
import Test.Tasty
import Test.Tasty.HUnit

import Tamal.Config

tests :: TestTree
tests =
  testGroup "Config"
    [ testCase "v1 default payload decodes" $
        -- role=0, io=00, sck=00, alert=0  ->  all-zero payload
        decodeConfig 0b000000 @?= Right (Config Controller X1 Sck20 AlertPin)
    , testCase "alert_source=io1 accepted" $
        decodeConfig 0b000001 @?= Right (Config Controller X1 Sck20 AlertIo1)
    , testCase "target role rejected in v1" $
        decodeConfig 0b100000 @?= Left UnsupportedRole
    , testCase "dual I/O rejected in v1" $
        decodeConfig 0b001000 @?= Left UnsupportedIoMode
    , testCase "33 MHz rejected in v1" $
        decodeConfig 0b000010 @?= Left UnsupportedSck
    ]
