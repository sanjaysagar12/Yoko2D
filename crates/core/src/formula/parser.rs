use super::error::FormulaError; // the error type every parsing function can fail with
use super::lexer::Token; // the token kind produced by `tokenize` and consumed here

// The abstract syntax tree a formula string parses into. `evaluate` (in
// `eval.rs`) walks this tree to compute a number.
#[derive(Debug, Clone, PartialEq)] // Debug: printable in test failures; Clone: sub-trees get cloned in a couple of test helpers; PartialEq: enables assert_eq! in tests
pub enum Expr {
    Number(f64),      // a literal number, e.g. `3.14`
    Variable(String), // a bare identifier not followed by `(`, e.g. `A1`
    Neg(Box<Expr>), // unary negation, e.g. `-x`; boxed because `Expr` would otherwise be infinitely sized
    BinOp {
        op: BinOp,      // which operator this node applies: +, -, *, /, or ^
        lhs: Box<Expr>, // the left operand, boxed for the same reason as `Neg`'s operand
        rhs: Box<Expr>, // the right operand
    },
    Call {
        name: String,    // the function's name as written in the formula, e.g. "sin"
        args: Vec<Expr>, // the parsed argument expressions, in the order they appeared
    },
}

// The binary operators `Expr::BinOp` can carry.
#[derive(Debug, Clone, PartialEq)] // same rationale as the derives on `Expr` above
pub enum BinOp {
    Add, // '+'
    Sub, // '-'
    Mul, // '*'
    Div, // '/'
    Pow, // '^'
}

// A cursor over a token slice, used internally by the recursive-descent
// functions below. Not public: callers only ever see the `parse` entry
// point at the bottom of this file.
struct Parser<'a> {
    tokens: &'a [Token], // the full token stream to parse, borrowed for as long as parsing runs
    pos: usize, // index of the next not-yet-consumed token; always valid since `tokens` ends in `Eof`
}

impl<'a> Parser<'a> {
    // Returns the next token without consuming it, so grammar functions can
    // decide what to do before committing to consuming it.
    fn peek(&self) -> &Token {
        &self.tokens[self.pos] // `tokenize` always appends `Eof`, so this index never goes out of bounds
    }

    // Consumes and returns the next token, advancing the internal cursor.
    fn advance(&mut self) -> Token {
        let tok = self.tokens[self.pos].clone(); // copy out the token before the cursor moves past it
        if self.pos + 1 < self.tokens.len() {
            // stop advancing once `Eof` is reached, so repeated calls keep returning `Eof` instead of panicking
            self.pos += 1; // move the cursor to the next token
        }
        tok // hand back the token that was just consumed
    }

    // Consumes the next token if it equals `expected`; otherwise produces a
    // parse error describing what was found instead.
    fn expect(&mut self, expected: Token) -> Result<(), FormulaError> {
        if *self.peek() == expected {
            // the very next token is exactly what the grammar requires here
            self.advance(); // consume it and continue
            Ok(())
        } else if *self.peek() == Token::Eof {
            // input ran out before the required token appeared
            Err(FormulaError::UnexpectedEof) // report end-of-input specifically, not a generic mismatch
        } else {
            // some other, wrong token is sitting where `expected` was required
            Err(FormulaError::UnexpectedToken {
                found: format!("{:?}", self.peek()), // describe the token that was actually found
                pos: self.pos,                       // its index in the token stream
            })
        }
    }

    // Implements: expr := term (("+" | "-") term)*
    fn parse_expr(&mut self) -> Result<Expr, FormulaError> {
        let mut lhs = self.parse_term()?; // the first term is the initial left-hand side of the chain
        loop {
            // fold in as many trailing "+ term" / "- term" pieces as appear
            match self.peek() {
                Token::Plus => {
                    // a '+' introduces another term to add
                    self.advance(); // consume the '+'
                    let rhs = self.parse_term()?; // parse the term being added
                    lhs = Expr::BinOp {
                        op: BinOp::Add,
                        lhs: Box::new(lhs), // everything folded so far becomes the new left operand
                        rhs: Box::new(rhs),
                    };
                }
                Token::Minus => {
                    // a '-' introduces another term to subtract
                    self.advance(); // consume the '-'
                    let rhs = self.parse_term()?; // parse the term being subtracted
                    lhs = Expr::BinOp {
                        op: BinOp::Sub,
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                    };
                }
                _ => break, // no '+'/'-' next: the expr rule is complete
            }
        }
        Ok(lhs) // the fully left-associative chain of +/- operations
    }

    // Implements: term := unary (("*" | "/") unary)*
    //
    // NOTE ON GRAMMAR SHAPE: a naive `power := unary ("^" power)?` /
    // `unary := "-" unary | primary` split (unary directly wrapping
    // primary, with power built on top of unary) makes "-2^2" parse as
    // `(-2)^2 == 4.0`, because the '-' is consumed and bound to the base
    // *before* `power` ever sees the '^'. That contradicts the required
    // "-2^2 == -4.0" (unary minus binds *looser* than '^', matching every
    // calculator and Python's `-2**2 == -4`). To get that, `unary` is
    // placed *above* `power` here — `term` calls `unary`, `unary` falls
    // through to `power`, and `power`'s exponent slot recurses into `unary`
    // (not `power`) so a signed exponent like "2^-2" still parses without
    // parentheses while remaining right-associative. See `parse_unary` and
    // `parse_power` below for exactly where that split happens.
    fn parse_term(&mut self) -> Result<Expr, FormulaError> {
        let mut lhs = self.parse_unary()?; // the first unary-expression is the initial left-hand side
        loop {
            // fold in as many trailing "* unary" / "/ unary" pieces as appear
            match self.peek() {
                Token::Star => {
                    // a '*' introduces another factor to multiply by
                    self.advance(); // consume the '*'
                    let rhs = self.parse_unary()?; // parse the factor being multiplied
                    lhs = Expr::BinOp {
                        op: BinOp::Mul,
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                    };
                }
                Token::Slash => {
                    // a '/' introduces another factor to divide by
                    self.advance(); // consume the '/'
                    let rhs = self.parse_unary()?; // parse the divisor
                    lhs = Expr::BinOp {
                        op: BinOp::Div,
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                    };
                }
                _ => break, // no '*' or '/' next: the term rule is complete
            }
        }
        Ok(lhs) // the fully left-associative chain of */÷ operations
    }

    // Implements: unary := "-" unary | power
    //
    // Sits above `power` (see the note on `parse_term` above) so a leading
    // '-' applies to an entire "base ^ exponent" chain rather than binding
    // only to the base.
    fn parse_unary(&mut self) -> Result<Expr, FormulaError> {
        if matches!(self.peek(), Token::Minus) {
            // a leading '-' is unary negation, not subtraction, at this point in the grammar
            self.advance(); // consume the '-'
            let operand = self.parse_unary()?; // recurse so repeated '-' (e.g. "--3") stacks correctly
            Ok(Expr::Neg(Box::new(operand)))
        } else {
            self.parse_power() // no leading '-': fall through to a power expression
        }
    }

    // Implements: power := primary ("^" unary)?     (right-associative)
    //
    // The exponent slot recurses into `unary` (not `power`) so "2^-2"
    // parses as `2^(-2)` without needing parentheses, while "2^3^2" still
    // groups right-to-left as `2^(3^2)` since neither operand there has a
    // leading '-' for `unary` to consume before falling through to `power`.
    fn parse_power(&mut self) -> Result<Expr, FormulaError> {
        let base = self.parse_primary()?; // the base of a possible exponentiation; no leading '-' allowed here, that's `unary`'s job
        if matches!(self.peek(), Token::Caret) {
            // a '^' means this is an exponentiation
            self.advance(); // consume the '^'
            let exponent = self.parse_unary()?; // recurse into unary so a signed exponent and right-associativity both work
            Ok(Expr::BinOp {
                op: BinOp::Pow,
                lhs: Box::new(base),
                rhs: Box::new(exponent),
            })
        } else {
            Ok(base) // no '^' follows: the base alone is the result
        }
    }

    // Implements: primary := NUMBER | IDENT ("(" (expr ("," expr)*)? ")")? | "(" expr ")"
    fn parse_primary(&mut self) -> Result<Expr, FormulaError> {
        match self.advance() {
            // every primary starts with exactly one token, so consume it up front
            Token::Number(value) => Ok(Expr::Number(value)), // a bare numeric literal
            Token::Ident(name) => {
                // an identifier: either a plain variable or, if '(' follows, a function call
                if matches!(self.peek(), Token::LParen) {
                    // a following '(' means this identifier names a function being called
                    self.advance(); // consume the '('
                    let mut args = Vec::new(); // collects the parsed argument expressions, in order
                    if !matches!(self.peek(), Token::RParen) {
                        // a ')' immediately after '(' means zero arguments; otherwise at least one follows
                        args.push(self.parse_expr()?); // parse the first argument
                        while matches!(self.peek(), Token::Comma) {
                            // a ',' introduces another argument
                            self.advance(); // consume the ','
                            args.push(self.parse_expr()?); // parse the next argument
                        }
                    }
                    self.expect(Token::RParen)?; // the argument list must be closed
                    Ok(Expr::Call { name, args })
                } else {
                    Ok(Expr::Variable(name)) // no '(' follows: this identifier is a plain variable reference
                }
            }
            Token::LParen => {
                // a parenthesized sub-expression
                let inner = self.parse_expr()?; // parse whatever expression is inside the parentheses
                self.expect(Token::RParen)?; // it must be closed with a matching ')'
                Ok(inner) // the parentheses themselves don't produce an AST node
            }
            Token::Eof => Err(FormulaError::UnexpectedEof), // input ran out exactly where a primary was required
            other => Err(FormulaError::UnexpectedToken {
                // any other token (an operator, ')', ',') can't start a primary expression
                found: format!("{other:?}"), // describe the token that was found instead
                pos: self.pos.saturating_sub(1), // best-effort: the position of the token just consumed by `advance` above
            }),
        }
    }
}

// Entry point: parses a full token stream (as produced by `tokenize`) into
// a single `Expr`, and verifies every token was consumed — trailing tokens
// after an otherwise-valid expression (e.g. "1+2)") are a parse error, not
// silently ignored.
pub fn parse(tokens: &[Token]) -> Result<Expr, FormulaError> {
    let mut parser = Parser { tokens, pos: 0 }; // start a fresh cursor at the beginning of the stream
    let expr = parser.parse_expr()?; // parse the top-level `expr` grammar rule
    if *parser.peek() != Token::Eof {
        // anything left over after a complete expression means the input wasn't fully consumed
        return Err(FormulaError::UnexpectedToken {
            found: format!("{:?}", parser.peek()), // describe the unexpected leftover token
            pos: parser.pos,                       // where it starts in the token stream
        });
    }
    Ok(expr) // the entire token stream parsed cleanly into one expression
}

#[cfg(test)]
mod tests {
    use super::super::lexer::tokenize;
    use super::*;

    fn parse_str(source: &str) -> Expr {
        parse(&tokenize(source).unwrap()).unwrap()
    }

    #[test]
    fn parses_precedence_of_plus_and_star() {
        // 1+2*3 should parse as 1+(2*3), i.e. '*' binds tighter than '+'
        assert_eq!(
            parse_str("1+2*3"),
            Expr::BinOp {
                op: BinOp::Add,
                lhs: Box::new(Expr::Number(1.0)),
                rhs: Box::new(Expr::BinOp {
                    op: BinOp::Mul,
                    lhs: Box::new(Expr::Number(2.0)),
                    rhs: Box::new(Expr::Number(3.0)),
                }),
            }
        );
    }

    #[test]
    fn parses_parenthesized_grouping() {
        assert_eq!(
            parse_str("(1+2)*3"),
            Expr::BinOp {
                op: BinOp::Mul,
                lhs: Box::new(Expr::BinOp {
                    op: BinOp::Add,
                    lhs: Box::new(Expr::Number(1.0)),
                    rhs: Box::new(Expr::Number(2.0)),
                }),
                rhs: Box::new(Expr::Number(3.0)),
            }
        );
    }

    #[test]
    fn power_is_right_associative() {
        // 2^3^2 should parse as 2^(3^2), not (2^3)^2
        assert_eq!(
            parse_str("2^3^2"),
            Expr::BinOp {
                op: BinOp::Pow,
                lhs: Box::new(Expr::Number(2.0)),
                rhs: Box::new(Expr::BinOp {
                    op: BinOp::Pow,
                    lhs: Box::new(Expr::Number(3.0)),
                    rhs: Box::new(Expr::Number(2.0)),
                }),
            }
        );
    }

    #[test]
    fn unary_minus_binds_looser_than_power() {
        // -2^2 should parse as -(2^2), not (-2)^2
        assert_eq!(
            parse_str("-2^2"),
            Expr::Neg(Box::new(Expr::BinOp {
                op: BinOp::Pow,
                lhs: Box::new(Expr::Number(2.0)),
                rhs: Box::new(Expr::Number(2.0)),
            }))
        );
    }

    #[test]
    fn parses_function_call_with_multiple_args() {
        assert_eq!(
            parse_str("max(1,2)"),
            Expr::Call {
                name: "max".to_string(),
                args: vec![Expr::Number(1.0), Expr::Number(2.0)],
            }
        );
    }

    #[test]
    fn parses_bare_identifier_as_variable() {
        assert_eq!(parse_str("A1"), Expr::Variable("A1".to_string()));
    }

    #[test]
    fn malformed_inputs_each_fail_to_parse_without_panicking() {
        let malformed = ["1+", "(1+2", "+", ""];
        for source in malformed {
            let tokens = tokenize(source).unwrap(); // tokenizing itself always succeeds on this input set
            let result = parse(&tokens); // parsing is expected to fail for each of these
            assert!(result.is_err(), "expected parse error for {source:?}");
        }
    }

    #[test]
    fn trailing_tokens_after_valid_expression_are_an_error() {
        let tokens = tokenize("1+2)").unwrap();
        assert!(parse(&tokens).is_err());
    }
}
