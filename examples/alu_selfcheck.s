# alu_selfcheck.s — self-checking DATA-group (ALU) smoke test
#
# Exercises the RV32I-flavored ALU / immediate / shift instructions and
# CHECKS each result against a known-good value. On the first mismatch it
# halts with a unique non-zero status naming the failed op; if every check
# passes it halts 0x00. No DUT and no logic analyzer needed — you read the
# verdict straight off the drained HALT status byte.
#
# Instructions covered:
#   li / load_imm, lui, mov, add, sub, and, or, xor,
#   addi (incl. negative imm), andi, ori, xori, sll, srl, sra
#
# Expected drained trace (tamal-loader) on a correct engine:
#   HALT  status=0x00  (ok)
# Any other status 0xA1..0xAF => that specific ALU check failed.

    .equ  PASS,    0x00
    .equ  E_LI,    0xA1          # li / load_imm mismatch
    .equ  E_LUI,   0xA2          # lui (imm << 11) mismatch
    .equ  E_MOV,   0xA3          # mov mismatch
    .equ  E_ADD,   0xA4
    .equ  E_SUB,   0xA5
    .equ  E_AND,   0xA6
    .equ  E_OR,    0xA7
    .equ  E_XOR,   0xA8
    .equ  E_ADDI,  0xA9          # addi (positive and negative imm)
    .equ  E_ANDI,  0xAA
    .equ  E_ORI,   0xAB
    .equ  E_XORI,  0xAC
    .equ  E_SLL,   0xAD
    .equ  E_SRL,   0xAE
    .equ  E_SRA,   0xAF          # arithmetic (sign-preserving) right shift

    .text
    .globl _start
_start:
    # li vs addi: two independent immediate paths must agree (== 100)
    li   t0, 100
    addi t1, zero, 100
    bne  t0, t1, fail_li

    # lui writes imm << 11:  lui 1 => 0x800
    lui  t0, 1
    li   t1, 0x800
    bne  t0, t1, fail_lui

    # mov copies a register
    li   t0, 0x55
    mov  t2, t0
    li   t1, 0x55
    bne  t2, t1, fail_mov

    # add:  5 + 8 == 13
    li   t0, 5
    li   t1, 8
    add  t2, t0, t1
    li   s0, 13
    bne  t2, s0, fail_add

    # sub:  20 - 7 == 13
    li   t0, 20
    li   t1, 7
    sub  t2, t0, t1
    li   s0, 13
    bne  t2, s0, fail_sub

    # bitwise ops share the operands 0xF0 and 0x3C
    li   t0, 0xF0
    li   t1, 0x3C
    and  t2, t0, t1              # 0xF0 & 0x3C == 0x30
    li   s0, 0x30
    bne  t2, s0, fail_and
    or   t2, t0, t1             # 0xF0 | 0x3C == 0xFC
    li   s0, 0xFC
    bne  t2, s0, fail_or
    xor  t2, t0, t1             # 0xF0 ^ 0x3C == 0xCC
    li   s0, 0xCC
    bne  t2, s0, fail_xor

    # addi with a positive then a negative immediate:  5 + 10 - 5 == 10
    li   t0, 5
    addi t2, t0, 10
    li   s0, 15
    bne  t2, s0, fail_addi
    addi t2, t2, -5
    li   s0, 10
    bne  t2, s0, fail_addi

    # andi / ori / xori against 0x0F
    li   t0, 0xFF
    andi t2, t0, 0x0F           # 0xFF & 0x0F == 0x0F
    li   s0, 0x0F
    bne  t2, s0, fail_andi
    li   t0, 0xF0
    ori  t2, t0, 0x0F           # 0xF0 | 0x0F == 0xFF
    li   s0, 0xFF
    bne  t2, s0, fail_ori
    li   t0, 0xFF
    xori t2, t0, 0x0F           # 0xFF ^ 0x0F == 0xF0
    li   s0, 0xF0
    bne  t2, s0, fail_xori

    # shifts
    li   t0, 1
    sll  t2, t0, 4             # 1 << 4 == 0x10
    li   s0, 0x10
    bne  t2, s0, fail_sll
    li   t0, 0x100
    srl  t2, t0, 4             # 0x100 >> 4 == 0x10 (logical)
    li   s0, 0x10
    bne  t2, s0, fail_srl
    li   t0, -16
    sra  t2, t0, 2            # -16 >> 2 == -4 (arithmetic, sign kept)
    li   s0, -4
    bne  t2, s0, fail_sra

    halt PASS                   # every ALU check passed

    # --- failure exits: reached only by a mismatched branch above ---
fail_li:    halt E_LI
fail_lui:   halt E_LUI
fail_mov:   halt E_MOV
fail_add:   halt E_ADD
fail_sub:   halt E_SUB
fail_and:   halt E_AND
fail_or:    halt E_OR
fail_xor:   halt E_XOR
fail_addi:  halt E_ADDI
fail_andi:  halt E_ANDI
fail_ori:   halt E_ORI
fail_xori:  halt E_XORI
fail_sll:   halt E_SLL
fail_srl:   halt E_SRL
fail_sra:   halt E_SRA
