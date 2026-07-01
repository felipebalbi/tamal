-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

{- |
Top entity for the tamal gateware.

This is a **placeholder heartbeat**: a free-running counter whose top bit blinks
the board LED, so the Clash → Vivado pipeline has a real synthesizable entity to
build until the eSPI cycle engine lands. There is intentionally no reset port
(the registers rely on power-up @init@, like the sibling Clash examples).
-}
module Tamal where

import Clash.Annotations.TH
import Clash.Prelude

import Tamal.Domain (Dom100)

{- | A free-running counter whose most-significant bit toggles the LED at roughly
@100e6 / 2^27 ≈ 0.75 Hz@ — a visible blink. Lives in a 'HiddenClockResetEnable'
helper so its @where@-bound register can be clocked; 'topEntity' discharges the
constraint via 'withClockResetEnable'.
-}
heartbeat ::
        forall dom.
        (HiddenClockResetEnable dom) =>
        Signal dom Bit
heartbeat = msb <$> counter
    where
        -- The explicit @forall dom@ above brings @dom@ into scope here (via
        -- ScopedTypeVariables) so this inner signature refers to the *same*
        -- domain as 'heartbeat'. Without it, @dom@ would be a fresh variable
        -- and the hidden-clock functional dependency wouldn't resolve.
        counter :: Signal dom (Unsigned 27)
        counter = register 0 (counter + 1)

{- | Synthesis entry point. The @"clk"@/@"led"@ named-port annotations (plus
'makeTopEntity') fix the Verilog port names that @constraints/arty_a7.xdc@ binds
to pins.

Like the sibling examples there is no reset port: 'topEntity' hands Clash a
permanently de-asserted reset so the registers start from their power-up @init@
and Clash emits no @reset@ port.
-}
topEntity ::
        -- | 100 MHz board clock (Arty A7 CLK100MHZ, pin E3)
        "clk" ::: Clock Dom100 ->
        -- | Heartbeat LED (Arty A7 LD4)
        "led" ::: Signal Dom100 Bit
topEntity clk = withClockResetEnable clk noReset enableGen heartbeat
    where
        -- No user-reset pin: tie reset permanently de-asserted so the register
        -- starts from its power-up @init@ and Clash emits no @reset@ port.
        noReset = unsafeFromActiveHigh (pure False)

makeTopEntity 'topEntity
