# branch_selfcheck.s — self-checking control-flow smoke test
#
# Exercises every CTRL-group branch and the jump pseudo-ops, checking BOTH
# the taken and the not-taken direction of each. A "TAKEN" test branches over
# a trap halt; a "NOT taken" test falls through past a trap halt. The first
# branch that behaves wrong halts with a unique non-zero status; if control
# flow is correct throughout it halts 0x00.
#
# Instructions covered:
#   beq, bne, bltu, bgeu (taken + not-taken each), j, beqz, bnez
#
# Expected drained trace (tamal-loader) on a correct engine:
#   HALT  status=0x00  (ok)
# Any status 0xB1..0xBD => that specific branch direction misbehaved.

    .equ  PASS,        0x00
    .equ  E_BEQ_T,     0xB1      # beq failed to take when equal
    .equ  E_BEQ_NT,    0xB2      # beq took when not equal
    .equ  E_BNE_T,     0xB3
    .equ  E_BNE_NT,    0xB4
    .equ  E_BLTU_T,    0xB5      # bltu failed to take when a < b
    .equ  E_BLTU_NT,   0xB6      # bltu took when a >= b
    .equ  E_BGEU_T,    0xB7
    .equ  E_BGEU_NT,   0xB8
    .equ  E_J,         0xB9      # unconditional j did not jump
    .equ  E_BEQZ_T,    0xBA
    .equ  E_BEQZ_NT,   0xBB
    .equ  E_BNEZ_T,    0xBC
    .equ  E_BNEZ_NT,   0xBD

    .text
    .globl _start
_start:
    # beq TAKEN: 7 == 7 must branch over the trap
    li   t0, 7
    li   t1, 7
    beq  t0, t1, beq_nt
    halt E_BEQ_T
beq_nt:
    # beq NOT taken: 7 == 9 must fall through
    li   t1, 9
    beq  t0, t1, fail_beq_nt

    # bne TAKEN: 7 != 9 must branch
    bne  t0, t1, bne_nt
    halt E_BNE_T
bne_nt:
    # bne NOT taken: 9 == 9 must fall through
    li   t0, 9
    bne  t0, t1, fail_bne_nt

    # bltu TAKEN: 3 < 5 (unsigned) must branch
    li   t0, 3
    li   t1, 5
    bltu t0, t1, bltu_nt
    halt E_BLTU_T
bltu_nt:
    # bltu NOT taken: 5 < 3 ? no -> fall through
    bltu t1, t0, fail_bltu_nt

    # bgeu TAKEN: 5 >= 3 (unsigned) must branch
    bgeu t1, t0, bgeu_nt
    halt E_BGEU_T
bgeu_nt:
    # bgeu NOT taken: 3 >= 5 ? no -> fall through
    bgeu t0, t1, fail_bgeu_nt

    # j: unconditional jump must skip the trap
    j    j_ok
    halt E_J
j_ok:
    # beqz TAKEN: (t0 == 0) must branch
    li   t0, 0
    beqz t0, beqz_nt
    halt E_BEQZ_T
beqz_nt:
    # beqz NOT taken: 4 != 0 -> fall through
    li   t0, 4
    beqz t0, fail_beqz_nt

    # bnez TAKEN: 4 != 0 must branch
    bnez t0, bnez_nt
    halt E_BNEZ_T
bnez_nt:
    # bnez NOT taken: 0 -> fall through
    li   t0, 0
    bnez t0, fail_bnez_nt

    halt PASS                    # every branch direction was correct

    # --- failure exits for the not-taken checks (reached only if mis-taken) ---
fail_beq_nt:   halt E_BEQ_NT
fail_bne_nt:   halt E_BNE_NT
fail_bltu_nt:  halt E_BLTU_NT
fail_bgeu_nt:  halt E_BGEU_NT
fail_beqz_nt:  halt E_BEQZ_NT
fail_bnez_nt:  halt E_BNEZ_NT
