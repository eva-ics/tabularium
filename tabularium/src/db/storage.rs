//! Backend storage contract — no cache, no search; the façade coordinates those relics.

use async_trait::async_trait;

use super::EntryId;
use super::entry_kind::EntryKind;
use super::meta::{DocumentMeta, ListedEntry};
use crate::{Result, Timestamp};

/// Async persistence for the hierarchical `entries` table (`sqlx` today, another engine tomorrow).
#[async_trait]
pub trait Storage: Send + Sync {
    /// Updates only `accessed_at` for a file row.
    async fn touch(&self, file_id: EntryId) -> Result<()>;

    /// Resolve an absolute path to an entry id (`/` → root directory).
    async fn resolve_path(&self, path: &str, expected_kind: Option<EntryKind>) -> Result<EntryId>;

    /// Canonical absolute path for any entry (e.g. `/a/b`); root is `/`.
    async fn canonical_path(&self, entry_id: EntryId) -> Result<String>;

    /// Parent directory canonical path for a file (for search index prefix field).
    async fn parent_dir_path_for_file(&self, file_id: EntryId) -> Result<String>;

    /// Create a single directory at `path` (parent must exist).
    async fn create_directory(&self, path: &str, description: Option<&str>) -> Result<EntryId>;

    /// Delete directory if empty; fails with [`crate::Error::NotEmpty`] if it has children.
    async fn delete_directory(&self, path: &str) -> Result<()>;

    /// Remove directory and full subtree (files and subdirectories).
    async fn delete_directory_recursive(&self, path: &str) -> Result<()>;

    /// Rename a directory’s last segment within the same parent (`old_path` → `new_path`).
    async fn rename_directory(&self, old_path: &str, new_path: &str) -> Result<()>;

    /// Create a file at `path` (parent directory must exist).
    async fn create_file(&self, path: &str, content: impl AsRef<str> + Send) -> Result<EntryId>;

    async fn delete_file(&self, file_id: EntryId) -> Result<()>;

    async fn update_file(
        &self,
        file_id: EntryId,
        new_content: impl AsRef<str> + Send,
    ) -> Result<()>;

    async fn append_file(&self, file_id: EntryId, to_append: impl AsRef<str> + Send) -> Result<()>;

    /// Bump `modified_at` only (content and `created_at` unchanged).
    async fn bump_file_modified_at(&self, file_id: EntryId) -> Result<()>;

    /// Set `modified_at` for a file or directory row (not root).
    async fn set_entry_modified_at(&self, entry_id: EntryId, modified_at: Timestamp) -> Result<()>;

    /// Move file to a new parent directory (by path); optional rename via final segment of `new_path`.
    async fn move_file(
        &self,
        file_id: EntryId,
        new_parent_path: &str,
        new_name: &str,
    ) -> Result<()>;

    /// Rename file within the same parent (`new_name` only).
    async fn rename_file(&self, file_id: EntryId, new_name: impl AsRef<str> + Send) -> Result<()>;

    async fn get_file_content(&self, file_id: EntryId) -> Result<String>;

    /// Children of directory at `dir_path` (`/` for root), mixed kinds.
    async fn list_directory(&self, dir_path: &str) -> Result<Vec<ListedEntry>>;

    async fn get_file_meta(&self, file_id: EntryId) -> Result<DocumentMeta>;

    /// For search hits: id → (canonical_path, content).
    async fn files_display_batch(&self, ids: &[EntryId]) -> Result<Vec<(EntryId, String, String)>>;

    async fn file_parent_and_content(&self, file_id: EntryId) -> Result<(EntryId, String)>;

    /// All files, or only files under `directory_path` subtree, for Tantivy rebuild.
    /// Tuple: `(id, parent_dir_path, file_name, description, content)`.
    async fn files_for_search_reindex(
        &self,
        directory_path_filter: Option<&str>,
    ) -> Result<Vec<(EntryId, String, String, String, String)>>;

    /// Fields needed for one search index row (`dir_path`, `name`, `description`, `content`).
    async fn file_search_index_fields(
        &self,
        file_id: EntryId,
    ) -> Result<(String, String, String, String)>;

    /// Move directory to new parent path + new name (last segment); anti-cycle enforced in SQL txn.
    async fn move_directory(
        &self,
        dir_id: EntryId,
        new_parent_path: &str,
        new_name: &str,
    ) -> Result<()>;

    /// Ensure each segment exists as a directory from root (mkdir -p).
    async fn ensure_directory_path(&self, dir_path: &str) -> Result<EntryId>;

    /// `description` column for a file or directory at `path` (`None` when unset).
    async fn entry_description(&self, path: &str) -> Result<Option<String>>;

    /// Set or clear `description` for a file or directory; `None` stores SQL NULL.
    async fn set_entry_description(&self, path: &str, description: Option<&str>) -> Result<()>;
}
