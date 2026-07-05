-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
Top entity for the tamal gateware on the Digilent Arty A7-100T: the thin
pin-binding shell. It ties the 100 MHz clock, binds the tri-state @IO[3:0]@ pads
via 'Tamal.Io.espiPads', and wires the UART / sideband / LED pins around
'Tamal.Top.system'. No reset port (power-up @init@, per AGENTS.md).

The four @IO@ lanes are exposed as four scalar @inout@ ports (@io0@..@io3@): Clash
fuses a 'BiSignalIn' argument with the matching 'BiSignalOut' result into one
@inout@ port per lane. A @Vec 4@ of BiSignals does /not/ fuse (it lowers to a plain
input), so the per-lane scalar form is required.
-}
module Tamal.Board.ArtyA7 where

import Clash.Annotations.TH
import Clash.Prelude

import Tamal.Domain (Dom100)
import Tamal.Io (espiPads)
import Tamal.Top (system)

topEntity ::
  "clk" ::: Clock Dom100 ->
  "uart_rx" ::: Signal Dom100 Bit ->
  "io0" ::: BiSignalIn 'PullUp Dom100 1 ->
  "io1" ::: BiSignalIn 'PullUp Dom100 1 ->
  "io2" ::: BiSignalIn 'PullUp Dom100 1 ->
  "io3" ::: BiSignalIn 'PullUp Dom100 1 ->
  "alert_n" ::: Signal Dom100 Bit ->
  ( "io0" ::: BiSignalOut 'PullUp Dom100 1
  , "io1" ::: BiSignalOut 'PullUp Dom100 1
  , "io2" ::: BiSignalOut 'PullUp Dom100 1
  , "io3" ::: BiSignalOut 'PullUp Dom100 1
  , "uart_tx" ::: Signal Dom100 Bit
  , "cs_n" ::: Signal Dom100 Bit
  , "sck" ::: Signal Dom100 Bit
  , "reset_n" ::: Signal Dom100 Bit
  , "led" ::: Signal Dom100 Bit
  )
topEntity clk uartRx io0 io1 io2 io3 alertN =
  withClockResetEnable clk noReset enableGen
    $ let (txLine, lanesO, csO, sckO, rstO, ledOut) = system uartRx ioIn alertIn
          (ioDrive, csPin, sckPin, rstPin, ioIn, alertIn) =
            espiPads lanesO csO sckO rstO alertN (io0 :> io1 :> io2 :> io3 :> Nil)
          (d0 :> d1 :> d2 :> d3 :> Nil) = ioDrive
       in (d0, d1, d2, d3, txLine, csPin, sckPin, rstPin, ledOut)
 where
  noReset = unsafeFromActiveHigh (pure False)

makeTopEntity 'topEntity
