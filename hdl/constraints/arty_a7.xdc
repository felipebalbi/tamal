## Tamal — minimal Vivado constraints for the Digilent Arty A7-100T.
##
## Only the ports of the current placeholder heartbeat top are constrained:
## the 100 MHz system clock and one LED. Extend this as the eSPI cycle engine
## lands (CS#, CLK, IO[3:0], ALERT#, RESET#). The full board master XDC is
## published by Digilent:
##   https://github.com/Digilent/digilent-xdc  (Arty-A7-100-Master.xdc)

## ---- 100 MHz system clock (CLK100MHZ, bank 35, pin E3) ----------------------
set_property -dict { PACKAGE_PIN E3  IOSTANDARD LVCMOS33 } [get_ports { clk }]
create_clock -name sys_clk -period 10.000 [get_ports { clk }]

## ---- LED LD4 (green) — heartbeat ------------------------------------------
set_property -dict { PACKAGE_PIN H5  IOSTANDARD LVCMOS33 } [get_ports { led }]
