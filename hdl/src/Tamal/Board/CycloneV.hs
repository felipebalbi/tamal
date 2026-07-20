-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-W-2.0

{- |
Top entity for the tamal gateware on the Terasic Cyclone V GX Starter Kit (C5G):
the thin pin-binding shell. Mirrors 'Tamal.Board.ArtyA7', but the C5G oscillator
is 50 MHz, so an Altera PLL ('alteraPllSync') multiplies it to the design's
100 MHz 'Dom100'. The PLL @areset@ is tied off (no reset port, per AGENTS.md); the
PLL-lock-derived reset holds the design in reset until the 100 MHz clock is stable,
then it runs — behaviourally identical to the Arty's power-up @init@, and strictly
safer (it waits for a stable clock).

The eSPI bus is on the 2x20 GPIO header, the host UART on the board UART pins, and
the status LED on an on-board green LED (see @constraints/c5g_pins.tcl@). As on the
Arty, the four @IO@ lanes are four scalar @inout@ ports (@io0@..@io3@) — a @Vec 4@ of
BiSignals does not fuse to @inout@ in Clash.
-}
module Tamal.Board.CycloneV where

import Clash.Annotations.TH
import Clash.Intel.ClockGen (alteraPllSync)
import Clash.Prelude

import Tamal.Domain (Dom100, DomInput50)
import Tamal.Io (espiPads)
import Tamal.Top (system)

topEntity ::
  "clk" ::: Clock DomInput50 ->
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
topEntity clk50 uartRx io0 io1 io2 io3 alertN =
  withClockResetEnable clk100 rst100 enableGen
    $ let (txLine, lanesO, csO, sckO, rstO, ledOut) = system uartRx ioIn alertIn
          (d0', d1', d2', d3', csPin, sckPin, rstPin, ioIn, alertIn) =
            espiPads lanesO csO sckO rstO alertN io0 io1 io2 io3
       in (d0', d1', d2', d3', txLine, csPin, sckPin, rstPin, ledOut)
 where
  -- PLL areset tied off; (clk100, rst100) come from the Altera PLL. rst100 stays
  -- asserted until the PLL locks, then the design runs on the stable 100 MHz clock.
  (clk100 :: Clock Dom100, rst100 :: Reset Dom100) =
    alteraPllSync clk50 (unsafeFromActiveHigh (pure False))

makeTopEntity 'topEntity
