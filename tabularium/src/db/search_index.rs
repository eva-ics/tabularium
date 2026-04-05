//! Tantivy full-text index — separate from `Storage`, as the meeting commanded.

use std::path::Path;

use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, Query, QueryParser, RegexQuery, TermQuery};
use tantivy::schema::document::OwnedValue;
use tantivy::schema::{Field, IndexRecordOption, Schema, TextFieldIndexing, TextOptions};
use tantivy::{Index, IndexReader, IndexWriter, TantivyDocument, Term, doc};
use tracing::instrument;

use crate::db::EntryId;
use crate::{Error, Result};

const WRITER_HEAP: usize = 50_000_000;
const SEARCH_LIMIT: usize = 10_000;

/// Subtree filter on the raw-tokenized `dir_path` field: exact `norm` (files in that directory)
/// or a strict descendant (`/a/b` matches `/a/b` and `/a/b/c`, not `/a/bc`).
///
/// [`RegexQuery`] uses `tantivy_fst::Regex`: each pattern matches the whole term (implicit
/// `^…$`); literal `^` / `$` in the pattern are rejected. Child paths use `{norm}/.+`.
fn dir_path_subtree_query(field: Field, norm: &str) -> Result<Box<dyn Query>> {
    let term = Term::from_field_text(field, norm);
    let exact_q: Box<dyn Query> = Box::new(TermQuery::new(term, IndexRecordOption::Basic));
    let child_pat = format!("{}/.+", regex::escape(norm));
    let child_q: Box<dyn Query> = Box::new(
        RegexQuery::from_pattern(&child_pat, field).map_err(|e| Error::Search(e.to_string()))?,
    );
    Ok(Box::new(BooleanQuery::new(vec![
        (Occur::Should, exact_q),
        (Occur::Should, child_q),
    ])))
}

fn numeric_id_options() -> tantivy::schema::NumericOptions {
    tantivy::schema::NumericOptions::default()
        .set_indexed()
        .set_fast()
        .set_stored()
}

fn raw_text_options() -> TextOptions {
    TextOptions::default().set_stored().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("raw")
            .set_index_option(IndexRecordOption::Basic),
    )
}

fn build_schema() -> Schema {
    let mut b = Schema::builder();
    b.add_u64_field("doc_id", numeric_id_options());
    b.add_text_field("dir_path", raw_text_options());
    let text_opts = TextOptions::default().set_stored().set_indexing_options(
        tantivy::schema::TextFieldIndexing::default()
            .set_tokenizer("default")
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    b.add_text_field("file_name", text_opts.clone());
    b.add_text_field("description", text_opts.clone());
    b.add_text_field("content", text_opts);
    b.build()
}

fn schema_fields(schema: &Schema) -> Result<(Field, Field, Field, Field, Field)> {
    const STALE: &str = "search index schema is outdated — remove the index directory or run `reindex /` after upgrade";
    let err = || Error::Search(STALE.into());
    Ok((
        schema.get_field("doc_id").map_err(|_| err())?,
        schema.get_field("dir_path").map_err(|_| err())?,
        schema.get_field("file_name").map_err(|_| err())?,
        schema.get_field("description").map_err(|_| err())?,
        schema.get_field("content").map_err(|_| err())?,
    ))
}

/// Full-text search sidecar; lives beside the SQLite file per meeting notes.
pub struct SearchIndex {
    index: Index,
    writer: tokio::sync::Mutex<IndexWriter>,
    reader: tokio::sync::Mutex<IndexReader>,
    f_doc_id: Field,
    f_dir_path: Field,
    f_file_name: Field,
    f_description: Field,
    f_content: Field,
}

impl SearchIndex {
    /// Opens an existing index or creates a new directory + schema.
    #[instrument(name = "search_index_open", skip(index_path), fields(path = %index_path.as_ref().display()), err(Debug))]
    pub fn open(index_path: impl AsRef<Path>) -> Result<Self> {
        let index_path = index_path.as_ref();
        fs_err::create_dir_all(index_path)?;
        let schema = build_schema();
        let index = if index_path.join("meta.json").exists() {
            Index::open_in_dir(index_path)?
        } else {
            Index::create_in_dir(index_path, schema)?
        };
        let (f_doc_id, f_dir_path, f_file_name, f_description, f_content) =
            schema_fields(&index.schema())?;
        let writer = index.writer(WRITER_HEAP)?;
        let reader = index.reader()?;
        Ok(Self {
            index,
            writer: tokio::sync::Mutex::new(writer),
            reader: tokio::sync::Mutex::new(reader),
            f_doc_id,
            f_dir_path,
            f_file_name,
            f_description,
            f_content,
        })
    }

    async fn reload_reader(&self) -> Result<()> {
        self.reader.lock().await.reload()?;
        Ok(())
    }

    /// Replace or insert a file row in the index (by sqlite entry id).
    #[instrument(
        skip(self, file_name, description, content),
        fields(
            file_id = file_id.raw(),
            dir_path = %dir_path,
            content_len = content.len()
        ),
        err(Debug)
    )]
    pub async fn upsert_file(
        &self,
        file_id: EntryId,
        dir_path: &str,
        file_name: &str,
        description: &str,
        content: &str,
    ) -> Result<()> {
        let mut w = self.writer.lock().await;
        let doc_u64 = u64::try_from(file_id.raw())
            .map_err(|_| Error::InvalidInput(format!("file id {} out of range", file_id.raw())))?;
        w.delete_term(Term::from_field_u64(self.f_doc_id, doc_u64));
        let d = doc!(
            self.f_doc_id => doc_u64,
            self.f_dir_path => dir_path,
            self.f_file_name => file_name,
            self.f_description => description,
            self.f_content => content,
        );
        w.add_document(d)?;
        w.commit()?;
        drop(w);
        self.reload_reader().await?;
        Ok(())
    }

    #[instrument(skip(self), fields(file_id = file_id.raw()), err(Debug))]
    pub async fn delete_file(&self, file_id: EntryId) -> Result<()> {
        let mut w = self.writer.lock().await;
        w.delete_term(Term::from_field_u64(
            self.f_doc_id,
            u64::try_from(file_id.raw()).map_err(|_| {
                Error::InvalidInput(format!("file id {} out of range", file_id.raw()))
            })?,
        ));
        w.commit()?;
        drop(w);
        self.reload_reader().await?;
        Ok(())
    }

    /// Clear every indexed segment, then insert rows in one commit (full rebuild from SQLite).
    #[instrument(skip(self, rows), fields(row_count = rows.len()), err(Debug))]
    pub async fn replace_all_from_rows(
        &self,
        rows: &[(EntryId, String, String, String, String)],
    ) -> Result<()> {
        let mut w = self.writer.lock().await;
        w.delete_all_documents()?;
        for (file_id, dir_path, file_name, description, content) in rows {
            let doc_u64 = u64::try_from(file_id.raw()).map_err(|_| {
                Error::InvalidInput(format!("file id {} out of range", file_id.raw()))
            })?;
            let d = doc!(
                self.f_doc_id => doc_u64,
                self.f_dir_path => dir_path.as_str(),
                self.f_file_name => file_name.as_str(),
                self.f_description => description.as_str(),
                self.f_content => content.as_str(),
            );
            w.add_document(d)?;
        }
        w.commit()?;
        drop(w);
        self.reload_reader().await
    }

    /// Re-index a subset of files without wiping other indexed documents.
    #[instrument(skip(self, rows), fields(row_count = rows.len()), err(Debug))]
    pub async fn upsert_batch(
        &self,
        rows: &[(EntryId, String, String, String, String)],
    ) -> Result<()> {
        let mut w = self.writer.lock().await;
        for (file_id, dir_path, file_name, description, content) in rows {
            let doc_u64 = u64::try_from(file_id.raw()).map_err(|_| {
                Error::InvalidInput(format!("file id {} out of range", file_id.raw()))
            })?;
            w.delete_term(Term::from_field_u64(self.f_doc_id, doc_u64));
            let d = doc!(
                self.f_doc_id => doc_u64,
                self.f_dir_path => dir_path.as_str(),
                self.f_file_name => file_name.as_str(),
                self.f_description => description.as_str(),
                self.f_content => content.as_str(),
            );
            w.add_document(d)?;
        }
        w.commit()?;
        drop(w);
        self.reload_reader().await
    }

    /// Keyword search; optional directory prefix (subtree). Returns ids by descending relevance score.
    #[instrument(
        skip(self, keywords),
        fields(keywords_len = keywords.as_ref().len(), directory = ?directory_prefix),
        err(Debug)
    )]
    pub async fn search(
        &self,
        keywords: impl AsRef<str>,
        directory_prefix: Option<&str>,
    ) -> Result<Vec<EntryId>> {
        Ok(self
            .search_scored_inner(keywords, directory_prefix)
            .await?
            .into_iter()
            .map(|(id, _)| id)
            .collect())
    }

    /// Same as [`Self::search`], but retains Tantivy scores for ranking in APIs.
    #[instrument(
        skip(self, keywords),
        fields(keywords_len = keywords.as_ref().len(), directory = ?directory_prefix),
        err(Debug)
    )]
    pub async fn search_scored(
        &self,
        keywords: impl AsRef<str>,
        directory_prefix: Option<&str>,
    ) -> Result<Vec<(EntryId, f32)>> {
        self.search_scored_inner(keywords, directory_prefix).await
    }

    async fn search_scored_inner(
        &self,
        keywords: impl AsRef<str>,
        directory_prefix: Option<&str>,
    ) -> Result<Vec<(EntryId, f32)>> {
        let reader = self.reader.lock().await;
        let searcher = reader.searcher();
        let parser = QueryParser::for_index(
            &self.index,
            vec![self.f_content, self.f_file_name, self.f_description],
        );
        let text_q = parser
            .parse_query(keywords.as_ref())
            .map_err(|e| Error::Search(e.to_string()))?;
        let q: Box<dyn Query> = match directory_prefix {
            None | Some("") => text_q,
            Some(prefix) => {
                let norm = prefix.trim().trim_end_matches('/').to_string();
                if norm.is_empty() {
                    text_q
                } else {
                    let dir_q = dir_path_subtree_query(self.f_dir_path, &norm)?;
                    Box::new(BooleanQuery::new(vec![
                        (Occur::Must, text_q),
                        (Occur::Must, dir_q),
                    ]))
                }
            }
        };
        let top = searcher
            .search(&q, &TopDocs::with_limit(SEARCH_LIMIT))
            .map_err(|e| Error::Search(e.to_string()))?;
        let mut out = Vec::with_capacity(top.len());
        for (score, addr) in top {
            let d: TantivyDocument = searcher
                .doc(addr)
                .map_err(|e| Error::Search(e.to_string()))?;
            let Some(OwnedValue::U64(id)) = d.get_first(self.f_doc_id).cloned() else {
                return Err(Error::Search("missing doc_id in indexed document".into()));
            };
            out.push((
                EntryId::from_raw(
                    i64::try_from(id)
                        .map_err(|_| Error::Search("doc_id does not fit i64".into()))?,
                ),
                score,
            ));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod dir_path_regex_tests {
    use regex::Regex;

    #[test]
    fn subtree_child_pattern_respects_segment_boundary() {
        let esc = regex::escape("/a/b");
        let child = format!("{}/.+", esc);
        let re = Regex::new(&format!("^{child}$")).unwrap();
        assert!(re.is_match("/a/b/c"));
        assert!(!re.is_match("/a/b"));
        assert!(!re.is_match("/a/bc"));
        assert!(!re.is_match("/a/b_other"));
    }
}
