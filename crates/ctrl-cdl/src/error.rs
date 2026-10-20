use std::fmt;

/// One problem in a descriptor, located at file, line and column (1-based).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.file, self.line, self.col, self.message
        )
    }
}

impl std::error::Error for Error {}

/// Every problem found in one load, in source order per file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Errors(pub Vec<Error>);

impl fmt::Display for Errors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, error) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Errors {}
