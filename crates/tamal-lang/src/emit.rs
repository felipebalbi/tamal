//! Emit: lower a `Module` (Plan-1 subset) to tamal-asm text. A `test` becomes
//! the entry label; `pass`/`fail`/raw statements become instruction lines.

use crate::parser::{Module, Stmt};

/// Lower a Plan-1 `Module` (exactly one test, enforced by the driver) to
/// tamal-asm text: the entry label followed by one line per statement.
pub fn emit(module: &Module) -> String {
    let mut out = String::new();
    for test in &module.tests {
        out.push_str(".globl _start\n");
        out.push_str("_start:\n");
        for stmt in &test.stmts {
            match stmt {
                Stmt::Pass => out.push_str("\thalt 0x00\n"),
                Stmt::Fail { code, .. } => {
                    out.push_str("\thalt ");
                    out.push_str(code);
                    out.push('\n');
                }
                Stmt::Raw { mnemonic, operands, .. } => {
                    out.push('\t');
                    out.push_str(mnemonic);
                    if !operands.is_empty() {
                        out.push(' ');
                        out.push_str(&operands.join(", "));
                    }
                    out.push('\n');
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{Module, Stmt, Test};

    fn one(stmts: Vec<Stmt>) -> Module {
        Module {
            tests: vec![Test { name: "t".into(), name_span: 0..1, stmts }],
        }
    }

    #[test]
    fn emits_entry_and_pass() {
        let asm = emit(&one(vec![Stmt::Pass]));
        assert_eq!(asm, ".globl _start\n_start:\n\thalt 0x00\n");
    }

    #[test]
    fn emits_fail_code_verbatim() {
        let asm = emit(&one(vec![Stmt::Fail { code: "0x11".into(), span: 0..1 }]));
        assert_eq!(asm, ".globl _start\n_start:\n\thalt 0x11\n");
    }

    #[test]
    fn emits_raw_instructions() {
        let asm = emit(&one(vec![
            Stmt::Raw { mnemonic: "cs_assert".into(), operands: vec![], span: 0..1 },
            Stmt::Raw {
                mnemonic: "mark".into(),
                operands: vec!["1".into(), "x1".into()],
                span: 0..1,
            },
            Stmt::Pass,
        ]));
        assert_eq!(
            asm,
            ".globl _start\n_start:\n\tcs_assert\n\tmark 1, x1\n\thalt 0x00\n"
        );
    }
}
