# Single-shot non-project Vivado flow: synth -> opt -> place -> route -> bitstream.
#
# Invoked by the Makefile from _build/Tamal.Board.ArtyA7/02-vivado/ as:
#   vivado -mode batch -source <abs>/vivado/build.tcl \
#          -tclargs <part> <top> <hdldir> <xdc> <proj>
#
# Running the whole flow in one Vivado invocation amortises the (dominant)
# tool-startup cost instead of paying it once per stage. The intermediate
# checkpoints and reports are still written into the cwd so each stage remains
# inspectable (open post_route.dcp in the GUI, read the timing summaries, etc.).

lassign $argv part top hdldir xdc proj

# --- Read sources ------------------------------------------------------------
# Every Clash-emitted Verilog source (top entity + any submodules) staged in
# <hdldir> by the Makefile.
set sources [glob -nocomplain [file join $hdldir *.v]]
if {[llength $sources] == 0} {
    error "build.tcl: no Verilog sources found in $hdldir"
}
foreach v $sources {
    read_verilog $v
}

# Hand-written constraints: the 100 MHz clock plus pin LOC/IOSTANDARD.
read_xdc $xdc

# --- Synthesis ---------------------------------------------------------------
synth_design -top $top -part $part
write_checkpoint -force post_synth.dcp
report_utilization    -file post_synth_util.rpt
report_timing_summary -file post_synth_timing.rpt

# --- Implementation (optimise, place, route) ---------------------------------
opt_design
place_design
route_design
write_checkpoint -force post_route.dcp
report_timing_summary -file post_route_timing.rpt
report_drc            -file post_route_drc.rpt

# --- Bitstream ---------------------------------------------------------------
write_bitstream -force $proj.bit
