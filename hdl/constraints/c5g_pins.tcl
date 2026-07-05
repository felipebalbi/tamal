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
