pub mod ast;
pub mod error;
pub mod executor;
pub mod lexer;
pub mod parser;

pub use ast::*;
pub use error::QlError;
pub use executor::{
    register_pack_sift_handler, PackSiftHandler, QueryResult, S3AQLEngine,
};
pub use lexer::{Lexer, Token};
pub use parser::Parser;

/// Convenience function to parse and execute a full S3A-QL / STQL query string.
pub fn execute_query(query: &str) -> Result<QueryResult, QlError> {
    let mut lexer = Lexer::new(query);
    let tokens = lexer.tokenize_all()?;
    let mut parser = Parser::new(tokens);
    let stmt = parser.parse_statement()?;
    S3AQLEngine::execute(&stmt)
}
