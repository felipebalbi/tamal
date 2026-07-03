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
-}
espiPads ::
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
  -- | the four IO pads (read side)
  Vec 4 (BiSignalIn 'PullUp dom 1) ->
  ( Vec 4 (BiSignalOut 'PullUp dom 1) -- the four IO pads (drive side)
  , Signal dom Bit -- @CS#@   pin out
  , Signal dom Bit -- @SCK@   pin out
  , Signal dom Bit -- @RESET#@ pin out
  , Signal dom (Vec 4 Bit) -- @ioIn@    -> @BusIn.ioIn@
  , Signal dom Bit -- @alertIn@ -> @BusIn.alertIn@
  )
espiPads = undefined
