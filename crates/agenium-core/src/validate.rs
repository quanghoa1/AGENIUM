//! Field-level validation for forms. Errors go back to the model as a structured tool result (decision 36), so
//! every message names the field and says what is expected.

use std::fmt;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct FormErrors(pub Vec<FieldError>);

impl FormErrors {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for FormErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{}: {}", e.field, e.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for FormErrors {}

/// Something that can be checked beyond what serde already enforces.
pub trait Validate {
    fn validate(&self) -> Result<(), FormErrors>;
}

/// Collects field errors; `finish` turns them into a result.
#[derive(Default)]
pub struct Checker {
    errors: Vec<FieldError>,
}

impl Checker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fail(&mut self, field: impl Into<String>, message: impl Into<String>) {
        self.errors.push(FieldError {
            field: field.into(),
            message: message.into(),
        });
    }

    pub fn check(&mut self, ok: bool, field: impl Into<String>, message: impl Into<String>) {
        if !ok {
            self.fail(field, message);
        }
    }

    /// Character count (not bytes) within `min..=max`.
    pub fn len(&mut self, field: &str, value: &str, min: usize, max: usize) {
        let n = value.chars().count();
        if n < min {
            if min == 1 {
                self.fail(field, "must not be empty");
            } else {
                self.fail(
                    field,
                    format!("must have at least {min} characters (got {n})"),
                );
            }
        } else if n > max {
            self.fail(
                field,
                format!("must have at most {max} characters (got {n})"),
            );
        }
    }

    pub fn non_blank(&mut self, field: &str, value: &str) {
        self.check(!value.trim().is_empty(), field, "must not be blank");
    }

    pub fn items<T>(&mut self, field: &str, v: &[T], min: usize, max: usize) {
        if v.len() < min {
            self.fail(field, format!("needs at least {min} item(s)"));
        } else if v.len() > max {
            self.fail(
                field,
                format!("allows at most {max} items (got {})", v.len()),
            );
        }
    }

    pub fn finish(self) -> Result<(), FormErrors> {
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(FormErrors(self.errors))
        }
    }
}
