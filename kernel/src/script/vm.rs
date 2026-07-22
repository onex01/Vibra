use alloc::string::String;
use alloc::format;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::parser::{Expr, Stmt, BinOp, UnOp, Program};
use super::ScriptResult;

#[derive(Debug, Clone)]
pub enum Value { Num(i64), Str(String), Nil }

impl Value {
    fn to_num(&self) -> i64 { match self { Value::Num(n) => *n, Value::Str(s) => s.parse().unwrap_or(0), Value::Nil => 0 } }
    fn to_str(&self) -> String { match self { Value::Num(n) => format!("{}", n), Value::Str(s) => s.clone(), Value::Nil => String::from("null") } }
}

pub struct VM { vars: BTreeMap<String, Value>, console: *mut crate::framebuffer::Console }

impl VM {
    fn new(console: *mut crate::framebuffer::Console) -> Self { VM { vars: BTreeMap::new(), console } }
    fn get_var(&self, n: &str) -> Value { self.vars.get(n).cloned().unwrap_or(Value::Nil) }
    fn set_var(&mut self, n: &str, v: Value) { self.vars.insert(String::from(n), v); }
    fn print(&mut self, t: &str) { unsafe { (*self.console).print(t); } }
    fn println(&mut self, t: &str) { unsafe { (*self.console).print(t); (*self.console).put_char('\n'); } }

    fn eval(&mut self, e: &Expr) -> Value {
        match e {
            Expr::Number(n) => Value::Num(*n),
            Expr::Str(s) => Value::Str(s.clone()),
            Expr::Var(n) => self.get_var(n),
            Expr::BinOp(l, op, r) => {
                let lv = self.eval(l); let rv = self.eval(r);
                match op {
                    BinOp::Add => match (&lv, &rv) { (Value::Str(a), Value::Str(b)) => Value::Str(format!("{}{}", a, b)), _ => Value::Num(lv.to_num() + rv.to_num()) },
                    BinOp::Sub => Value::Num(lv.to_num() - rv.to_num()),
                    BinOp::Mul => Value::Num(lv.to_num() * rv.to_num()),
                    BinOp::Div => { let d = rv.to_num(); Value::Num(if d == 0 { 0 } else { lv.to_num() / d }) }
                    BinOp::Mod => Value::Num(lv.to_num() % rv.to_num()),
                    BinOp::Eq => Value::Num(if lv.to_num() == rv.to_num() { 1 } else { 0 }),
                    BinOp::Ne => Value::Num(if lv.to_num() != rv.to_num() { 1 } else { 0 }),
                    BinOp::Lt => Value::Num(if lv.to_num() < rv.to_num() { 1 } else { 0 }),
                    BinOp::Gt => Value::Num(if lv.to_num() > rv.to_num() { 1 } else { 0 }),
                    BinOp::Le => Value::Num(if lv.to_num() <= rv.to_num() { 1 } else { 0 }),
                    BinOp::Ge => Value::Num(if lv.to_num() >= rv.to_num() { 1 } else { 0 }),
                }
            }
            Expr::UnaryOp(op, e) => { let v = self.eval(e); match op { UnOp::Not => Value::Num(if v.to_num() == 0 { 1 } else { 0 }), UnOp::Neg => Value::Num(-v.to_num()) } }
            Expr::Call(name, args) => { let vals: Vec<Value> = args.iter().map(|a| self.eval(a)).collect(); self.call_fn(name, &vals) }
        }
    }

    fn call_fn(&mut self, name: &str, args: &[Value]) -> Value {
        match name {
            "len" => { if let Some(Value::Str(s)) = args.first() { Value::Num(s.len() as i64) } else { Value::Num(0) } }
            "abs" => Value::Num(args.first().map(|v| v.to_num().abs()).unwrap_or(0)),
            _ => { self.println(&format!("Unknown function: {}", name)); Value::Nil }
        }
    }

    pub fn run(&mut self, program: &Program) -> ScriptResult {
        for stmt in program {
            // Проверяем Ctrl+Z отмену
            if crate::is_cancelled() {
                crate::reset_cancel();
                self.println("\n[Script] Cancelled by user (Ctrl+Z)");
                return ScriptResult::Ok;
            }
            if let Err(e) = self.exec(stmt) { return ScriptResult::Error(e); }
        }
        ScriptResult::Ok
    }

    fn exec(&mut self, stmt: &Stmt) -> Result<(), String> {
        match stmt {
            Stmt::VarDecl(name, expr) => { let v = self.eval(expr); self.set_var(name, v); Ok(()) }
            Stmt::Assign(name, expr) => { let v = self.eval(expr); self.set_var(name, v); Ok(()) }
            Stmt::Print(exprs) => {
                let mut first = true;
                let strs: Vec<String> = exprs.iter().map(|e| self.eval(e).to_str()).collect(); let mut first = true; for s in &strs { if !first { self.print(" "); } self.print(s); first = false; }
                self.println(""); Ok(())
            }
            Stmt::If { cond, then_body, else_body } => {
                if self.eval(cond).to_num() != 0 { for s in then_body { self.exec(s)?; } }
                else { for s in else_body { self.exec(s)?; } }
                Ok(())
            }
            Stmt::While { cond, body } => {
                while self.eval(cond).to_num() != 0 { for s in body { self.exec(s)?; } }
                Ok(())
            }
            Stmt::Beep(expr) => { let f = self.eval(expr).to_num() as u32; crate::devices::pc_speaker::beep(f); Ok(()) }
            Stmt::Sleep(expr) => {
                let ticks = self.eval(expr).to_num() as u64;
                crate::task::sleep_task(crate::task::current_task_id().unwrap_or(0), ticks);
                crate::task::yield_now();
                Ok(())
            }
            Stmt::Exit => Ok(()),
            Stmt::Expr(expr) => { self.eval(expr); Ok(()) }
        }
    }
}

pub fn execute(program: &Program, console: &mut crate::framebuffer::Console) -> ScriptResult {
    let mut vm = VM::new(console as *mut _);
    vm.run(program)
}
