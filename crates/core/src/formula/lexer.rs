use super::error::FormulaError; // the error type tokenize can fail with

// Every kind of lexical token a formula string can be broken into.
// `PartialEq` lets tests compare a produced token stream against an
// expected `Vec<Token>` with `assert_eq!`.
#[derive(Debug, Clone, PartialEq)] // Debug: printable in test failures; Clone: tokens get copied while parsing; PartialEq: enables assert_eq! in tests
pub enum Token {
    Number(f64),   // a numeric literal, already converted from its source text to an f64
    Ident(String), // a run of letters/digits/underscores starting with a letter or underscore — a variable or function name
    Plus,          // the '+' character
    Minus, // the '-' character (binary subtraction or unary negation; the parser decides which)
    Star,  // the '*' character
    Slash, // the '/' character
    Caret, // the '^' character, used for exponentiation
    LParen, // the '(' character
    RParen, // the ')' character
    Comma, // the ',' character, separates arguments inside a function call
    Eof,   // sentinel appended after the real tokens, marking "no more input"
}

// Converts formula source text into a flat list of tokens, ending in `Eof`.
// Never panics: every character that isn't whitespace, part of a number,
// part of an identifier, or one of the single-character symbols below
// produces a `FormulaError::UnexpectedToken` instead of a panic or an
// out-of-bounds access.
pub fn tokenize(source: &str) -> Result<Vec<Token>, FormulaError> {
    let chars: Vec<char> = source.chars().collect(); // collect into a Vec so positions can be indexed and looked ahead without re-scanning UTF-8
    let mut tokens = Vec::new(); // the token list being built up, in source order
    let mut pos = 0; // index into `chars` of the next character to examine

    while pos < chars.len() {
        // loop until every character has been consumed
        let c = chars[pos]; // the character at the current position, not yet consumed

        if c.is_whitespace() {
            // spaces/tabs/newlines carry no meaning between tokens
            pos += 1; // skip past this one whitespace character
            continue; // go straight to the next iteration without emitting a token
        }

        if c.is_ascii_digit() || c == '.' {
            // a digit or a leading '.' begins a numeric literal
            let start = pos; // remember where the literal starts, to slice it out once it ends
            let mut seen_dot = c == '.'; // tracks whether a '.' has already appeared, so a second one isn't consumed
            pos += 1; // consume the character that started the literal
            while pos < chars.len() {
                // keep extending the literal while more valid characters follow
                let next = chars[pos]; // the next character being considered for inclusion
                if next.is_ascii_digit() {
                    // digits always continue a numeric literal
                    pos += 1; // consume this digit
                } else if next == '.' && !seen_dot {
                    // a '.' is allowed, but only the first one
                    seen_dot = true; // remember a decimal point has now been used
                    pos += 1; // consume the decimal point
                } else {
                    break; // any other character ends the numeric literal
                }
            }
            let text: String = chars[start..pos].iter().collect(); // rebuild the literal's source text from its char range
            let value: f64 = text.parse().map_err(|_| FormulaError::UnexpectedToken {
                // parsing fails only on malformed input like "1.2.3" or a lone "."
                found: text.clone(), // report the exact text that failed to parse as a number
                pos: start,          // and the character position where that text began
            })?; // propagate a parse failure as a proper FormulaError rather than unwrapping
            tokens.push(Token::Number(value)); // record the fully parsed numeric literal
            continue; // resume the outer loop at whatever character follows the literal
        }

        if c.is_alphabetic() || c == '_' {
            // identifiers must start with a letter or underscore, never a digit
            let start = pos; // remember where the identifier starts
            pos += 1; // consume its first character
            while pos < chars.len() && (chars[pos].is_alphanumeric() || chars[pos] == '_') {
                // letters, digits, and underscores may all continue an identifier after the first character
                pos += 1; // consume each such character
            }
            let text: String = chars[start..pos].iter().collect(); // rebuild the identifier's source text
            tokens.push(Token::Ident(text)); // record it; the parser later decides if it's a variable or a function name
            continue; // resume the outer loop at whatever character follows the identifier
        }

        // everything else must be exactly one of these single-character symbols
        let token = match c {
            '+' => Token::Plus,   // addition
            '-' => Token::Minus,  // subtraction / unary negation
            '*' => Token::Star,   // multiplication
            '/' => Token::Slash,  // division
            '^' => Token::Caret,  // exponentiation
            '(' => Token::LParen, // opens a grouped expression or a call's argument list
            ')' => Token::RParen, // closes a grouped expression or a call's argument list
            ',' => Token::Comma,  // separates arguments within a call
            other => {
                // any character that isn't whitespace, digit, letter, '_', or a symbol above is invalid
                return Err(FormulaError::UnexpectedToken {
                    found: other.to_string(), // report exactly which character was rejected
                    pos,                      // and exactly where it occurred
                });
            }
        };
        tokens.push(token); // record the matched single-character token
        pos += 1; // consume the one character that produced it
    }

    tokens.push(Token::Eof); // append the end-of-stream sentinel so the parser always has a definite stopping point
    Ok(tokens) // hand back the complete token stream
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_arithmetic_expression() {
        let tokens = tokenize("1+2*3").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Number(1.0),
                Token::Plus,
                Token::Number(2.0),
                Token::Star,
                Token::Number(3.0),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn tokenizes_function_call() {
        let tokens = tokenize("sin(A1)").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::Ident("sin".to_string()),
                Token::LParen,
                Token::Ident("A1".to_string()),
                Token::RParen,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn tokenizes_unary_minus_and_decimal() {
        let tokens = tokenize("-3.5").unwrap();
        assert_eq!(tokens, vec![Token::Minus, Token::Number(3.5), Token::Eof]);
    }

    #[test]
    fn tokenizes_parenthesized_expression() {
        let tokens = tokenize("(1+2)*3").unwrap();
        assert_eq!(
            tokens,
            vec![
                Token::LParen,
                Token::Number(1.0),
                Token::Plus,
                Token::Number(2.0),
                Token::RParen,
                Token::Star,
                Token::Number(3.0),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn rejects_invalid_character() {
        let err = tokenize("1+@2").unwrap_err();
        assert!(matches!(
            err,
            FormulaError::UnexpectedToken { found, .. } if found == "@"
        ));
    }

    #[test]
    fn tokenizes_empty_input_to_just_eof() {
        assert_eq!(tokenize("").unwrap(), vec![Token::Eof]);
    }

    #[test]
    fn never_panics_on_garbage_input() {
        for input in ["@@@", "$$$", "1.2.3", "....", "\0\0\0", "🙂🙂"] {
            let _ = tokenize(input);
        }
    }
}
