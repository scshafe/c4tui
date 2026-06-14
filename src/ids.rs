use std::borrow::Borrow;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ViewId(usize);

impl ViewId {
    pub(crate) const fn new(index: usize) -> Self {
        Self(index)
    }

    pub(crate) const fn first() -> Self {
        Self(0)
    }

    pub(crate) const fn index(self) -> usize {
        self.0
    }
}

impl From<usize> for ViewId {
    fn from(index: usize) -> Self {
        Self::new(index)
    }
}

impl fmt::Display for ViewId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ElementId(String);

impl ElementId {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ElementId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for ElementId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for ElementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Borrow<str> for ElementId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct RelationshipId(String);

impl RelationshipId {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for RelationshipId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for RelationshipId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for RelationshipId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Borrow<str> for RelationshipId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}
