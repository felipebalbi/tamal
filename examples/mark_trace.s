# mark_trace.s — trace/observability smoke test (MARK records)
#
# With no logic analyzer, MARK is how the engine talks back: each `mark`
# streams a tagged 32-bit register payload into the trace ring, which
# tamal-loader prints one line per record. This program marks two small
# constants and one large value (which forces the `li` lui+addi tiling),
# then halts 0x00 — so you can eyeball that computed register state round-
# trips over UART, in order, ahead of the terminating HALT.
#
# Instructions covered:  li (small + tiled), mark, halt
#
# Expected drained trace (tamal-loader) on a correct engine:
#   REVISION 0.1.0
#   [0] MARK     label=0x0001  payload=0x0000DEAD
#   [1] MARK     label=0x0002  payload=0x0000BEEF
#   [2] MARK     label=0x0003  payload=0x00123456
#   HALT  status=0x00  (ok)

    .equ  TAG_A,      1          # mark tags are numeric (0..2047), not labels
    .equ  TAG_B,      2
    .equ  TAG_C,      3
    .equ  VERDICT_OK, 0x00

    .text
    .globl _start
_start:
    li   t0, 0xDEAD             # small immediate: single load_imm
    mark TAG_A, t0
    li   t0, 0xBEEF             # small immediate: single load_imm
    mark TAG_B, t0
    li   t0, 0x123456           # > 2^20: forces li tiling (lui + addi)
    mark TAG_C, t0
    halt VERDICT_OK
