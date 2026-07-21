//! End-to-end: tamal-lang source compiles to the same bytecode as the
//! hand-written `.s` references, proving the full lex→parse→emit→assemble path.

#[test]
fn smoke_matches_hand_written_asm() {
    let prog = tamal_lang::compile("test smoke {\n    pass\n}\n").expect("compile smoke.tam");
    let reference =
        tamal_asm::assemble(include_str!("../../../examples/smoke_halt.s")).expect("assemble ref");
    assert_eq!(
        prog.to_le_bytes(),
        reference.to_le_bytes(),
        "smoke.tam must byte-match smoke_halt.s"
    );
    assert_eq!(prog.words().count(), 1);
}

#[test]
fn raw_instructions_pass_through_and_assemble() {
    // li x1, 0xDEAD (fits signed-21 -> 1 word) + mark 1, x1 + halt = 3 words.
    let tam = "test probe {\n    li x1, 0xDEAD\n    mark 1, x1\n    pass\n}\n";
    let prog = tamal_lang::compile(tam).expect("compile raw pass-through");
    assert_eq!(prog.words().count(), 3);
}
