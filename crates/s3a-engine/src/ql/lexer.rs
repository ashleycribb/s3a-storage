use crate::ql::error::QlError;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Keywords
    Fetch,
    Record,
    At,
    From,
    Sift,
    Where,
    And,
    Between,
    Limit,
    Insert,
    Into,
    Delete,
    Compact,
    Archive,
    Fuse,
    Tiles,
    With,
    True,
    False,
    Version,
    Pragma,

    // Domain keywords
    Telemetry,
    Kinematics,
    GisMesh,
    Embeddings,
    DaCommitments,
    LearningActivities,
    HumanLrs,
    AiTraces,
    AcademicPapers,

    // Condition keywords
    Time,
    SensorId,
    MetricId,
    ActorId,
    VerbId,
    SpatialBox,
    In,
    Simplex,
    Min,
    Max,
    Latitude,
    Longitude,
    Elevation,
    Similarity,
    To,
    Using,
    Cosine,
    DotProduct,
    Threshold,
    BlockHeight,
    Compose,
    Weight,
    SpatialProximity,
    Decay,
    Halflife,

    // Literals
    Coordinate(String),
    StringLiteral(String),
    Number(f64),
    Identifier(String),

    // Symbols
    Comma,
    Equal,
    Semicolon,
    LParen,
    RParen,
    LBracket,
    RBracket,

    Eof,
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

pub struct Lexer<'a> {
    _input: &'a str,
    chars: Vec<char>,
    pos: usize,
    pub line: u32,
    pub col: u32,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            _input: input,
            chars: input.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.chars.get(self.pos).copied();
        if let Some(c) = ch {
            self.pos += 1;
            if c == '\n' {
                self.line += 1;
                self.col = 1;
            } else {
                self.col += 1;
            }
        }
        ch
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() {
                self.advance();
            } else if ch == '-'
                && self.pos + 1 < self.chars.len()
                && self.chars[self.pos + 1] == '-'
            {
                // SQL-style single-line comment '--'
                while let Some(c) = self.advance() {
                    if c == '\n' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    pub fn next_token(&mut self) -> Result<Token, QlError> {
        self.skip_whitespace();

        let ch = match self.peek() {
            Some(c) => c,
            None => return Ok(Token::Eof),
        };

        // Symbols
        match ch {
            ',' => {
                self.advance();
                return Ok(Token::Comma);
            }
            '=' => {
                self.advance();
                return Ok(Token::Equal);
            }
            ';' => {
                self.advance();
                return Ok(Token::Semicolon);
            }
            '(' => {
                self.advance();
                return Ok(Token::LParen);
            }
            ')' => {
                self.advance();
                return Ok(Token::RParen);
            }
            '[' => {
                self.advance();
                return Ok(Token::LBracket);
            }
            ']' => {
                self.advance();
                return Ok(Token::RBracket);
            }
            '"' | '\'' => {
                let start_line = self.line;
                let start_col = self.col;
                let quote = self.advance().unwrap();
                let mut s = String::new();
                while let Some(c) = self.advance() {
                    if c == quote {
                        return Ok(Token::StringLiteral(s));
                    }
                    s.push(c);
                }
                return Err(QlError::Parse {
                    message: "Unterminated string literal".to_string(),
                    line: start_line,
                    col: start_col,
                });
            }
            _ => {}
        }

        // Numbers (including negative numbers and floats)
        if ch.is_ascii_digit()
            || (ch == '-'
                && self.pos + 1 < self.chars.len()
                && self.chars[self.pos + 1].is_ascii_digit())
        {
            return self.lex_number();
        }

        // Identifiers, keywords, or coordinate addresses
        if ch.is_ascii_alphabetic() || ch == '_' {
            return self.lex_ident_or_keyword();
        }

        Err(QlError::Parse {
            message: format!("Unexpected character: '{}'", ch),
            line: self.line,
            col: self.col,
        })
    }

    fn lex_number(&mut self) -> Result<Token, QlError> {
        let start_line = self.line;
        let start_col = self.col;
        let mut s = String::new();
        if self.peek() == Some('-') {
            s.push(self.advance().unwrap());
        }

        let mut seen_dot = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                s.push(self.advance().unwrap());
            } else if c == '.' && !seen_dot {
                seen_dot = true;
                s.push(self.advance().unwrap());
            } else if c == '_' {
                // Ignore underscores inside numbers (e.g. 1_000_000 or 10_us)
                self.advance();
                // Check if followed by unit like 'us' or 's'
                while let Some(unit_ch) = self.peek() {
                    if unit_ch.is_ascii_alphabetic() {
                        self.advance();
                    } else {
                        break;
                    }
                }
            } else {
                break;
            }
        }

        match s.parse::<f64>() {
            Ok(num) => Ok(Token::Number(num)),
            Err(_) => Err(QlError::Parse {
                message: format!("Failed to parse number: '{}'", s),
                line: start_line,
                col: start_col,
            }),
        }
    }

    fn lex_ident_or_keyword(&mut self) -> Result<Token, QlError> {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '_' || c == ':' {
                s.push(self.advance().unwrap());
            } else {
                break;
            }
        }

        // Check if coordinate format L<level>:T<tile>:R<offset>
        if s.starts_with('L') && s.contains(":T") && s.contains(":R") {
            return Ok(Token::Coordinate(s));
        }

        let upper = s.to_ascii_uppercase();
        let token = match upper.as_str() {
            "VERSION" => Token::Version,
            "PRAGMA" => Token::Pragma,
            "FETCH" => Token::Fetch,
            "RECORD" => Token::Record,
            "AT" => Token::At,
            "FROM" => Token::From,
            "SIFT" => Token::Sift,
            "WHERE" => Token::Where,
            "AND" => Token::And,
            "BETWEEN" => Token::Between,
            "LIMIT" => Token::Limit,
            "INSERT" => Token::Insert,
            "INTO" => Token::Into,
            "DELETE" => Token::Delete,
            "COMPACT" => Token::Compact,
            "ARCHIVE" => Token::Archive,
            "FUSE" => Token::Fuse,
            "TILES" => Token::Tiles,
            "WITH" => Token::With,
            "TRUE" => Token::True,
            "FALSE" => Token::False,

            "TELEMETRY" => Token::Telemetry,
            "KINEMATICS" => Token::Kinematics,
            "GIS_MESH" => Token::GisMesh,
            "EMBEDDINGS" => Token::Embeddings,
            "DA_COMMITMENTS" => Token::DaCommitments,
            "LEARNING_ACTIVITIES" | "ACTIVITIES" | "LRS" => Token::LearningActivities,
            "HUMAN_LRS" => Token::HumanLrs,
            "AI_TRACES" | "AI_TRACE_LOG" => Token::AiTraces,
            "ACADEMIC_PAPERS" | "PAPERS" => Token::AcademicPapers,

            "TIME" => Token::Time,
            "SENSOR_ID" => Token::SensorId,
            "METRIC_ID" => Token::MetricId,
            "ACTOR_ID" => Token::ActorId,
            "VERB_ID" => Token::VerbId,
            "SPATIAL_BOX" => Token::SpatialBox,
            "IN" => Token::In,
            "SIMPLEX" => Token::Simplex,
            "MIN" => Token::Min,
            "MAX" => Token::Max,
            "LATITUDE" => Token::Latitude,
            "LONGITUDE" => Token::Longitude,
            "ELEVATION" => Token::Elevation,
            "SIMILARITY" => Token::Similarity,
            "TO" => Token::To,
            "USING" => Token::Using,
            "COSINE" => Token::Cosine,
            "DOT_PRODUCT" => Token::DotProduct,
            "THRESHOLD" => Token::Threshold,
            "BLOCK_HEIGHT" => Token::BlockHeight,
            "COMPOSE" => Token::Compose,
            "WEIGHT" => Token::Weight,
            "SPATIAL_PROXIMITY" => Token::SpatialProximity,
            "DECAY" => Token::Decay,
            "HALFLIFE" => Token::Halflife,

            _ => Token::Identifier(s),
        };

        Ok(token)
    }

    pub fn tokenize_all(&mut self) -> Result<Vec<Token>, QlError> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token()?;
            if tok == Token::Eof {
                tokens.push(Token::Eof);
                break;
            }
            tokens.push(tok);
        }
        Ok(tokens)
    }
}
