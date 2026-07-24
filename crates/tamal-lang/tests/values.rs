//! End-to-end: `const` + `bytes` + `crc8` fold an eSPI command phase at compile
//! time, byte-identically to hand-written assembly.

#[test]
fn command_phase_byte_matches_hand_written_asm() {
    // `send [..] + crc8` folds crc8([0x44,0x00,0x64]) = 0x16 at compile time.
    let tam = "const PUT_IORD1 = 0x44\n\
               test io {\n\
               \x20   cs_assert\n\
               \x20   send [PUT_IORD1, 0x00, 0x64] + crc8\n\
               \x20   tar 2\n\
               \x20   cs_deassert\n\
               \x20   halt 0x00\n\
               }\n";
    let reference = ".globl _start\n_start:\n\
                     \tcs_assert\n\
                     \tput_byte 0x44\n\tput_byte 0x00\n\tput_byte 0x64\n\tput_byte 0x16\n\
                     \ttar 2\n\tcs_deassert\n\thalt 0x00\n";
    let got = tamal_lang::compile(tam)
        .expect("compile .tam")
        .to_le_bytes();
    let want = tamal_asm::assemble(reference)
        .expect("assemble ref")
        .to_le_bytes();
    assert_eq!(
        got, want,
        "send+crc8 command phase must byte-match hand-written asm"
    );
}

#[test]
fn crc_region_byte_matches_the_same_command_phase() {
    let tam = "test io {\n\
               \x20   cs_assert\n\
               \x20   crc_region {\n\
               \x20       send [0x44]\n\
               \x20       send [0x00, 0x64]\n\
               \x20   }\n\
               \x20   tar 2\n\
               \x20   cs_deassert\n\
               \x20   halt 0x00\n\
               }\n";
    let reference = ".globl _start\n_start:\n\
                     \tcs_assert\n\
                     \tput_byte 0x44\n\tput_byte 0x00\n\tput_byte 0x64\n\tput_byte 0x16\n\
                     \ttar 2\n\tcs_deassert\n\thalt 0x00\n";
    let got = tamal_lang::compile(tam)
        .expect("compile .tam")
        .to_le_bytes();
    let want = tamal_asm::assemble(reference)
        .expect("assemble ref")
        .to_le_bytes();
    assert_eq!(got, want, "crc_region must fold the same 0x16 as `+ crc8`");
}

#[test]
fn deliberate_wrong_crc_folds_to_xored_byte() {
    // A deliberately-corrupted CRC is greppable and folds to 0x16 ^ 0xFF = 0xE9.
    let tam = "test bad_crc {\n\
               \x20   send [0x44, 0x00, 0x64] ++ [crc8([0x44, 0x00, 0x64]) ^ 0xFF]\n\
               \x20   halt 0x00\n\
               }\n";
    let asm = tamal_lang::lower_to_asm(tam).expect("lower");
    assert!(
        asm.contains("put_byte 0xE9"),
        "wrong CRC must fold to 0xE9:\n{asm}"
    );
    assert!(
        !asm.contains("put_byte 0x16"),
        "the correct CRC must not appear"
    );
}
