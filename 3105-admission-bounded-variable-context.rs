use std::cmp::Ordering;
use std::fmt;

/// A [SPARQL query](https://www.w3.org/TR/sparql11-query/) owned variable.
///
/// The default string formatter is returning a SPARQL compatible representation:
/// ```
/// use oxrdf::{Variable, VariableNameParseError};
///
/// assert_eq!("?foo", Variable::new("foo")?.to_string());
/// # Result::<_,VariableNameParseError>::Ok(())
/// ```
#[derive(Eq, PartialEq, Ord, PartialOrd, Debug, Clone, Hash)]
pub struct Variable {
    name: String,
}

impl Variable {
    /// Creates a variable name from a unique identifier.
    ///
    /// The variable identifier must be valid according to the SPARQL grammar.
    pub fn new(name: impl Into<String>) -> Result<Self, VariableNameParseError> {
        let name = name.into();
        validate_variable_identifier(&name)?;
        Ok(Self::new_unchecked(name))
    }

    /// Creates a variable name from a unique identifier without validation.
    ///
    /// It is the caller's responsibility to ensure that `id` is a valid blank node identifier
    /// according to the SPARQL grammar.
    ///
    /// [`Variable::new()`] is a safe version of this constructor and should be used for untrusted data.
    #[inline]
    pub fn new_unchecked(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        &self.name
    }

    #[inline]
    pub fn into_string(self) -> String {
        self.name
    }

    #[inline]
    pub fn as_ref(&self) -> VariableRef<'_> {
