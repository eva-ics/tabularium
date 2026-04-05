//! Listing and metadata types — accessors only, no public fields.

use bma_ts::Timestamp;

use super::EntryId;
use super::entry_kind::EntryKind;

/// One row from `document_grep` (1-based line index).
#[derive(Debug, Clone)]
pub struct GrepLine {
    line: usize,
    text: String,
}

impl GrepLine {
    pub(crate) fn new(line: usize, text: String) -> Self {
        Self { line, text }
    }

    pub fn line(&self) -> usize {
        self.line
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

/// Word/line/byte/char counts for `wc`-style RPC.
#[derive(Debug, Clone)]
pub struct WcStats {
    bytes: u64,
    lines: usize,
    words: usize,
    chars: usize,
}

impl WcStats {
    pub(crate) fn from_content(content: &str) -> Self {
        Self {
            bytes: content.len() as u64,
            lines: content.lines().count(),
            words: content.split_whitespace().count(),
            chars: content.chars().count(),
        }
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn lines(&self) -> usize {
        self.lines
    }

    pub fn words(&self) -> usize {
        self.words
    }

    pub fn chars(&self) -> usize {
        self.chars
    }
}

/// Search result for REST/RPC (snippet is a short excerpt around the query).
#[derive(Debug, Clone)]
pub struct SearchHit {
    document_id: EntryId,
    path: String,
    snippet: String,
    score: f32,
    line_number: Option<usize>,
}

impl SearchHit {
    pub(crate) fn new(
        document_id: EntryId,
        path: String,
        snippet: String,
        score: f32,
        line_number: Option<usize>,
    ) -> Self {
        Self {
            document_id,
            path,
            snippet,
            score,
            line_number,
        }
    }

    pub fn document_id(&self) -> EntryId {
        self.document_id
    }

    /// Full canonical path to the file (e.g. `/notes/readme.md`).
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn snippet(&self) -> &str {
        &self.snippet
    }

    pub fn score(&self) -> f32 {
        self.score
    }

    /// 1-based line of the first query-token match in the body, when resolved.
    pub fn line_number(&self) -> Option<usize> {
        self.line_number
    }
}

/// One child entry listed under a directory.
#[derive(Debug, Clone)]
pub struct ListedEntry {
    id: EntryId,
    kind: EntryKind,
    name: String,
    description: Option<String>,
    created_at: Timestamp,
    modified_at: Timestamp,
    accessed_at: Timestamp,
    size_bytes: Option<i64>,
    /// Recursive file count under this directory; `0` for files.
    recursive_file_count: u64,
}

impl ListedEntry {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: EntryId,
        kind: EntryKind,
        name: String,
        description: Option<String>,
        created_at: Timestamp,
        modified_at: Timestamp,
        accessed_at: Timestamp,
        size_bytes: Option<i64>,
        recursive_file_count: u64,
    ) -> Self {
        Self {
            id,
            kind,
            name,
            description,
            created_at,
            modified_at,
            accessed_at,
            size_bytes,
            recursive_file_count,
        }
    }

    pub fn id(&self) -> EntryId {
        self.id
    }

    pub fn kind(&self) -> EntryKind {
        self.kind
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub fn modified_at(&self) -> Timestamp {
        self.modified_at
    }

    pub fn accessed_at(&self) -> Timestamp {
        self.accessed_at
    }

    pub fn size_bytes(&self) -> Option<i64> {
        self.size_bytes
    }

    pub fn recursive_file_count(&self) -> u64 {
        self.recursive_file_count
    }
}

/// File entry metadata without body text.
#[derive(Debug, Clone)]
pub struct DocumentMeta {
    id: EntryId,
    parent_id: EntryId,
    name: String,
    canonical_path: String,
    created_at: Timestamp,
    modified_at: Timestamp,
    accessed_at: Timestamp,
    size_bytes: i64,
}

impl DocumentMeta {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: EntryId,
        parent_id: EntryId,
        name: String,
        canonical_path: String,
        created_at: Timestamp,
        modified_at: Timestamp,
        accessed_at: Timestamp,
        size_bytes: i64,
    ) -> Self {
        Self {
            id,
            parent_id,
            name,
            canonical_path,
            created_at,
            modified_at,
            accessed_at,
            size_bytes,
        }
    }

    pub fn id(&self) -> EntryId {
        self.id
    }

    pub fn parent_id(&self) -> EntryId {
        self.parent_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn canonical_path(&self) -> &str {
        &self.canonical_path
    }

    pub fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub fn modified_at(&self) -> Timestamp {
        self.modified_at
    }

    pub fn accessed_at(&self) -> Timestamp {
        self.accessed_at
    }

    pub fn size_bytes(&self) -> i64 {
        self.size_bytes
    }
}
