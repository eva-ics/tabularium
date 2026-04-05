//! Higher-level document operations (resolve keys, text slices, search hits).

use std::collections::HashMap;

use regex::Regex;
use tracing::instrument;

use crate::db::entry_kind::EntryKind;
use crate::db::meta::{DocumentMeta, GrepLine, SearchHit, WcStats};
use crate::db::{Database, EntryId, Storage};
use crate::resource_path::{canonical_path_segments, parent_and_final_name};
use crate::validation::{
    escape_chat_heading_label, validate_chat_speaker_id, validate_entity_name,
};
use crate::{Error, Result};

impl<S: Storage> Database<S> {
    /// Note: `NotFound` is normal when probing file-vs-directory (REST `get_or_list`); no `err(Debug)` to avoid ERROR spam.
    #[instrument(skip(self), fields(file_path = %file_path.as_ref()))]
    pub async fn resolve_file_path(&self, file_path: impl AsRef<str> + Send) -> Result<EntryId> {
        let p = file_path.as_ref();
        self.storage.resolve_path(p, Some(EntryKind::File)).await
    }

    #[instrument(skip(self), fields(dir_path = %dir_path.as_ref()), err(Debug))]
    pub async fn resolve_directory_path(
        &self,
        dir_path: impl AsRef<str> + Send,
    ) -> Result<EntryId> {
        self.storage
            .resolve_path(dir_path.as_ref(), Some(EntryKind::Dir))
            .await
    }

    #[instrument(skip(self), fields(file_id = file_id.raw()), err(Debug))]
    pub async fn get_document_meta(&self, file_id: EntryId) -> Result<DocumentMeta> {
        self.storage.get_file_meta(file_id).await
    }

    #[instrument(skip(self), fields(file_id = file_id.raw()), err(Debug))]
    pub async fn cat_document_bundle(&self, file_id: EntryId) -> Result<(DocumentMeta, String)> {
        let meta = self.storage.get_file_meta(file_id).await?;
        let body = self.get_document(file_id).await?;
        Ok((meta, body))
    }

    #[instrument(skip(self, path), err(Debug))]
    pub async fn document_ref_by_path(&self, path: impl AsRef<str> + Send) -> Result<DocumentMeta> {
        let id = self.resolve_file_path(path.as_ref()).await?;
        self.storage.get_file_meta(id).await
    }

    /// Append to an existing file, or create it (and parent directories) if absent.
    #[instrument(skip(self, path, to_append), err(Debug))]
    pub async fn append_document_by_path(
        &self,
        path: impl AsRef<str> + Send,
        to_append: impl AsRef<str> + Send,
    ) -> Result<()> {
        let path = path.as_ref();
        let (parent, name) = parent_and_final_name(path)?;
        validate_entity_name(&name)?;
        self.storage.ensure_directory_path(&parent).await?;
        let piece = to_append.as_ref();
        match self.resolve_file_path(path).await {
            Ok(fid) => self.append_document(fid, piece).await,
            Err(Error::NotFound(_)) => {
                let id = self.storage.create_file(path, piece).await?;
                self.reindex_file_in_search(id).await?;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    #[instrument(skip(self, path, from_id, text), err(Debug))]
    pub async fn say_document_by_path(
        &self,
        path: impl AsRef<str> + Send,
        from_id: impl AsRef<str> + Send,
        text: impl AsRef<str> + Send,
    ) -> Result<()> {
        let path = path.as_ref();
        let from_id = from_id.as_ref();
        validate_chat_speaker_id(from_id)?;
        let body = text.as_ref().trim_end_matches(['\r', '\n']);
        let label = escape_chat_heading_label(from_id);
        let fid = match self.resolve_file_path(path).await {
            Ok(id) => id,
            Err(Error::NotFound(_)) => {
                return Err(Error::InvalidInput(format!(
                    "say_document: document does not exist (create it with append_document or put_document first): {path}"
                )));
            }
            Err(e) => return Err(e),
        };
        let current = self.storage.get_file_content(fid).await?;
        let sep = if current.is_empty() || current.ends_with("\n\n") {
            ""
        } else if current.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        let piece = format!("{sep}## {label}\n\n{body}\n\n");
        self.append_document(fid, &piece).await
    }

    #[instrument(
        skip(self),
        fields(file_id = file_id.raw(), lines),
        err(Debug)
    )]
    pub async fn document_head(&self, file_id: EntryId, lines: u32) -> Result<String> {
        let content = self.get_document(file_id).await?;
        Ok(crate::text_lines::head_logical_lines(&content, lines))
    }

    #[instrument(
        skip(self),
        fields(file_id = file_id.raw(), tail = ?mode),
        err(Debug)
    )]
    pub async fn document_tail(
        &self,
        file_id: EntryId,
        mode: crate::text_lines::TailMode,
    ) -> Result<String> {
        let content = self.get_document(file_id).await?;
        Ok(crate::text_lines::apply_tail_logical_lines(&content, mode))
    }

    #[instrument(
        skip(self),
        fields(file_id = file_id.raw(), start_line, end_line),
        err(Debug)
    )]
    pub async fn document_slice(
        &self,
        file_id: EntryId,
        start_line: u32,
        end_line: u32,
    ) -> Result<String> {
        if start_line == 0 || end_line == 0 || start_line > end_line {
            return Err(Error::InvalidInput("line range invalid".into()));
        }
        let content = self.get_document(file_id).await?;
        Ok(slice_lines(
            &content,
            start_line as usize,
            end_line as usize,
        ))
    }

    #[instrument(skip(self), fields(file_id = file_id.raw()), err(Debug))]
    pub async fn document_wc(&self, file_id: EntryId) -> Result<WcStats> {
        let content = self.get_document(file_id).await?;
        Ok(WcStats::from_content(&content))
    }

    #[instrument(skip(self), fields(file_id = file_id.raw()), err(Debug))]
    pub async fn document_stat(&self, file_id: EntryId) -> Result<(DocumentMeta, String, usize)> {
        let meta = self.storage.get_file_meta(file_id).await?;
        let parent_path = self.storage.canonical_path(meta.parent_id()).await?;
        let content = self.get_document(file_id).await?;
        let lines = content.lines().count();
        Ok((meta, parent_path, lines))
    }

    #[instrument(
        skip(self, pattern),
        fields(file_id = file_id.raw(), pattern_len = pattern.len(), max_matches),
        err(Debug)
    )]
    pub async fn document_grep(
        &self,
        file_id: EntryId,
        pattern: &str,
        max_matches: usize,
        invert_match: bool,
    ) -> Result<Vec<GrepLine>> {
        let re = Regex::new(pattern).map_err(|e| Error::InvalidInput(e.to_string()))?;
        let content = self.get_document(file_id).await?;
        let cap = if max_matches == 0 {
            usize::MAX
        } else {
            max_matches
        };
        let mut out = Vec::new();
        for (i, line) in content.lines().enumerate() {
            let matched = re.is_match(line);
            if matched != invert_match {
                out.push(GrepLine::new(i + 1, line.to_string()));
                if out.len() >= cap {
                    break;
                }
            }
        }
        Ok(out)
    }

    #[instrument(
        skip(self, keywords),
        fields(
            keywords_len = keywords.as_ref().len(),
            directory = ?directory_prefix,
            limit,
        ),
        err(Debug)
    )]
    pub async fn search_hits(
        &self,
        keywords: impl AsRef<str>,
        directory_prefix: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        let norm = directory_prefix.and_then(|p| {
            let t = p.trim();
            if t.is_empty() || t == "/" {
                None
            } else {
                Some(t.trim_end_matches('/'))
            }
        });
        let scored = self.search.search_scored(keywords.as_ref(), norm).await?;
        let take = scored.len().min(limit);
        let scored: Vec<_> = scored.into_iter().take(take).collect();
        let ids: Vec<EntryId> = scored.iter().map(|(id, _)| *id).collect();
        let rows = self.storage.files_display_batch(&ids).await?;
        let mut map: HashMap<i64, (String, String)> = HashMap::new();
        for (id, path, body) in rows {
            map.insert(id.raw(), (path, body));
        }
        let mut hits = Vec::new();
        for (id, score) in scored {
            let Some((path, content)) = map.get(&id.raw()) else {
                continue;
            };
            let (snippet, line_number) = search_snippet_and_line(content, keywords.as_ref());
            hits.push(SearchHit::new(
                id,
                path.clone(),
                snippet,
                score,
                line_number,
            ));
        }
        Ok(hits)
    }

    #[instrument(skip(self, path), err(Debug))]
    pub async fn document_exists_at_path(&self, path: impl AsRef<str> + Send) -> Result<bool> {
        match self.resolve_file_path(path.as_ref()).await {
            Ok(_) => Ok(true),
            Err(Error::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Create a file at an absolute path (parent directories must exist).
    #[instrument(skip(self, path, content), err(Debug))]
    pub async fn create_document_at_path(
        &self,
        path: impl AsRef<str> + Send,
        content: impl AsRef<str> + Send,
    ) -> Result<EntryId> {
        let path = path.as_ref();
        canonical_path_segments(path)?;
        let (parent, name) = parent_and_final_name(path)?;
        validate_entity_name(&name)?;
        self.create_file_in_directory(parent, name, content.as_ref())
            .await
    }

    /// Create parent directories as needed, then create or replace file body.
    #[instrument(skip(self, path, content), err(Debug))]
    pub async fn put_document_by_path(
        &self,
        path: impl AsRef<str> + Send,
        content: impl AsRef<str> + Send,
    ) -> Result<()> {
        let path = path.as_ref();
        canonical_path_segments(path)?;
        let (parent, name) = parent_and_final_name(path)?;
        validate_entity_name(&name)?;
        self.storage.ensure_directory_path(&parent).await?;
        let content = content.as_ref();
        match self.resolve_file_path(path).await {
            Ok(id) => {
                self.update_document(id, content).await?;
            }
            Err(Error::NotFound(_)) => {
                self.create_file_in_directory(parent, name, content).await?;
            }
            Err(e) => return Err(e),
        }
        Ok(())
    }

    /// Resolve path for RPC: file path must exist as a file.
    #[instrument(skip(self, path))]
    pub async fn resolve_existing_file_path(
        &self,
        path: impl AsRef<str> + Send,
    ) -> Result<EntryId> {
        self.resolve_file_path(path.as_ref()).await
    }
}

fn slice_lines(content: &str, start: usize, end: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if start > lines.len() {
        return String::new();
    }
    let end = end.min(lines.len());
    lines[start - 1..end].join("\n")
}

fn floor_utf8_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn search_snippet_and_line(content: &str, query: &str) -> (String, Option<usize>) {
    let q = query.trim();
    if q.is_empty() {
        return (content.chars().take(200).collect(), None);
    }
    let needle = q.split_whitespace().next().unwrap_or("");
    if needle.is_empty() {
        return (content.chars().take(200).collect(), None);
    }
    let Ok(re) = Regex::new(&format!(r"(?i){}", regex::escape(needle))) else {
        return (content.chars().take(200).collect(), None);
    };
    let Some(m) = re.find(content) else {
        return (content.chars().take(200).collect(), None);
    };
    let pos = m.start();
    let match_end = m.end();
    let line_number = Some(1 + content[..pos].bytes().filter(|&b| b == b'\n').count());
    let start_byte = pos.saturating_sub(40);
    let start = floor_utf8_boundary(content, start_byte);
    let slice = content.get(start..).unwrap_or(content);
    let mut s: String = slice.chars().take(200).collect();
    if start > 0 {
        s.insert(0, '…');
    }
    if match_end < content.len() {
        s.push('…');
    }
    (s, line_number)
}
