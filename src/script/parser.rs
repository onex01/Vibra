use alloc::vec;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::boxed::Box;
use super::lexer::Token;

#[derive(Debug, Clone)]
pub enum Expr {
    Number(i64),
    Str(String),
    Var(String),
    BinOp(Box<Expr>, BinOp, Box<Expr>),
    UnaryOp(UnOp, Box<Expr>),
    Call(String, Vec<Expr>),
}

#[derive(Debug, Clone)]
pub enum BinOp { Add, Sub, Mul, Div, Mod, Eq, Ne, Lt, Gt, Le, Ge }

#[derive(Debug, Clone)]
pub enum UnOp { Not, Neg }

#[derive(Debug, Clone)]
pub enum Stmt {
    VarDecl(String, Expr),
    Assign(String, Expr),
    Print(Vec<Expr>),
    If { cond: Expr, then_body: Vec<Stmt>, else_body: Vec<Stmt> },
    While { cond: Expr, body: Vec<Stmt> },
    For { var: String, start: Expr, end: Expr, body: Vec<Stmt> },
    Input(String),
    Beep(Expr),
    Sleep(Expr),
    Exit,
    Expr(Expr),
}

pub type Program = Vec<Stmt>;

struct Parser<'a> { tokens: &'a [Token], pos: usize }

pub fn parse(tokens: &[Token]) -> Result<Program, String> {
    let mut p = Parser { tokens, pos: 0 };
    let mut stmts = Vec::new();
    while p.pos < p.tokens.len() {
        match p.peek() {
            Token::Eof => break,
            Token::Newline | Token::Semicolon => { p.advance(); continue; }
            _ => stmts.push(p.stmt()?),
        }
    }
    Ok(stmts)
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Token { self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof) }
    fn advance(&mut self) -> Token { let t = self.peek(); self.pos += 1; t }
    fn expect(&mut self, t: &Token) -> Result<(), String> {
        let tok = self.advance();
        if &tok == t { Ok(()) } else { Err(alloc::format!("Expected {:?}, got {:?}", t, tok)) }
    }

    fn stmt(&mut self) -> Result<Stmt, String> {
        loop { match self.peek() { Token::Semicolon | Token::Newline => { self.advance(); }, _ => break } }
        match self.peek() {
            Token::Var => {
                self.advance();
                let name = match self.advance() { Token::Ident(n) => n, t => return Err(alloc::format!("Expected name, got {:?}", t)) };
                self.expect(&Token::Assign)?;
                let e = self.expr()?;
                Ok(Stmt::VarDecl(name, e))
            }
            Token::If => self.parse_if(),
            Token::While => self.parse_while(),
            Token::For => self.parse_for(),
            Token::Input => {
                self.advance();
                let prompt = match self.peek() {
                    Token::Str(s) => { let s = s.clone(); self.advance(); s }
                    _ => String::new(),
                };
                let var_name = match self.advance() {
                    Token::Ident(n) => n,
                    t => return Err(alloc::format!("Expected variable name after input, got {:?}", t)),
                };
                Ok(Stmt::Input(var_name))
            }
            Token::Print => {
                self.advance();
                let mut args = Vec::new();
                loop { match self.peek() { Token::Newline | Token::Eof | Token::Semicolon | Token::RBrace => break, _ => { args.push(self.expr()?); if self.peek() == Token::Comma { self.advance(); } } } }
                Ok(Stmt::Print(args))
            }
            Token::Beep => { self.advance(); Ok(Stmt::Beep(self.expr()?)) }
            Token::Sleep => { self.advance(); Ok(Stmt::Sleep(self.expr()?)) }
            Token::Exit => { self.advance(); Ok(Stmt::Exit) }
            Token::LBrace => {
                self.advance();
                let mut s = Vec::new();
                loop { match self.peek() { Token::RBrace | Token::Eof => { self.advance(); break; }, Token::Newline => { self.advance(); }, _ => s.push(self.stmt()?) } }
                Ok(Stmt::Expr(Expr::Number(0))) // block handled inline
            }
            Token::Ident(name) => {
                let n = name.clone(); self.advance();
                if self.peek() == Token::Assign { self.advance(); Ok(Stmt::Assign(n, self.expr()?)) }
                else { Ok(Stmt::Expr(Expr::Var(n))) }
            }
            Token::Newline => { self.advance(); self.stmt() }
            _ => { let e = self.expr()?; Ok(Stmt::Expr(e)) }
        }
    }

    fn parse_if(&mut self) -> Result<Stmt, String> {
        self.advance();
        let cond = self.expr()?;
        self.expect(&Token::LBrace)?;
        let mut then_body = Vec::new();
        loop { match self.peek() { Token::RBrace | Token::Eof => { self.advance(); break; }, Token::Newline => { self.advance(); }, _ => then_body.push(self.stmt()?) } }
        let else_body = if self.peek() == Token::Else {
            self.advance(); self.expect(&Token::LBrace)?;
            let mut eb = Vec::new();
            loop { match self.peek() { Token::RBrace | Token::Eof => { self.advance(); break; }, Token::Newline => { self.advance(); }, _ => eb.push(self.stmt()?) } }
            eb
        } else { Vec::new() };
        Ok(Stmt::If { cond, then_body, else_body })
    }

    fn parse_while(&mut self) -> Result<Stmt, String> {
        self.advance();
        let cond = self.expr()?;
        self.expect(&Token::LBrace)?;
        let mut body = Vec::new();
        loop { match self.peek() { Token::RBrace | Token::Eof => { self.advance(); break; }, Token::Newline | Token::Semicolon => { self.advance(); }, _ => body.push(self.stmt()?) } }
        Ok(Stmt::While { cond, body })
    }

    fn parse_for(&mut self) -> Result<Stmt, String> {
        self.advance(); // for
        let var_name = match self.advance() {
            Token::Ident(n) => n,
            t => return Err(alloc::format!("Expected variable name in for loop, got {:?}", t)),
        };
        self.expect(&Token::Assign)?;
        let start = self.expr()?;
        if self.peek() == Token::To || self.peek() == Token::Ident("until".into()) {
            self.advance();
        }
        let end = self.expr()?;

        // Пропускаем ; и { до тела
        loop { match self.peek() { Token::Semicolon | Token::Newline => { self.advance(); }, _ => break } }

        let body = if self.peek() == Token::LBrace {
            self.advance();
            let mut b = Vec::new();
            loop { match self.peek() { Token::RBrace | Token::Eof => { self.advance(); break; }, Token::Newline | Token::Semicolon => { self.advance(); }, _ => b.push(self.stmt()?) } }
            b
        } else {
            vec![self.stmt()?]
        };
        Ok(Stmt::For { var: var_name, start, end, body })
    }

    fn expr(&mut self) -> Result<Expr, String> {
        let mut left = self.term()?;
        loop { match self.peek() { Token::Plus => { self.advance(); left = Expr::BinOp(Box::new(left), BinOp::Add, Box::new(self.term()?)); }, Token::Minus => { self.advance(); left = Expr::BinOp(Box::new(left), BinOp::Sub, Box::new(self.term()?)); }, _ => break } }
        Ok(left)
    }

    fn term(&mut self) -> Result<Expr, String> {
        let mut left = self.factor()?;
        loop { match self.peek() { Token::Star => { self.advance(); left = Expr::BinOp(Box::new(left), BinOp::Mul, Box::new(self.factor()?)); }, Token::Slash => { self.advance(); left = Expr::BinOp(Box::new(left), BinOp::Div, Box::new(self.factor()?)); }, Token::Percent => { self.advance(); left = Expr::BinOp(Box::new(left), BinOp::Mod, Box::new(self.factor()?)); }, _ => break } }
        Ok(left)
    }

    fn factor(&mut self) -> Result<Expr, String> {
        // comparison
        let mut left = self.unary()?;
        loop { match self.peek() { Token::Eq => { self.advance(); left = Expr::BinOp(Box::new(left), BinOp::Eq, Box::new(self.unary()?)); }, Token::Ne => { self.advance(); left = Expr::BinOp(Box::new(left), BinOp::Ne, Box::new(self.unary()?)); }, Token::Lt => { self.advance(); left = Expr::BinOp(Box::new(left), BinOp::Lt, Box::new(self.unary()?)); }, Token::Gt => { self.advance(); left = Expr::BinOp(Box::new(left), BinOp::Gt, Box::new(self.unary()?)); }, Token::Le => { self.advance(); left = Expr::BinOp(Box::new(left), BinOp::Le, Box::new(self.unary()?)); }, Token::Ge => { self.advance(); left = Expr::BinOp(Box::new(left), BinOp::Ge, Box::new(self.unary()?)); }, _ => break } }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        match self.peek() { Token::Minus => { self.advance(); Ok(Expr::UnaryOp(UnOp::Neg, Box::new(self.atom()?))) }, Token::Not => { self.advance(); Ok(Expr::UnaryOp(UnOp::Not, Box::new(self.atom()?))) }, _ => self.atom() }
    }

    fn atom(&mut self) -> Result<Expr, String> {
        match self.peek().clone() {
            Token::Number(n) => { self.advance(); Ok(Expr::Number(n)) }
            Token::Str(s) => { self.advance(); Ok(Expr::Str(s)) }
            Token::Ident(name) => { self.advance(); Ok(Expr::Var(name)) }
            Token::LParen => { self.advance(); let e = self.expr()?; self.expect(&Token::RParen)?; Ok(e) }
            t => Err(alloc::format!("Unexpected: {:?}", t)),
        }
    }
}
