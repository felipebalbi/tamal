# Cyclone V GX board target + multi-board build Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the Terasic Cyclone V GX Starter Kit (C5G) as a second synthesis target beside the Arty A7-100T, selectable with `make BOARD=cyclonev` (default `arty-a7`), reusing the board-agnostic core unchanged.

**Architecture:** Keep `Tamal.Top.system` / `Tamal.Io` / `Tamal.Domain.Dom100` untouched. Relocate the Arty pin-shell to `Tamal.Board.ArtyA7`; add `Tamal.Board.CycloneV` that multiplies the C5G's 50 MHz oscillator to `Dom100` with `Clash.Intel.ClockGen.alteraPllSync` (areset tied off → no reset port). Split the Makefile by toolchain (`Makefile.vivado` / `Makefile.quartus`, included via `BOARD`); the Quartus flow mirrors `~/workspace/hdl/cyclonev-clash-examples`.

**Tech Stack:** Clash (clash-prelude 1.10.0, `Clash.Intel.ClockGen`), Cabal, GNU Make, Quartus Prime (Cyclone V, `5CGXFC5C6F27C7`), Vivado (Artix-7).

**Spec:** `docs/superpowers/specs/2026-07-05-tamal-cyclonev-board-design.md`

**Working directory:** all `cabal` / `make` / `git` commands run from `hdl/` unless noted. Branch: `feat/cyclonev-board`.

**Convention:** run `make format` before committing any `.hs` change (fourmolu; CI runs `make format-check`). Keep the SPDX header (`CERN-OHL-P-2.0`) on every new `hdl/**/*.hs` file.

---

### Task 1: Add the 50 MHz PLL-input domain

**Files:**
- Modify: `hdl/src/Tamal/Domain.hs`

- [ ] **Step 1: Add `DomInput50` after `Dom100`**

Append to `hdl/src/Tamal/Domain.hs` (after the existing `createDomain vSystem{...Dom100...}` splice):

```haskell
-- | 50 MHz reference domain for the C5G oscillator (CLOCK_50_B5B, pin R20). It
-- is the input to the Cyclone V PLL ('Tamal.Board.CycloneV'); the PLL multiplies
-- it to the design's 100 MHz 'Dom100'. Asynchronous + ActiveHigh (the 'vSystem'
-- default), which satisfies 'alteraPllSync''s @HasAsynchronousReset@ requirement.
createDomain
  vSystem
    { vName = "DomInput50"
    , vPeriod = hzToPeriod 50_000_000
    }
```

- [ ] **Step 2: Build the library**

Run: `cabal build tamal`
Expected: compiles with no errors (an unused type synonym does not warn).

- [ ] **Step 3: Commit**

```bash
git add hdl/src/Tamal/Domain.hs
git commit -m "feat(hdl): add DomInput50 (50 MHz PLL-input domain) for the C5G"
```

---

### Task 2: Relocate the Arty pin-shell to `Tamal.Board.ArtyA7`

Pure relocation — the topEntity logic is unchanged, only the module name. Keeps the
existing (monolithic) Makefile working via a one-line `NAME` bump; the Makefile is
fully replaced in Task 6.

**Files:**
- Create: `hdl/src/Tamal/Board/ArtyA7.hs`
- Delete: `hdl/src/Tamal.hs`
- Modify: `hdl/tamal.cabal` (exposed-modules), `hdl/Makefile` (NAME + source wildcard)

- [ ] **Step 1: Create `hdl/src/Tamal/Board/ArtyA7.hs`**

Content (identical to the current `src/Tamal.hs` except the module name):

```haskell
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
```

- [ ] **Step 2: Delete the old shell**

```bash
git rm hdl/src/Tamal.hs
```

- [ ] **Step 3: Swap the exposed module in `hdl/tamal.cabal`**

In the `library` stanza's `exposed-modules`, replace the line `    Tamal` with
`    Tamal.Board.ArtyA7`. (Leave every other module line unchanged.)

- [ ] **Step 4: Bump the Makefile so `make` still targets the relocated module**

In `hdl/Makefile`, change `NAME = Tamal` to:

```makefile
NAME = Tamal.Board.ArtyA7
```

and widen the Clash-codegen prerequisite wildcard (the shell now lives three
levels deep, `src/Tamal/Board/`), i.e. change the recipe line
`$(CLASHOUT)/$(TOP).v: $(wildcard src/*.hs src/*/*.hs)` to:

```makefile
$(CLASHOUT)/$(TOP).v: $(wildcard src/*.hs src/*/*.hs src/*/*/*.hs)
```

- [ ] **Step 5: Format**

Run: `make format`
Expected: no diff on `ArtyA7.hs` (byte-copy of an already-formatted file).

- [ ] **Step 6: Verify the library + tests still build**

Run: `cabal test`
Expected: all tests pass (exit 0) — `Test.Top` imports `Tamal.Top`, not the shell, so it is unaffected.

- [ ] **Step 7: Verify Arty codegen under the new name**

Run: `cabal run clash -- Tamal.Board.ArtyA7 --verilog`
Expected: writes `verilog/Tamal.Board.ArtyA7.topEntity/topEntity.v` (exit 0).

- [ ] **Step 8: Commit**

```bash
git add hdl/src/Tamal/Board/ArtyA7.hs hdl/tamal.cabal hdl/Makefile
git commit -m "refactor(hdl): relocate Arty topEntity to Tamal.Board.ArtyA7"
```

---

### Task 3: Add the `Tamal.Board.CycloneV` shell (50→100 MHz PLL)

**Files:**
- Create: `hdl/src/Tamal/Board/CycloneV.hs`
- Modify: `hdl/tamal.cabal` (exposed-modules)

- [ ] **Step 1: Create `hdl/src/Tamal/Board/CycloneV.hs`**

```haskell
-- SPDX-FileCopyrightText: 2026 Felipe Balbi
-- SPDX-License-Identifier: CERN-OHL-P-2.0

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
          (ioDrive, csPin, sckPin, rstPin, ioIn, alertIn) =
            espiPads lanesO csO sckO rstO alertN (io0 :> io1 :> io2 :> io3 :> Nil)
          (d0 :> d1 :> d2 :> d3 :> Nil) = ioDrive
       in (d0, d1, d2, d3, txLine, csPin, sckPin, rstPin, ledOut)
 where
  -- PLL areset tied off; (clk100, rst100) come from the Altera PLL. rst100 stays
  -- asserted until the PLL locks, then the design runs on the stable 100 MHz clock.
  (clk100 :: Clock Dom100, rst100 :: Reset Dom100) =
    alteraPllSync clk50 (unsafeFromActiveHigh (pure False))

makeTopEntity 'topEntity
```

- [ ] **Step 2: Add the module to `hdl/tamal.cabal`**

In the `library` `exposed-modules`, add a line beside `Tamal.Board.ArtyA7`:

```
    Tamal.Board.CycloneV
```

- [ ] **Step 3: Format**

Run: `make format`
Expected: `CycloneV.hs` conforms (small or no diff).

- [ ] **Step 4: Verify the library builds**

Run: `cabal build tamal`
Expected: compiles (exit 0). If GHC cannot infer the PLL tuple, the explicit
`(clk100 :: Clock Dom100, rst100 :: Reset Dom100)` pattern signature (already in the
code above) resolves it.

- [ ] **Step 5: Verify C5G codegen emits the PLL + a `.qsys`**

Run: `cabal run clash -- Tamal.Board.CycloneV --verilog`
Expected: writes `verilog/Tamal.Board.CycloneV.topEntity/` (exit 0).

Then confirm the PLL IP was emitted:

```bash
ls verilog/Tamal.Board.CycloneV.topEntity/*.qsys
grep -l altera_pll verilog/Tamal.Board.CycloneV.topEntity/*.v
```

Expected: at least one `.qsys` file exists, and an `altera_pll` component is
instantiated in the generated Verilog. Also eyeball the top port list for a `clk`
input plus `io0..io3` `inout` and the named UART/eSPI/LED ports.

- [ ] **Step 6: Commit**

```bash
git add hdl/src/Tamal/Board/CycloneV.hs hdl/tamal.cabal
git commit -m "feat(hdl): add Tamal.Board.CycloneV shell (alteraPllSync 50->100 MHz)"
```

---

### Task 4: Cyclone V constraints (timing + pins)

**Files:**
- Create: `hdl/constraints/c5g.sdc`
- Create: `hdl/constraints/c5g_pins.tcl`

- [ ] **Step 1: Create `hdl/constraints/c5g.sdc`**

```tcl
# Tamal — Quartus timing constraints for the Terasic Cyclone V GX Starter Kit (C5G).
#
# The board oscillator is 50 MHz (CLOCK_50_B5B, pin R20); the design runs at
# 100 MHz via an Altera PLL (alteraPllSync, Tamal.Board.CycloneV). Constrain the
# 50 MHz input on the `clk` port and let derive_pll_clocks constrain the
# PLL-generated 100 MHz output.

create_clock -name clk50 -period 20.000 [get_ports {clk}]

# Constrain the PLL output clock(s) from the altera_pll IP, plus jitter/uncertainty.
derive_pll_clocks
derive_clock_uncertainty
```

- [ ] **Step 2: Create `hdl/constraints/c5g_pins.tcl`**

```tcl
# Tamal — Quartus pin assignments for the Terasic Cyclone V GX Starter Kit (C5G).
# Sourced by quartus/build.tcl during project generation. Pin locations and I/O
# standards are from Terasic's C5G_Default.qsf, bound to the Clash topEntity port
# names. Host UART on the board UART pins; eSPI bus on the 2x20 GPIO header; status
# LED on an on-board green LED. Weak pull-ups on the eSPI IO lanes + ALERT# realise
# the espiPads 'PullUp default (eSPI idle-high) — the C5G analogue of the Arty XDC's
# PULLUP TRUE. All other C5G pins keep Quartus's safe default (input, weak pull-up).

# --- 50 MHz reference clock (CLOCK_50_B5B) -----------------------------------
set_location_assignment PIN_R20 -to clk
set_instance_assignment -name IO_STANDARD "3.3-V LVTTL" -to clk

# --- Host UART (board UART pins) ---------------------------------------------
set_location_assignment PIN_M9 -to uart_rx
set_instance_assignment -name IO_STANDARD "2.5 V" -to uart_rx
set_location_assignment PIN_L9 -to uart_tx
set_instance_assignment -name IO_STANDARD "2.5 V" -to uart_tx

# --- eSPI data lanes IO[3:0] — GPIO[0..3], weak pull-up (eSPI idle-high) ------
set_location_assignment PIN_T21 -to io0
set_instance_assignment -name IO_STANDARD "3.3-V LVTTL" -to io0
set_instance_assignment -name WEAK_PULL_UP_RESISTOR ON -to io0
set_location_assignment PIN_D26 -to io1
set_instance_assignment -name IO_STANDARD "3.3-V LVTTL" -to io1
set_instance_assignment -name WEAK_PULL_UP_RESISTOR ON -to io1
set_location_assignment PIN_K25 -to io2
set_instance_assignment -name IO_STANDARD "3.3-V LVTTL" -to io2
set_instance_assignment -name WEAK_PULL_UP_RESISTOR ON -to io2
set_location_assignment PIN_E26 -to io3
set_instance_assignment -name IO_STANDARD "3.3-V LVTTL" -to io3
set_instance_assignment -name WEAK_PULL_UP_RESISTOR ON -to io3

# --- eSPI sideband — GPIO[4..7] ----------------------------------------------
set_location_assignment PIN_K26 -to sck
set_instance_assignment -name IO_STANDARD "3.3-V LVTTL" -to sck
set_location_assignment PIN_M26 -to cs_n
set_instance_assignment -name IO_STANDARD "3.3-V LVTTL" -to cs_n
set_location_assignment PIN_M21 -to reset_n
set_instance_assignment -name IO_STANDARD "3.3-V LVTTL" -to reset_n
set_location_assignment PIN_P20 -to alert_n
set_instance_assignment -name IO_STANDARD "3.3-V LVTTL" -to alert_n
set_instance_assignment -name WEAK_PULL_UP_RESISTOR ON -to alert_n

# --- Status LED — on-board green LEDG[0] -------------------------------------
set_location_assignment PIN_L7 -to led
set_instance_assignment -name IO_STANDARD "2.5 V" -to led
```

- [ ] **Step 3: Commit** (no build yet — these are consumed in Task 6)

```bash
git add hdl/constraints/c5g.sdc hdl/constraints/c5g_pins.tcl
git commit -m "feat(hdl): C5G Quartus constraints (50 MHz clock + derive_pll_clocks, GPIO/UART/LED pins)"
```

---

### Task 5: Quartus project-generation Tcl

**Files:**
- Create: `hdl/quartus/build.tcl`

- [ ] **Step 1: Create `hdl/quartus/build.tcl`**

```tcl
# Tamal — Quartus project generation for the Terasic Cyclone V GX Starter Kit (C5G).
#
# Run by Makefile.quartus from _build/Tamal.Board.CycloneV/02-quartus/ as:
#   quartus_sh -t <abs>/quartus/build.tcl <device> <top> <proj> <hdldir> <sdc> <pins>
#
# It (re)creates the Quartus project there: device, the Clash-generated Verilog,
# the altera_pll QSYS_FILE emitted by Clash's alteraPllSync blackbox, the timing
# SDC, and the pin assignments. The discrete compile stages (quartus_map/fit/asm/
# sta) then run from the Makefile, mirroring the sibling cyclonev-clash-examples.

lassign $argv device top proj hdldir sdc pins

package require ::quartus::project

# -overwrite: clean slate each build; the Makefile owns staleness.
project_new $proj -overwrite

set_global_assignment -name NUM_PARALLEL_PROCESSORS ALL
set_global_assignment -name FAMILY "Cyclone V"
set_global_assignment -name DEVICE $device
set_global_assignment -name TOP_LEVEL_ENTITY $top

# Clash-generated Verilog (top + any submodules), staged into $hdldir by the Makefile.
set verilogs [glob -nocomplain [file join $hdldir *.v]]
if {[llength $verilogs] == 0} {
    project_close
    error "build.tcl: no Verilog sources found in $hdldir"
}
foreach v $verilogs {
    set_global_assignment -name VERILOG_FILE $v
}

# The Altera PLL IP emitted alongside the Verilog by alteraPllSync. Adding it as a
# QSYS_FILE lets quartus_map (Analysis & Synthesis) generate the IP automatically.
foreach q [glob -nocomplain [file join $hdldir *.qsys]] {
    set_global_assignment -name QSYS_FILE $q
}

# Hand-written timing (50 MHz create_clock + derive_pll_clocks). Only this SDC is
# added, so there is no duplicate-create_clock conflict with Clash's staged .sdc.
set_global_assignment -name SDC_FILE $sdc

# Pin locations + I/O standards.
source $pins

project_close
```

- [ ] **Step 2: Commit**

```bash
git add hdl/quartus/build.tcl
git commit -m "feat(hdl): Quartus project-generation tcl (sources + QSYS PLL + SDC + pins)"
```

---

### Task 6: Split the Makefile by toolchain + wire the Quartus flow

Replace the monolithic `Makefile` with a board-dispatch core plus two toolchain
fragments, and add the Quartus tool variables. After this task, `make` builds the
Arty (Vivado) and `make BOARD=cyclonev` builds the C5G (Quartus).

**Files:**
- Modify: `hdl/Makefile` (replace with the dispatch core below)
- Create: `hdl/Makefile.vivado`, `hdl/Makefile.quartus`
- Modify: `hdl/build.cfg` (add `QUARTUS_*`)

- [ ] **Step 1: Replace `hdl/Makefile` with the board-dispatch core**

```makefile
##----------------------------------------------------------------------------##
#   Project Settings                                                           #
##----------------------------------------------------------------------------##
#
# Two-board build. BOARD selects the target and the toolchain fragment:
#
#   make [BOARD=arty-a7]   Digilent Arty A7-100T  -> Vivado  -> tamal.bit
#   make BOARD=cyclonev    Terasic C5G (Cyclone V) -> Quartus -> tamal.sof
#
# Common to both: Stage 1 Clash+GHC compile src/ -> Verilog (cabal). The
# per-toolchain flow (synth/place/route/bitstream + JTAG program) lives in
# Makefile.vivado / Makefile.quartus, included below by TOOLCHAIN.
#
# NAME is the Haskell module holding the top entity; TOP is the entity itself
# (`topEntity`). Clash writes HDL to verilog/$(NAME).$(TOP)/. PROJ names the artefact.
#
# Filesystem / cd steps are wrapped in `$(NU) -c "..."` (nushell) for Windows
# portability; see the original single-board history for the full rationale.

BOARD ?= arty-a7

TOP  = topEntity
PROJ = tamal

ifeq ($(BOARD),arty-a7)
  NAME      = Tamal.Board.ArtyA7
  TOOLCHAIN = vivado
else ifeq ($(BOARD),cyclonev)
  NAME      = Tamal.Board.CycloneV
  TOOLCHAIN = quartus
else
  $(error Unknown BOARD '$(BOARD)' -- use BOARD=arty-a7 or BOARD=cyclonev)
endif

# Build tree, keyed on the design (NAME) so both boards' artefacts coexist.
BUILDDIR = _build/$(NAME)
HDLDIR   = $(BUILDDIR)/01-hdl

# Where Clash drops its output.
CLASHOUT = verilog/$(NAME).$(TOP)

# nushell, used for filesystem / cd steps. Override in build.cfg.local if `nu`
# isn't on PATH.
NU = nu

# Tool paths live in build.cfg; copy it to build.cfg.local to override locally.
-include build.cfg
-include build.cfg.local

##----------------------------------------------------------------------------##
#   Build Rules                                                                #
##----------------------------------------------------------------------------##

default: bitstream

# Per-toolchain flow: defines `bitstream`, `program`, and the artefact rules,
# plus board-specific vars (PART/DEVICE, constraints).
include Makefile.$(TOOLCHAIN)

# Stage the Clash-generated HDL (*.v [+ *.qsys for cyclonev]) into the build tree.
$(HDLDIR)/$(TOP).v: $(CLASHOUT)/$(TOP).v
	$(NU) -c "rm -rf $(HDLDIR)"
	$(NU) -c "mkdir $(BUILDDIR)"
	$(NU) -c "cp -r $(CLASHOUT) $(HDLDIR)"

# Generate Verilog from Clash. Prereqs from $(wildcard) (a make built-in, no shell)
# over all source levels, including the board shells in src/Tamal/Board/.
$(CLASHOUT)/$(TOP).v: $(wildcard src/*.hs src/*/*.hs src/*/*/*.hs)
	$(CLASH) $(NAME) --verilog

##----------------------------------------------------------------------------##
#   Tests / Formatting / Cleanup                                               #
##----------------------------------------------------------------------------##

test:
	$(TEST)

HS = $(wildcard \
  src/*.hs src/*/*.hs src/*/*/*.hs src/*/*/*/*.hs \
  tests/*.hs tests/*/*.hs tests/*/*/*.hs \
  bin/*.hs)

format:
	$(FOURMOLU) --mode inplace $(HS)

format-check:
	$(FOURMOLU) --check-idempotence --mode check $(HS)

clean:
	$(NU) -c "rm -rf _build verilog"

##----------------------------------------------------------------------------##
#   Special Targets                                                            #
##----------------------------------------------------------------------------##

.PHONY: default bitstream program test clean format format-check
.SECONDARY:
```

- [ ] **Step 2: Add the Quartus tool variables to `hdl/build.cfg`**

Append to `hdl/build.cfg` (before the closing `####` banner):

```makefile
# Quartus Prime (Cyclone V / C5G, BOARD=cyclonev). Defaults assume quartus_* are on
# PATH. If your install isn't on PATH, override in build.cfg.local, e.g.
#   QUARTUS_SH = /home/you/altera_lite/25.1std/quartus/bin/quartus_sh
QUARTUS_SH  = quartus_sh
QUARTUS_MAP = quartus_map
QUARTUS_FIT = quartus_fit
QUARTUS_ASM = quartus_asm
QUARTUS_STA = quartus_sta
QUARTUS_PGM = quartus_pgm
```

---

- [ ] **Step 3: Create `hdl/Makefile.vivado`** (Arty flow, lifted verbatim from the old Makefile)

```makefile
##----------------------------------------------------------------------------##
#   Digilent Arty A7-100T -- Vivado flow (BOARD=arty-a7)                        #
##----------------------------------------------------------------------------##
# Included by the main Makefile when TOOLCHAIN=vivado. One non-project Vivado
# launch (synth -> opt -> place -> route -> bitstream), then JTAG program.

# Xilinx part for the Digilent Arty A7-100T.
PART = xc7a100tcsg324-1

VDIR = $(BUILDDIR)/02-vivado

# Hand-written Vivado constraints (absolute; the recipe cd's into $(VDIR)).
XDC = $(abspath constraints/arty_a7.xdc)

BUILD_TCL = $(abspath vivado/build.tcl)
PGM_TCL   = $(abspath vivado/program.tcl)

BIT = $(VDIR)/$(PROJ).bit

bitstream: $(BIT)

# Program the Arty A7 over its built-in JTAG (volatile SRAM config).
program: $(BIT)
	$(NU) -c "cd $(VDIR); $(VIVADO) -mode batch -source $(PGM_TCL) -tclargs $(PROJ).bit"

# The whole Vivado flow in one launch: synth -> opt -> place -> route -> bitstream.
$(BIT): $(HDLDIR)/$(TOP).v $(XDC)
	$(NU) -c "rm -rf $(VDIR)"
	$(NU) -c "mkdir $(VDIR)"
	$(NU) -c "cd $(VDIR); $(VIVADO) -mode batch -source $(BUILD_TCL) -tclargs $(PART) $(TOP) $(abspath $(HDLDIR)) $(XDC) $(PROJ)"

.PHONY: bitstream program
```

- [ ] **Step 4: Create `hdl/Makefile.quartus`** (C5G flow)

```makefile
##----------------------------------------------------------------------------##
#   Terasic Cyclone V GX Starter Kit (C5G) -- Quartus flow (BOARD=cyclonev)     #
##----------------------------------------------------------------------------##
# Included by the main Makefile when TOOLCHAIN=quartus. Mirrors the sibling
# cyclonev-clash-examples: quartus_sh -t build.tcl (project) -> quartus_map ->
# quartus_fit -> quartus_asm -> quartus_sta -> quartus_pgm (embedded USB-Blaster).

# Cyclone V device on the C5G. The chip is marked 5CGXFC5C6F27C7N; Quartus's DEVICE
# drops the trailing "N" (lead-free suffix) or quartus_map fails "Part name illegal".
DEVICE = 5CGXFC5C6F27C7
QPROJ  = $(PROJ)

QDIR = $(BUILDDIR)/02-quartus

# Hand-written constraints + project tcl (absolute; recipes cd into $(QDIR)).
SDC       = $(abspath constraints/c5g.sdc)
PINS      = $(abspath constraints/c5g_pins.tcl)
BUILD_TCL = $(abspath quartus/build.tcl)

# Quartus stage markers (each discrete tool writes one into $(QDIR)).
QSF = $(QDIR)/$(QPROJ).qsf
MAP = $(QDIR)/$(QPROJ).map.rpt
FIT = $(QDIR)/$(QPROJ).fit.rpt
SOF = $(QDIR)/$(QPROJ).sof
STA = $(QDIR)/$(QPROJ).sta.rpt

bitstream: $(SOF)
timing:    $(STA)

# Program the C5G over its built-in USB-Blaster. `.sof` is volatile SRAM config
# (lost on power cycle). `-o "p;<file>"` programs the first device in the JTAG
# chain (the C5G has one); run `jtagconfig` to confirm the cable.
program: $(SOF)
	$(NU) -c "cd $(QDIR); $(QUARTUS_PGM) -m jtag -o \"p;$(QPROJ).sof\""

# 6. Timing analysis (off the default path; reports slack against the SDC).
$(STA): $(SOF)
	$(NU) -c "cd $(QDIR); $(QUARTUS_STA) $(QPROJ)"

# 5. Assembler: emit the bitstream (.sof) from the fitted design.
$(SOF): $(FIT)
	$(NU) -c "cd $(QDIR); $(QUARTUS_ASM) $(QPROJ)"

# 4. Fitter: place & route.
$(FIT): $(MAP)
	$(NU) -c "cd $(QDIR); $(QUARTUS_FIT) $(QPROJ)"

# 3. Analysis & synthesis. quartus_map auto-generates the altera_pll IP from the
#    staged *.qsys (added as QSYS_FILE by build.tcl).
$(MAP): $(QSF)
	$(NU) -c "cd $(QDIR); $(QUARTUS_MAP) $(QPROJ)"

# 2. Generate the Quartus project (device, sources, QSYS PLL, SDC, pins).
$(QSF): $(HDLDIR)/$(TOP).v $(BUILD_TCL) $(SDC) $(PINS)
	$(NU) -c "rm -rf $(QDIR)"
	$(NU) -c "mkdir $(QDIR)"
	$(NU) -c "cd $(QDIR); $(QUARTUS_SH) -t $(BUILD_TCL) $(DEVICE) $(TOP) $(QPROJ) $(abspath $(HDLDIR)) $(SDC) $(PINS)"

.PHONY: bitstream timing program
```

- [ ] **Step 5: Verify the dispatch + bad-board guard**

Run: `make BOARD=bogus`
Expected: fails immediately with `Unknown BOARD 'bogus' -- use BOARD=arty-a7 or BOARD=cyclonev`.

Run: `make test`
Expected: all tests pass (exit 0) — confirms the common rules still parse/run.

- [ ] **Step 6: Verify the Arty path regenerates (codegen + staging)**

Run: `make BOARD=arty-a7 verilog/Tamal.Board.ArtyA7.topEntity/topEntity.v`
Expected: Clash regenerates the Arty top (exit 0). (The full `.bit` needs Vivado;
run `make` if Vivado is installed — otherwise this codegen step is the regression gate.)

- [ ] **Step 7: Build the C5G bitstream end-to-end (Quartus)**

Run: `make BOARD=cyclonev`
Expected: Clash → stage → `quartus_sh` (project) → `quartus_map` (generates the
altera_pll from the `.qsys`) → `quartus_fit` → `quartus_asm`, ending with
`_build/Tamal.Board.CycloneV/02-quartus/tamal.sof` created. This is the primary
end-to-end gate for the whole design.

If `quartus_map` cannot find the generated PLL (the §7 risk in the spec), apply the
fallback: add an explicit `qsys-generate` before `quartus_map` in the `$(QSF)`
recipe, e.g.
`$(NU) -c "cd $(QDIR); qsys-generate --synthesis=VERILOG --family=\"Cyclone V\" --part=$(DEVICE) <name>.qsys"`,
then add the resulting `<name>/synthesis/<name>.qip` as a `QIP_FILE` in `build.tcl`.

- [ ] **Step 8: Commit**

```bash
git add hdl/Makefile hdl/Makefile.vivado hdl/Makefile.quartus hdl/build.cfg
git commit -m "build(hdl): BOARD dispatch + Makefile.vivado/Makefile.quartus split; C5G Quartus flow"
```

---

### Task 7: Documentation

Prose updates so the two-board reality is discoverable. Use the exact text below.

**Files:**
- Modify: `hdl/README.md`, `AGENTS.md` (repo root), `hdl/PLAN.md`

- [ ] **Step 1: `AGENTS.md` — "Target hardware" section**

Replace the current Arty-only bullets under `## Target hardware` with:

```markdown
Two boards, selected with `make BOARD=…` (the gateware core is board-agnostic; only
the pin-shell + toolchain differ):

- **Digilent Arty A7-100T** (default, `BOARD=arty-a7`), part `xc7a100tcsg324-1`,
  100 MHz clock (pin E3). Vivado → `tamal.bit`, JTAG via the Vivado hardware manager.
- **Terasic Cyclone V GX Starter Kit** (`BOARD=cyclonev`), device `5CGXFC5C6F27C7`,
  50 MHz oscillator (CLOCK_50_B5B, pin R20) multiplied to 100 MHz by an Altera PLL
  (`Clash.Intel.ClockGen.alteraPllSync`). Quartus → `tamal.sof`, JTAG via the
  embedded USB-Blaster (`quartus_pgm`). eSPI bus on the 2×20 GPIO header; host UART
  on the board UART pins.
- **Transport:** the Arty exposes USB-UART + JTAG; the C5G host UART uses the board
  UART pins. Keep `tamal-abi` transport-agnostic (FX3 GPIF II shield is a future backend).
```

- [ ] **Step 2: `AGENTS.md` — "HDL build flow" section**

Add this paragraph at the end of the `## HDL build flow (Clash → Vivado)` section:

```markdown
The build is **two-board**: `make BOARD=arty-a7` (default) runs the Vivado flow above;
`make BOARD=cyclonev` runs a Quartus flow instead (`quartus_sh -t quartus/build.tcl`
to create the project, then `quartus_map → quartus_fit → quartus_asm → quartus_sta`,
then `quartus_pgm` over the embedded USB-Blaster). `BOARD` selects the Clash top module
(`Tamal.Board.ArtyA7` / `Tamal.Board.CycloneV`) and includes the matching toolchain
fragment (`Makefile.vivado` / `Makefile.quartus`). The Cyclone V path adds a 50→100 MHz
Altera PLL whose IP Clash emits as a `.qsys` (generated by `quartus_map` via `QSYS_FILE`).
Constraints: `constraints/arty_a7.xdc` (Vivado), `constraints/c5g.sdc` +
`constraints/c5g_pins.tcl` (Quartus).
```

- [ ] **Step 3: `hdl/README.md` — intro**

Replace the opening sentence "The Clash FPGA gateware for the tamal eSPI compliance
rig, targeting the **Digilent Arty A7-100T** (`xc7a100tcsg324-1`, 100 MHz).
Self-contained: Clash → Verilog → Vivado bitstream, driven by `make`." with:

```markdown
The Clash FPGA gateware for the tamal eSPI compliance rig, targeting the **Digilent
Arty A7-100T** (`xc7a100tcsg324-1`, Vivado — default `make`) and the **Terasic
Cyclone V GX Starter Kit** (`5CGXFC5C6F27C7`, Quartus — `make BOARD=cyclonev`). Both
run the design on `Dom100` (100 MHz); the C5G's 50 MHz oscillator is multiplied by an
Altera PLL. Self-contained: Clash → Verilog → vendor bitstream, driven by `make`.
```

Also update the "one clock domain (`Dom100`, 100 MHz) — no PLL, no CDC, no FIFOs"
note: change "no PLL, no CDC" to:

```markdown
no CDC (the C5G multiplies its 50 MHz oscillator to `Dom100` with an Altera PLL, but
it is still a single design domain), no FIFOs
```

- [ ] **Step 4: `hdl/PLAN.md` — note the second target**

Add to the top "Where things stand" prose:

```markdown
tamal now builds to a v1 bitstream on **two boards**: the Arty A7-100T (Vivado,
`make`) and the Cyclone V GX Starter Kit (Quartus, `make BOARD=cyclonev`, 50→100 MHz
Altera PLL). The gateware core is shared; only the `Tamal.Board.*` pin-shell and the
toolchain fragment differ.
```

- [ ] **Step 5: Verify formatting untouched + commit**

Run: `make format-check`
Expected: passes (docs are not Haskell; this confirms Task 6's Makefile still runs fourmolu).

```bash
git add AGENTS.md hdl/README.md hdl/PLAN.md
git commit -m "docs: describe the two-board (Arty/Cyclone V) build"
```

---

### Task 8: Program + smoke-test on the C5G (hardware gate)

Final validation on the physical board. Requires the C5G connected over USB (JTAG)
and Quartus on `PATH`. No code changes; do not commit.

- [ ] **Step 1: Confirm the cable is visible**

Run: `jtagconfig`
Expected: lists a USB-BlasterII with one `5CGXFC5C...` device in the chain. If it is
missing, fix cable/permissions before continuing (`quartus_pgm --list`).

- [ ] **Step 2: Flash the bitstream**

Run: `make BOARD=cyclonev program`
Expected: `quartus_pgm` reports `Configuration succeeded` / `100% complete`.

- [ ] **Step 3: Smoke-check the LED lifecycle**

Observe on-board `LEDG[0]`: after configuration it shows the "Waiting" pattern
(slow blink). Driving a program over the UART (host tooling) advances it
Waiting → Running → Done — the same first sanity check as the Arty. Full eSPI
bring-up on the GPIO header is out of scope for this build change (see spec §2).

---

## Verification summary (run order)

From `hdl/`:

1. `cabal test` — the board-agnostic suite stays green (Tasks 2–3).
2. `cabal run clash -- Tamal.Board.ArtyA7 --verilog` — Arty codegen (Task 2).
3. `cabal run clash -- Tamal.Board.CycloneV --verilog` — C5G codegen; a `.qsys` +
   `altera_pll` appear (Task 3).
4. `make BOARD=bogus` errors cleanly; `make test` / `make format-check` pass (Task 6).
5. `make` (or the Arty codegen step) — Arty path regression (Task 6).
6. `make BOARD=cyclonev` → `tamal.sof` — the primary end-to-end gate (Task 6).
7. `make BOARD=cyclonev program` — on-hardware flash + LED smoke test (Task 8).

