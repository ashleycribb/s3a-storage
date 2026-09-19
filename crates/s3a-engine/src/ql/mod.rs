pub mod ast;
pub mod lexer;
pub mod parser;
pub mod executor;

pub use ast::*;
pub use lexer::{Lexer, Token};
pub use parser::Parser;
pub use executor::{S3AQLEngine, QueryResult};

/// Convenience function to parse and execute a full S3A-QL / STQL query string.
pub fn execute_query(query: &str) -> Result<QueryResult, String> {
    let mut lexer = Lexer::new(query);
    let tokens = lexer.tokenize_all()?;
    let mut parser = Parser::new(tokens);
    let stmt = parser.parse_statement()?;
    S3AQLEngine::execute(&stmt)
}
