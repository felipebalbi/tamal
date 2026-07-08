# smoke_halt.s — minimal engine smoke test
#
# The smallest possible tamal program: halt immediately with status 0x00.
# It exercises nothing but the load → trigger → halt → trace-drain round-trip,
# so it is the first thing to run on a freshly-flashed board: if the drained
# trace comes back with a clean HALT, the whole host↔engine path is alive.
#
# No DUT, no bus activity, no logic analyzer required.
#
# Expected drained trace (tamal-loader):
#   REVISION 0.1.0
#   HALT  status=0x00  (ok)

    .equ  VERDICT_OK, 0x00       # host verdict code (written by halt)

    .text
    .globl _start
_start:
    halt VERDICT_OK              # end program; status 0x00 == pass
