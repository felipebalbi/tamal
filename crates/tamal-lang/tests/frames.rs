//! End-to-end: a full eSPI Peripheral channel test written in tamal-lang lowers
//! to bytecode byte-identical *modulo register allocation* to the hand-written
//! `examples/peripheral_io_read.s`, and its `+ crc8` re-derives the 0x16 TX CRC.

mod common;

use common::canon;

/// The HLL peripheral I/O read. Plan 3 has no `import espi` yet (that is Plan 5),
/// so the command opcode is a local `const`; every other line is the domain
/// sugar this plan adds.
const PERIPHERAL_TAM: &str = "\
const PUT_IORD1 = 0x44

test io_read {
    config controller, x1, sck20, alert_pin
    frame {
        send [PUT_IORD1, 0x00, 0x64] + crc8
        tar 2
        wait_state
        recv data, status0, status1
        expect crc else 0x11
    }
    pass
}
";

#[test]
fn peripheral_io_read_byte_matches_asm_modulo_registers() {
    let reference = include_str!("../../../examples/peripheral_io_read.s");
    let got = tamal_lang::compile(PERIPHERAL_TAM).expect("compile .tam");
    let want = tamal_asm::assemble(reference).expect("assemble reference .s");
    assert_eq!(
        canon(&got),
        canon(&want),
        "the HLL peripheral read must match the hand-written .s modulo registers"
    );
}

#[test]
fn peripheral_io_read_lowers_to_the_expected_asm() {
    // Pins the allocator's lowest-free-first numbering and the gensym labels.
    // NB: gensym is a single shared counter, so `wait_state` takes `__wait0`
    // (counter -> 1) and the following `expect crc` takes `__fail1` (not
    // `__fail0`).
    let asm = tamal_lang::lower_to_asm(PERIPHERAL_TAM).expect("lower .tam");
    let expected = "\
.globl _start
_start:
\tset_config controller, x1, sck20, alert_pin
\tcs_assert
\tput_byte 0x44
\tput_byte 0x00
\tput_byte 0x64
\tput_byte 0x16
\ttar 2
__wait0:
\tcrc_reset
\tget_byte x1
\tli x2, 0x0F
\tbeq x1, x2, __wait0
\tget_byte x1
\tget_byte x2
\tget_byte x3
\tget_byte x4
\trdsr x4, crc
\tcs_deassert
\tbnez x4, __fail1
\thalt 0x00
__fail1:
\thalt 0x11
";
    assert_eq!(asm, expected);
}
