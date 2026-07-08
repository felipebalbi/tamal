##----------------------------------------------------------------------------##
#   Project Settings                                                           #
##----------------------------------------------------------------------------##
#
# Two-board build. BOARD selects the target and the toolchain fragment:
#
#   make [BOARD=arty-a7]   Digilent Arty A7-100T   -> Vivado  -> tamal.bit
#   make BOARD=cyclonev    Terasic C5G (Cyclone V) -> Quartus -> tamal.sof
#
# Common to both: Stage 1 Clash+GHC compile src/ -> Verilog (cabal). The
# per-toolchain flow (synth/place/route/bitstream + JTAG load / flash program) lives in
# Makefile.vivado / Makefile.quartus, included below by TOOLCHAIN.
#
# NAME is the Haskell module holding the top entity; TOP is the entity itself
# (`topEntity`). Clash writes HDL to verilog/$(NAME).$(TOP)/. PROJ names the artefact.
#
# Filesystem / cd steps are wrapped in `$(NU) -c "..."` (nushell) for Windows
# portability (see build.cfg.local to override NU). Note: the nushell recipes in
# Makefile.vivado / Makefile.quartus sequence commands with `;` (not `&&`) — leave
# them as-is.

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

# Per-toolchain flow: defines `bitstream`, `load`, `program`, and the artefact
# rules, plus board-specific vars (PART/DEVICE, constraints).
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

.PHONY: default bitstream load program test clean format format-check
.SECONDARY:
