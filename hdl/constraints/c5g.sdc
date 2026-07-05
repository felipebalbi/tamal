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
