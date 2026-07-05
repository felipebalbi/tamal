-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
The eSPI pad boundary (design doc 2026-07-02-tamal-io-design.md): four per-lane
tri-state @IO[3:0]@ buffers, three sideband output buffers (@CS#@/@SCK@/@RESET#@),
and the @ALERT#@ synchronizer. Realized with Clash 'BiSignal' ('PullUp',
per-lane width-1). A dependency-free leaf: imports only 'Clash.Prelude' and
'Tamal.Bus.Serdes' (for 'Lanes'); the topEntity projects @BusOut@/@BusIn@.
-}
module Tamal.Io
  ( espiPads
  , alertSync
  ) where

import Clash.Prelude
import Tamal.Bus.Serdes (Lanes)

{- | 2-flop synchronizer for the asynchronous, active-low @ALERT#@ sideband.
Both flops init high (deasserted); output lags the raw input by two cycles.
-}
alertSync ::
  (HiddenClockResetEnable dom) =>
  Signal dom Bit ->
  Signal dom Bit
alertSync alert = alert''
 where
  alert' = register high alert
  alert'' = register high alert'

{- | The eSPI pad boundary. Per lane: drive @Just o@ when @oe == 1@ else @Nothing@
(hi-Z); read the pad combinationally into @ioIn@. Sidebands pass through
(already registered upstream). @ALERT#@ is synchronized via 'alertSync'.

__Per-lane scalar 'BiSignal's, deliberately not a @Vec@.__ The four @IO@ lanes are
four separate 'BiSignalIn' arguments and four separate 'BiSignalOut' results, so
each @writeToBiSignal padK ...@ is a direct scalar result. Clash fuses a scalar
'BiSignalIn' argument with the scalar 'BiSignalOut' result derived from it into one
@inout@ port per lane — but if the lanes are routed through a @Vec@ of 'BiSignal's
(@zipWith@/@map@/@:>@ over 'BiSignalIn'/'BiSignalOut'), Clash treats the vector as an
opaque bundle and __drops the drive__: the @inout@ ports get a read path but no
tri-state driver (the write collapses to a dead net). So the lanes must stay scalar
end to end — here and in the topEntity shells. (Verified from the emitted Verilog:
each @io0@..@io3@ must carry its own tri-state driver.)
-}
espiPads ::
  forall dom.
  (HiddenClockResetEnable dom) =>
  -- | engine @lanesOut@ (per-lane (o, oe))
  Signal dom Lanes ->
  -- | @csOut@
  Signal dom Bit ->
  -- | @sckOut@
  Signal dom Bit ->
  -- | @rstOut@
  Signal dom Bit ->
  -- | @ALERT#@ (raw, async, active-low)
  Signal dom Bit ->
  -- | @IO0@ pad (read side)
  BiSignalIn 'PullUp dom 1 ->
  -- | @IO1@ pad (read side)
  BiSignalIn 'PullUp dom 1 ->
  -- | @IO2@ pad (read side)
  BiSignalIn 'PullUp dom 1 ->
  -- | @IO3@ pad (read side)
  BiSignalIn 'PullUp dom 1 ->
  ( BiSignalOut 'PullUp dom 1 -- @IO0@ pad (drive side)
  , BiSignalOut 'PullUp dom 1 -- @IO1@ pad (drive side)
  , BiSignalOut 'PullUp dom 1 -- @IO2@ pad (drive side)
  , BiSignalOut 'PullUp dom 1 -- @IO3@ pad (drive side)
  , Signal dom Bit -- @CS#@   pin out
  , Signal dom Bit -- @SCK@   pin out
  , Signal dom Bit -- @RESET#@ pin out
  , Signal dom (Vec 4 Bit) -- @ioIn@    -> @BusIn.ioIn@
  , Signal dom Bit -- @alertIn@ -> @BusIn.alertIn@
  )
espiPads lanesOut csOut sckOut rstOut alert pad0 pad1 pad2 pad3 =
  ( drive 0 pad0
  , drive 1 pad1
  , drive 2 pad2
  , drive 3 pad3
  , csOut
  , sckOut
  , rstOut
  , ioIn
  , alert'
  )
 where
  alert' = alertSync alert
  laneSigs = unbundle lanesOut
  drive :: Index 4 -> BiSignalIn 'PullUp dom 1 -> BiSignalOut 'PullUp dom 1
  drive i padIn = writeToBiSignal padIn (toDrive <$> (laneSigs !! i))
  ioIn =
    bundle
      ( readFromBiSignal pad0
          :> readFromBiSignal pad1
          :> readFromBiSignal pad2
          :> readFromBiSignal pad3
          :> Nil
      )
  toDrive (o, oe) =
    if oe == 1
      then Just o
      else Nothing
