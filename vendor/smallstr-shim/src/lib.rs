//! The owned UTF-8 string interface used by the native Yjs dependency.
//! This implements only that consumer's surface, using the standard library throughout.
#![forbid(unsafe_code)]

use std::{fmt, marker::PhantomData, ops::Deref};

/// An owned string with the upstream consumer's array parameter retained in its type.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SmallString<A> {
    value: String,
    array: PhantomData<A>,
}

impl<A> SmallString<A> {
    /// Copy a UTF-8 string, preserving the consumer's constructor signature.
    #[allow(clippy::should_implement_trait)] // The dependency requires an infallible inherent constructor.
    pub fn from_str(value: &str) -> Self {
        Self {
            value: value.into(),
            array: PhantomData,
        }
    }

    pub fn as_str(&self) -> &str {
        &self.value
    }

    pub fn push_str(&mut self, suffix: &str) {
        self.value.push_str(suffix);
    }

    pub fn into_string(self) -> String {
        self.value
    }
}

impl<A> From<&str> for SmallString<A> {
    fn from(value: &str) -> Self {
        Self::from_str(value)
    }
}

impl<A> Deref for SmallString<A> {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl<A> fmt::Display for SmallString<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.value, f)
    }
}
