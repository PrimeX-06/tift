//! Internal shared structural names; public Block remains an owned String record.
use crate::syntax::{Block, BlockKind};
use std::borrow::Borrow;
use std::collections::HashSet;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Deref, Range};
use std::rc::Rc;

#[derive(Clone, Default)]
pub(crate) struct SharedName(Option<Rc<str>>);

impl SharedName {
    pub(crate) fn as_str(&self) -> &str {
        self.0.as_deref().unwrap_or("")
    }
}
impl Deref for SharedName {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}
impl AsRef<str> for SharedName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
impl Borrow<str> for SharedName {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}
impl PartialEq for SharedName {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}
impl Eq for SharedName {}
impl Hash for SharedName {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}
impl PartialEq<&str> for SharedName {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}
impl PartialEq<String> for SharedName {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}
impl fmt::Debug for SharedName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}
impl fmt::Display for SharedName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
impl From<&str> for SharedName {
    fn from(name: &str) -> Self {
        if name.is_empty() {
            Self::default()
        } else {
            Self(Some(Rc::from(name)))
        }
    }
}
impl From<String> for SharedName {
    fn from(name: String) -> Self {
        Self::from(name.as_str())
    }
}

#[derive(Default)]
pub(crate) struct NamePool {
    names: HashSet<SharedName>,
}
impl NamePool {
    pub(crate) fn intern(&mut self, name: &str) -> SharedName {
        if name.is_empty() {
            return SharedName::default();
        }
        if let Some(shared) = self.names.get(name) {
            return shared.clone();
        }
        let shared = SharedName::from(name);
        self.names.insert(shared.clone());
        shared
    }
}

#[derive(Clone)]
pub(crate) struct StructuralBlock<N> {
    pub(crate) kind: BlockKind,
    pub(crate) name: N,
    pub(crate) begin: Range<usize>,
    pub(crate) body: Range<usize>,
    pub(crate) end: Range<usize>,
    pub(crate) parent: Option<usize>,
    pub(crate) branches: Vec<(bool, Range<usize>)>,
}
pub(crate) type SharedBlock = StructuralBlock<SharedName>;
impl<N: fmt::Debug> fmt::Debug for StructuralBlock<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Block")
            .field("kind", &self.kind)
            .field("name", &self.name)
            .field("begin", &self.begin)
            .field("body", &self.body)
            .field("end", &self.end)
            .field("parent", &self.parent)
            .field("branches", &self.branches)
            .finish()
    }
}
impl From<SharedBlock> for Block {
    fn from(block: SharedBlock) -> Self {
        Self {
            kind: block.kind,
            name: block.name.as_str().to_owned(),
            begin: block.begin,
            body: block.body,
            end: block.end,
            parent: block.parent,
            branches: block.branches,
        }
    }
}

pub(crate) trait BlockName:
    Clone
    + Default
    + AsRef<str>
    + Deref<Target = str>
    + fmt::Debug
    + fmt::Display
    + for<'a> PartialEq<&'a str>
    + for<'a> From<&'a str>
{
    fn as_str(&self) -> &str {
        self.as_ref()
    }
}
impl BlockName for String {}
impl BlockName for SharedName {}

pub(crate) trait NameStore {
    type Name: BlockName;
    fn make(&mut self, name: &str) -> Self::Name;
}
pub(crate) struct OwnedNames;
impl NameStore for OwnedNames {
    type Name = String;
    fn make(&mut self, name: &str) -> String {
        name.to_owned()
    }
}
impl NameStore for NamePool {
    type Name = SharedName;
    fn make(&mut self, name: &str) -> SharedName {
        self.intern(name)
    }
}
impl From<StructuralBlock<String>> for Block {
    fn from(block: StructuralBlock<String>) -> Self {
        Self {
            kind: block.kind,
            name: block.name,
            begin: block.begin,
            body: block.body,
            end: block.end,
            parent: block.parent,
            branches: block.branches,
        }
    }
}


