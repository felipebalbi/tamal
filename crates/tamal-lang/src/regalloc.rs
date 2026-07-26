//! Register allocation: bind named runtime variables and scratch temporaries to
//! the tamal ISA's 15 usable physical registers (`x1`..`x15`; `x0` is the wired
//! zero and is never allocated). A scoped, lowest-free-first pool: entering a
//! scope (a `frame`, or later a `proc`) and leaving it frees everything bound
//! inside it, so a nested scope can never clobber an outer live value. There is
//! no spill target, so exhaustion is a hard error (spec D5, §7).

use std::collections::HashMap;
use tamal_abi::isa::Reg;
use tamal_asm::{Diagnostic, Span};

/// The number of physical registers (`x0`..`x15`); `x0` is reserved (zero).
const NUM_REGS: usize = 16;

/// One open lexical scope: the registers allocated in it (freed on exit) and
/// the names bound in it (dropped from the environment on exit).
#[derive(Default)]
struct Scope {
    regs: Vec<Reg>,
    names: Vec<String>,
}

/// A scoped, lowest-free-first register allocator over `x1`..`x15`.
///
/// `x0` is never allocated. `exit_scope` frees everything allocated since the
/// matching `enter_scope`. Exhaustion is a hard error — there is no spill.
pub struct RegAlloc {
    /// `busy[n]` is true when physical register `xn` is in use.
    busy: [bool; NUM_REGS],
    /// The scope stack; index 0 is the root (test) scope.
    scopes: Vec<Scope>,
    /// Every live name binding → its physical register.
    env: HashMap<String, Reg>,
}

impl RegAlloc {
    /// A fresh allocator with a single root scope and nothing allocated.
    pub fn new() -> Self {
        RegAlloc {
            busy: [false; NUM_REGS],
            scopes: vec![Scope::default()],
            env: HashMap::new(),
        }
    }

    /// Open a nested scope (a `frame`); its allocations are released by the
    /// matching [`RegAlloc::exit_scope`].
    pub fn enter_scope(&mut self) {
        self.scopes.push(Scope::default());
    }

    /// Close the innermost scope, freeing its registers and dropping its names.
    pub fn exit_scope(&mut self) {
        if let Some(scope) = self.scopes.pop() {
            for reg in scope.regs {
                self.busy[reg.bits() as usize] = false;
            }
            for name in scope.names {
                self.env.remove(&name);
            }
        }
    }

    /// Allocate the lowest free physical register, recording it in the current
    /// scope. Errors (no spill) when `x1`..`x15` are all in use.
    fn alloc(&mut self, span: &Span) -> Result<Reg, Diagnostic> {
        for n in 1..NUM_REGS {
            if !self.busy[n] {
                self.busy[n] = true;
                let reg = Reg::new(n as u8).expect("n < 16 fits a 5-bit register");
                self.scopes
                    .last_mut()
                    .expect("there is always a root scope")
                    .regs
                    .push(reg);
                return Ok(reg);
            }
        }
        Err(Diagnostic::error(
            span.clone(),
            "out of registers (the tamal ISA has only x1..x15 and no spill)",
        )
        .with_help("free a value by ending its scope, or use fewer live variables"))
    }

    /// Allocate an anonymous scratch register (freed by [`RegAlloc::free`] or at
    /// scope exit).
    pub fn temp(&mut self, span: &Span) -> Result<Reg, Diagnostic> {
        self.alloc(span)
    }

    /// Allocate a register and bind `name` to it for the current scope.
    pub fn bind(&mut self, name: String, span: &Span) -> Result<Reg, Diagnostic> {
        let reg = self.alloc(span)?;
        self.env.insert(name.clone(), reg);
        self.scopes
            .last_mut()
            .expect("there is always a root scope")
            .names
            .push(name);
        Ok(reg)
    }

    /// The register a name is currently bound to, if any.
    pub fn lookup(&self, name: &str) -> Option<Reg> {
        self.env.get(name).copied()
    }

    /// Release a register early (before its scope ends). Idempotent: freeing a
    /// register twice, or one already released by `exit_scope`, is harmless.
    pub fn free(&mut self, reg: Reg) {
        self.busy[reg.bits() as usize] = false;
        if let Some(scope) = self.scopes.last_mut() {
            scope.regs.retain(|&r| r != reg);
        }
    }
}

impl Default for RegAlloc {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(n: u8) -> Reg {
        Reg::new(n).unwrap()
    }

    #[test]
    fn binds_lowest_free_first_and_reuses_after_free() {
        let mut a = RegAlloc::new();
        assert_eq!(a.bind("first".into(), &(0..0)).unwrap(), r(1));
        assert_eq!(a.bind("second".into(), &(0..0)).unwrap(), r(2));
        a.free(r(1));
        // x1 is free again, so the next temp takes it.
        assert_eq!(a.temp(&(0..0)).unwrap(), r(1));
    }

    #[test]
    fn lookup_finds_bound_names() {
        let mut a = RegAlloc::new();
        let reg = a.bind("data".into(), &(0..0)).unwrap();
        assert_eq!(a.lookup("data"), Some(reg));
        assert_eq!(a.lookup("nope"), None);
    }

    #[test]
    fn exit_scope_frees_regs_and_names() {
        let mut a = RegAlloc::new();
        a.enter_scope();
        a.bind("inner".into(), &(0..0)).unwrap(); // x1
        a.bind("inner2".into(), &(0..0)).unwrap(); // x2
        a.exit_scope();
        assert_eq!(a.lookup("inner"), None);
        // both x1 and x2 are free again
        assert_eq!(a.temp(&(0..0)).unwrap(), r(1));
        assert_eq!(a.temp(&(0..0)).unwrap(), r(2));
    }

    #[test]
    fn never_allocates_x0_and_caps_at_15() {
        let mut a = RegAlloc::new();
        let mut regs = Vec::new();
        for _ in 0..15 {
            regs.push(a.temp(&(0..0)).unwrap());
        }
        // x1..x15, never x0
        assert_eq!(
            regs.iter().map(|r| r.bits()).collect::<Vec<_>>(),
            (1..=15).collect::<Vec<u8>>()
        );
        // the 16th allocation has no register left: a hard error, no spill.
        let err = a.temp(&(0..0)).unwrap_err();
        assert!(err.message.contains("out of registers"));
    }
}
