use crate::ql::ast::*;
use crate::ql::lexer::Token;
use s3a_core::S3ACoordinate;

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn peek_ahead(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset)
    }

    fn advance(&mut self) -> &Token {
        let tok = self.tokens.get(self.pos).unwrap_or(&Token::Eof);
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, expected: Token) -> Result<(), String> {
        let current = self.peek().clone();
        if current == expected {
            self.advance();
            Ok(())
        } else {
            Err(format!("Expected token {:?}, found {:?}", expected, current))
        }
    }

    fn parse_file_path(&mut self) -> Result<String, String> {
        match self.peek().clone() {
            Token::StringLiteral(s) => {
                self.advance();
                Ok(s)
            }
            Token::Identifier(s) => {
                self.advance();
                Ok(s)
            }
            other => Err(format!("Expected file path string or identifier, found {:?}", other)),
        }
    }

    fn parse_u64(&mut self) -> Result<u64, String> {
        match self.peek().clone() {
            Token::Number(n) => {
                self.advance();
                Ok(n as u64)
            }
            other => Err(format!("Expected unsigned integer number, found {:?}", other)),
        }
    }

    fn parse_f64(&mut self) -> Result<f64, String> {
        match self.peek().clone() {
            Token::Number(n) => {
                self.advance();
                Ok(n)
            }
            other => Err(format!("Expected number, found {:?}", other)),
        }
    }

    fn parse_vector_3d(&mut self) -> Result<[f32; 3], String> {
        self.expect(Token::LBracket)?;
        let x = self.parse_f64()? as f32;
        self.expect(Token::Comma)?;
        let y = self.parse_f64()? as f32;
        self.expect(Token::Comma)?;
        let z = self.parse_f64()? as f32;
        self.expect(Token::RBracket)?;
        Ok([x, y, z])
    }

    fn parse_float_vector(&mut self) -> Result<Vec<f32>, String> {
        self.expect(Token::LBracket)?;
        let mut vec = Vec::new();
        loop {
            if self.peek() == &Token::RBracket {
                self.advance();
                break;
            }
            let val = self.parse_f64()? as f32;
            vec.push(val);
            if self.peek() == &Token::Comma {
                self.advance();
            } else if self.peek() == &Token::RBracket {
                self.advance();
                break;
            } else {
                return Err(format!("Expected ',' or ']' in vector, found {:?}", self.peek()));
            }
        }
        Ok(vec)
    }

    pub fn parse_statement(&mut self) -> Result<S3AStatement, String> {
        let stmt = match self.peek() {
            Token::Fetch => self.parse_fetch()?,
            Token::Sift => self.parse_sift()?,
            Token::Insert => self.parse_insert()?,
            Token::Delete => self.parse_delete()?,
            Token::Compact => self.parse_compact()?,
            Token::Fuse => self.parse_fuse()?,
            other => return Err(format!("Unexpected query start token: {:?}", other)),
        };

        // Consume optional trailing semicolon
        if self.peek() == &Token::Semicolon {
            self.advance();
        }

        Ok(stmt)
    }

    fn parse_fetch(&mut self) -> Result<S3AStatement, String> {
        self.expect(Token::Fetch)?;
        self.expect(Token::Record)?;
        self.expect(Token::At)?;

        let coord_str = match self.advance() {
            Token::Coordinate(s) => s.clone(),
            Token::Identifier(s) => s.clone(),
            other => return Err(format!("Expected S3A coordinate (e.g. L0:T4:R12), found {:?}", other)),
        };

        let coordinate: S3ACoordinate = coord_str.parse().map_err(|e| format!("Invalid coordinate format: {}", e))?;

        self.expect(Token::From)?;
        let from_path = self.parse_file_path()?;

        Ok(S3AStatement::Fetch(FetchStatement { coordinate, from_path }))
    }

    fn parse_sift(&mut self) -> Result<S3AStatement, String> {
        self.expect(Token::Sift)?;

        let domain = match self.advance() {
            Token::Telemetry => SiftDomain::Telemetry,
            Token::Kinematics => SiftDomain::Kinematics,
            Token::GisMesh => SiftDomain::GisMesh,
            Token::Embeddings => SiftDomain::Embeddings,
            Token::DaCommitments => SiftDomain::DaCommitments,
            Token::LearningActivities => SiftDomain::LearningActivities,
            other => return Err(format!("Expected SIFT domain, found {:?}", other)),
        };

        self.expect(Token::From)?;
        let from_path = self.parse_file_path()?;

        let mut conditions = Vec::new();
        if self.peek() == &Token::Where {
            self.advance();
            loop {
                let cond = self.parse_condition()?;
                conditions.push(cond);
                if self.peek() == &Token::And {
                    self.advance();
                } else if self.peek() == &Token::Compose {
                    let comp_cond = self.parse_condition()?;
                    conditions.push(comp_cond);
                    break;
                } else {
                    break;
                }
            }
        }

        if self.peek() == &Token::Compose {
            let comp_cond = self.parse_condition()?;
            conditions.push(comp_cond);
        }

        let mut limit = None;
        if self.peek() == &Token::Limit {
            self.advance();
            limit = Some(self.parse_u64()? as usize);
        }

        Ok(S3AStatement::Sift(SiftStatement {
            domain,
            from_path,
            conditions,
            limit,
        }))
    }

    fn parse_condition(&mut self) -> Result<Condition, String> {
        match self.peek() {
            Token::Time => {
                self.advance();
                self.expect(Token::Between)?;
                let min = self.parse_u64()?;
                self.expect(Token::And)?;
                let max = self.parse_u64()?;
                Ok(Condition::TimeBetween(min, max))
            }
            Token::SensorId => {
                self.advance();
                self.expect(Token::Equal)?;
                let id = self.parse_u64()? as u32;
                Ok(Condition::SensorId(id))
            }
            Token::MetricId => {
                self.advance();
                self.expect(Token::Equal)?;
                let id = self.parse_u64()? as u32;
                Ok(Condition::MetricId(id))
            }
            Token::ActorId => {
                self.advance();
                self.expect(Token::Equal)?;
                let id = self.parse_u64()?;
                Ok(Condition::ActorId(id))
            }
            Token::VerbId => {
                self.advance();
                self.expect(Token::Equal)?;
                let id = self.parse_u64()? as u32;
                Ok(Condition::VerbId(id))
            }
            Token::BlockHeight => {
                self.advance();
                self.expect(Token::Between)?;
                let min = self.parse_u64()?;
                self.expect(Token::And)?;
                let max = self.parse_u64()?;
                Ok(Condition::BlockHeightBetween(min, max))
            }
            Token::Latitude => {
                self.advance();
                self.expect(Token::Between)?;
                let min = self.parse_f64()?;
                self.expect(Token::And)?;
                let max = self.parse_f64()?;
                Ok(Condition::GisLatitudeBetween(min, max))
            }
            Token::Longitude => {
                self.advance();
                self.expect(Token::Between)?;
                let min = self.parse_f64()?;
                self.expect(Token::And)?;
                let max = self.parse_f64()?;
                Ok(Condition::GisLongitudeBetween(min, max))
            }
            Token::Elevation => {
                self.advance();
                self.expect(Token::Between)?;
                let min = self.parse_f64()?;
                self.expect(Token::And)?;
                let max = self.parse_f64()?;
                Ok(Condition::GisElevationBetween(min, max))
            }
            Token::SpatialBox => {
                self.advance();
                self.expect(Token::In)?;
                self.expect(Token::Simplex)?;
                self.expect(Token::LParen)?;
                self.expect(Token::Min)?;
                let min = self.parse_vector_3d()?;
                self.expect(Token::Comma)?;
                self.expect(Token::Max)?;
                let max = self.parse_vector_3d()?;
                self.expect(Token::RParen)?;
                Ok(Condition::SpatialBoxSimplex { min, max })
            }
            Token::Similarity => {
                self.advance();
                self.expect(Token::To)?;
                let vector = self.parse_float_vector()?;
                self.expect(Token::Using)?;
                let metric = match self.advance() {
                    Token::Cosine => SimilarityMetric::Cosine,
                    Token::DotProduct => SimilarityMetric::DotProduct,
                    other => return Err(format!("Expected COSINE or DOT_PRODUCT, found {:?}", other)),
                };
                self.expect(Token::Threshold)?;
                let threshold = self.parse_f64()? as f32;
                Ok(Condition::Similarity { vector, metric, threshold })
            }
            Token::Compose => {
                self.advance();
                self.expect(Token::Similarity)?;
                self.expect(Token::To)?;
                let vector = self.parse_float_vector()?;
                self.expect(Token::Weight)?;
                let vector_weight = self.parse_f64()? as f32;

                let mut spatial_target = None;
                let mut spatial_weight = 0.0f32;
                let mut halflife_secs = None;
                let mut time_weight = 0.0f32;

                while self.peek() == &Token::And {
                    let next_tok = self.peek_ahead(1);
                    if next_tok == Some(&Token::SpatialProximity) {
                        self.advance(); // AND
                        self.advance(); // SPATIAL_PROXIMITY
                        let pt = self.parse_vector_3d()?;
                        self.expect(Token::Weight)?;
                        let w = self.parse_f64()? as f32;
                        spatial_target = Some(pt);
                        spatial_weight = w;
                    } else if next_tok == Some(&Token::Decay) {
                        self.advance(); // AND
                        self.advance(); // DECAY
                        self.expect(Token::Halflife)?;
                        let hl = self.parse_f64()?;
                        self.expect(Token::Weight)?;
                        let w = self.parse_f64()? as f32;
                        halflife_secs = Some(hl);
                        time_weight = w;
                    } else {
                        break;
                    }
                }

                Ok(Condition::AlgebraicCompose {
                    vector,
                    vector_weight,
                    spatial_target,
                    spatial_weight,
                    halflife_secs,
                    time_weight,
                })
            }
            other => Err(format!("Unknown or unsupported WHERE condition: {:?}", other)),
        }
    }

    fn parse_insert(&mut self) -> Result<S3AStatement, String> {
        self.expect(Token::Insert)?;
        self.expect(Token::Telemetry)?;
        self.expect(Token::LParen)?;

        let timestamp = self.parse_u64()?;
        self.expect(Token::Comma)?;
        let sensor_id = self.parse_u64()? as u32;
        self.expect(Token::Comma)?;
        let metric_id = self.parse_u64()? as u32;
        self.expect(Token::Comma)?;
        let value = self.parse_f64()?;
        self.expect(Token::RParen)?;

        self.expect(Token::Into)?;
        let into_path = self.parse_file_path()?;

        Ok(S3AStatement::Insert(InsertStatement {
            timestamp,
            sensor_id,
            metric_id,
            value,
            into_path,
        }))
    }

    fn parse_delete(&mut self) -> Result<S3AStatement, String> {
        self.expect(Token::Delete)?;
        self.expect(Token::Telemetry)?;
        self.expect(Token::Where)?;

        let mut sensor_id = None;
        let mut metric_id = None;
        let mut timestamp = None;

        loop {
            match self.peek() {
                Token::SensorId => {
                    self.advance();
                    self.expect(Token::Equal)?;
                    sensor_id = Some(self.parse_u64()? as u32);
                }
                Token::MetricId => {
                    self.advance();
                    self.expect(Token::Equal)?;
                    metric_id = Some(self.parse_u64()? as u32);
                }
                Token::Time => {
                    self.advance();
                    self.expect(Token::Equal)?;
                    timestamp = Some(self.parse_u64()?);
                }
                other => return Err(format!("Unexpected field in DELETE WHERE clause: {:?}", other)),
            }

            if self.peek() == &Token::And {
                self.advance();
            } else {
                break;
            }
        }

        let sensor_id = sensor_id.ok_or_else(|| "Missing SENSOR_ID in DELETE clause".to_string())?;
        let metric_id = metric_id.ok_or_else(|| "Missing METRIC_ID in DELETE clause".to_string())?;
        let timestamp = timestamp.ok_or_else(|| "Missing TIME in DELETE clause".to_string())?;

        self.expect(Token::From)?;
        let from_path = self.parse_file_path()?;

        Ok(S3AStatement::Delete(DeleteStatement {
            sensor_id,
            metric_id,
            timestamp,
            from_path,
        }))
    }

    fn parse_compact(&mut self) -> Result<S3AStatement, String> {
        self.expect(Token::Compact)?;
        self.expect(Token::Archive)?;
        let archive_path = self.parse_file_path()?;

        Ok(S3AStatement::Compact(CompactStatement { archive_path }))
    }

    fn parse_fuse(&mut self) -> Result<S3AStatement, String> {
        self.expect(Token::Fuse)?;
        self.expect(Token::Tiles)?;
        self.expect(Token::From)?;
        self.expect(Token::LParen)?;

        let mut input_paths = Vec::new();
        loop {
            let path = self.parse_file_path()?;
            input_paths.push(path);
            if self.peek() == &Token::Comma {
                self.advance();
            } else if self.peek() == &Token::RParen {
                self.advance();
                break;
            } else {
                return Err(format!("Expected ',' or ')' in FROM list, found {:?}", self.peek()));
            }
        }

        self.expect(Token::Into)?;
        let output_path = self.parse_file_path()?;

        let mut resolve_versioned_updates = true;
        let mut purge_tombstones = true;
        let mut recalculate_simplex_hulls = true;

        if self.peek() == &Token::With {
            self.advance();
            self.expect(Token::LParen)?;
            while self.peek() != &Token::RParen && self.peek() != &Token::Eof {
                if let Token::Identifier(opt) = self.peek().clone() {
                    self.advance();
                    self.expect(Token::Equal)?;
                    let val = match self.advance() {
                        Token::True => true,
                        Token::False => false,
                        other => return Err(format!("Expected TRUE or FALSE, found {:?}", other)),
                    };
                    match opt.to_ascii_uppercase().as_str() {
                        "RESOLVE_VERSIONED_UPDATES" => resolve_versioned_updates = val,
                        "PURGE_TOMBSTONES" => purge_tombstones = val,
                        "RECALCULATE_SIMPLEX_HULLS" => recalculate_simplex_hulls = val,
                        _ => {}
                    }
                    if self.peek() == &Token::Comma {
                        self.advance();
                    }
                } else {
                    break;
                }
            }
            self.expect(Token::RParen)?;
        }

        Ok(S3AStatement::Fuse(FuseStatement {
            input_paths,
            output_path,
            resolve_versioned_updates,
            purge_tombstones,
            recalculate_simplex_hulls,
        }))
    }
}
