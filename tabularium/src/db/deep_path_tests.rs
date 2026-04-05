//! Deep hierarchical paths — proof for the forge (Ferrum § test audit).

use super::{EntryId, EntryKind, SqliteDatabase};
use crate::Error;

fn temp_db() -> (tempfile::TempDir, String, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("deep.db");
    let idx_path = dir.path().join("deep.idx");
    let uri = format!("sqlite://{}", db_path.display());
    (dir, uri, idx_path)
}

#[tokio::test]
async fn deep_mkdir_chain_resolve_and_missing_parent() {
    let (_d, uri, idx) = temp_db();
    let db = SqliteDatabase::init(&uri, &idx, 0).await.unwrap();
    let id_a = db.create_directory("/a", None).await.unwrap();
    let id_ab = db.create_directory("/a/b", None).await.unwrap();
    let id_abc = db.create_directory("/a/b/c", None).await.unwrap();
    assert_ne!(id_a, id_ab);
    assert_ne!(id_ab, id_abc);
    assert_eq!(db.resolve_directory_path("/a/b/c").await.unwrap(), id_abc);
    let err = db.create_directory("/x/y/z", None).await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)));
}

#[tokio::test]
async fn five_level_canonical_paths_and_root() {
    let (_d, uri, idx) = temp_db();
    let db = SqliteDatabase::init(&uri, &idx, 0).await.unwrap();
    db.create_directory("/l1", None).await.unwrap();
    db.create_directory("/l1/l2", None).await.unwrap();
    db.create_directory("/l1/l2/l3", None).await.unwrap();
    db.create_directory("/l1/l2/l3/l4", None).await.unwrap();
    let id_e = db.create_directory("/l1/l2/l3/l4/l5", None).await.unwrap();
    assert_eq!(
        db.test_storage_canonical_path(id_e).await.unwrap(),
        "/l1/l2/l3/l4/l5"
    );
    let root = EntryId::from_raw(1);
    assert_eq!(db.test_storage_canonical_path(root).await.unwrap(), "/");
}

#[tokio::test]
async fn root_file_listing_and_resolve() {
    let (_d, uri, idx) = temp_db();
    let db = SqliteDatabase::init(&uri, &idx, 0).await.unwrap();
    let id = db
        .create_document_at_path("/root_doc", "body-root")
        .await
        .unwrap();
    assert_eq!(db.get_document(id).await.unwrap(), "body-root");
    let listed = db.list_directory("/").await.unwrap();
    let names: Vec<_> = listed.iter().map(super::ListedEntry::name).collect();
    assert!(names.contains(&"root_doc"));
    assert_eq!(db.resolve_file_path("/root_doc").await.unwrap(), id);
}

#[tokio::test]
async fn ensure_directory_path_idempotent_and_deep_file_crud() {
    let (_d, uri, idx) = temp_db();
    let db = SqliteDatabase::init(&uri, &idx, 0).await.unwrap();
    let leaf = db
        .test_storage_ensure_directory_path("/p/q/r/s")
        .await
        .unwrap();
    let again = db
        .test_storage_ensure_directory_path("/p/q/r/s")
        .await
        .unwrap();
    assert_eq!(leaf, again);
    assert_eq!(
        db.test_storage_resolve_path("/p/q/r/s", Some(EntryKind::Dir))
            .await
            .unwrap(),
        leaf
    );
    let fid = db
        .create_file_in_directory("/p/q/r/s", "note", "v1")
        .await
        .unwrap();
    assert_eq!(db.get_document(fid).await.unwrap(), "v1");
    db.update_document(fid, "v2").await.unwrap();
    assert_eq!(db.get_document(fid).await.unwrap(), "v2");
    db.append_document(fid, "more").await.unwrap();
    assert_eq!(db.get_document(fid).await.unwrap(), "v2\nmore");
    db.delete_document(fid).await.unwrap();
    assert!(
        db.test_storage_resolve_path("/p/q/r/s/note", Some(EntryKind::File))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn list_directory_mixed_kinds_at_depths() {
    let (_d, uri, idx) = temp_db();
    let db = SqliteDatabase::init(&uri, &idx, 0).await.unwrap();
    db.create_directory("/a", None).await.unwrap();
    db.create_directory("/a/b", None).await.unwrap();
    db.create_directory("/a/b/c", None).await.unwrap();
    db.create_directory("/a/d", None).await.unwrap();
    db.create_file_in_directory("/a", "doc.txt", "x")
        .await
        .unwrap();
    let at_a = db.list_directory("/a").await.unwrap();
    let mut names: Vec<_> = at_a.iter().map(|e| (e.name(), e.kind())).collect();
    names.sort_by(|x, y| x.0.cmp(y.0));
    assert_eq!(names.len(), 3);
    assert!(names.iter().any(|(n, k)| *n == "b" && *k == EntryKind::Dir));
    assert!(names.iter().any(|(n, k)| *n == "d" && *k == EntryKind::Dir));
    assert!(
        names
            .iter()
            .any(|(n, k)| *n == "doc.txt" && *k == EntryKind::File)
    );
    let at_ab = db.list_directory("/a/b").await.unwrap();
    assert_eq!(at_ab.len(), 1);
    assert_eq!(at_ab[0].name(), "c");
    assert_eq!(at_ab[0].kind(), EntryKind::Dir);
}

#[tokio::test]
async fn move_file_between_directories_updates_search() {
    let (_d, uri, idx) = temp_db();
    let db = SqliteDatabase::init(&uri, &idx, 0).await.unwrap();
    db.create_directory("/src_dir", None).await.unwrap();
    db.create_directory("/dst_dir", None).await.unwrap();
    let fid = db
        .create_file_in_directory("/src_dir", "doc", "moveneedle_unique")
        .await
        .unwrap();
    let h1 = db
        .search_hits("moveneedle_unique", Some("/src_dir"), 10)
        .await
        .unwrap();
    assert_eq!(h1.len(), 1);
    db.move_document_to_directory(fid, "/dst_dir", "doc")
        .await
        .unwrap();
    assert!(
        db.search_hits("moveneedle_unique", Some("/src_dir"), 10)
            .await
            .unwrap()
            .is_empty()
    );
    let h2 = db
        .search_hits("moveneedle_unique", Some("/dst_dir"), 10)
        .await
        .unwrap();
    assert_eq!(h2.len(), 1);
    assert_eq!(h2[0].path(), "/dst_dir/doc");
}

#[tokio::test]
async fn move_directory_valid_and_rejects_cycle() {
    let (_d, uri, idx) = temp_db();
    let db = SqliteDatabase::init(&uri, &idx, 0).await.unwrap();
    db.create_directory("/tree", None).await.unwrap();
    db.create_directory("/tree/a", None).await.unwrap();
    db.create_directory("/tree/a/b", None).await.unwrap();
    db.create_directory("/tree/other", None).await.unwrap();
    db.move_directory("/tree/a/b", "/tree/other", "nested")
        .await
        .unwrap();
    db.resolve_directory_path("/tree/other/nested")
        .await
        .unwrap();

    db.create_directory("/cyc", None).await.unwrap();
    db.create_directory("/cyc/a", None).await.unwrap();
    db.create_directory("/cyc/a/b", None).await.unwrap();
    let err = db
        .move_directory("/cyc/a", "/cyc/a/b", "bad")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)));
}

#[tokio::test]
async fn recursive_delete_clears_storage_and_search() {
    let (_d, uri, idx) = temp_db();
    let db = SqliteDatabase::init(&uri, &idx, 0).await.unwrap();
    db.create_directory("/deep", None).await.unwrap();
    db.create_directory("/deep/x", None).await.unwrap();
    db.create_directory("/deep/x/y", None).await.unwrap();
    db.create_file_in_directory("/deep/x/y", "f1", "deepf1needle")
        .await
        .unwrap();
    db.create_file_in_directory("/deep/x/y", "f2", "deepf2needle")
        .await
        .unwrap();
    assert_eq!(
        db.search_hits("deepf1needle", None, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    db.delete_directory_recursive("/deep").await.unwrap();
    assert!(
        db.search_hits("deepf1needle", None, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(db.resolve_directory_path("/deep").await.is_err());
}

#[tokio::test]
async fn rename_directory_and_file_at_depth() {
    let (_d, uri, idx) = temp_db();
    let db = SqliteDatabase::init(&uri, &idx, 0).await.unwrap();
    db.create_directory("/d", None).await.unwrap();
    db.create_directory("/d/old", None).await.unwrap();
    let fid = db
        .create_file_in_directory("/d/old", "f", "txt")
        .await
        .unwrap();
    db.rename_directory("/d/old", "/d/new").await.unwrap();
    db.resolve_directory_path("/d/new").await.unwrap();
    assert!(db.resolve_directory_path("/d/old").await.is_err());
    assert_eq!(db.resolve_file_path("/d/new/f").await.unwrap(), fid);
    db.rename_document(fid, "g").await.unwrap();
    assert!(db.resolve_file_path("/d/new/f").await.is_err());
    db.resolve_file_path("/d/new/g").await.unwrap();
}

#[tokio::test]
async fn search_subtree_scoped_and_segment_boundary() {
    let (_d, uri, idx) = temp_db();
    let db = SqliteDatabase::init(&uri, &idx, 0).await.unwrap();
    db.create_directory("/ns", None).await.unwrap();
    db.create_directory("/ns/a", None).await.unwrap();
    db.create_directory("/ns/b", None).await.unwrap();
    db.create_file_in_directory("/ns/a", "doc1", "unique_deep_a_alpha")
        .await
        .unwrap();
    db.create_file_in_directory("/ns/b", "doc2", "unique_deep_b_beta")
        .await
        .unwrap();
    let a_only = db
        .search_hits("unique_deep_a", Some("/ns/a"), 10)
        .await
        .unwrap();
    assert_eq!(a_only.len(), 1);
    assert_eq!(a_only[0].path(), "/ns/a/doc1");
    let ns_b = db
        .search_hits("unique_deep_b", Some("/ns"), 10)
        .await
        .unwrap();
    assert_eq!(ns_b.len(), 1);
    assert!(
        db.search_hits("unique_deep_a", Some("/ns/b"), 10)
            .await
            .unwrap()
            .is_empty()
    );
    let both = db.search_hits("unique_deep", None, 10).await.unwrap();
    assert_eq!(both.len(), 2);
    db.create_directory("/segtrap", None).await.unwrap();
    db.create_directory("/segtrap/a", None).await.unwrap();
    db.create_directory("/segtrap/a/b", None).await.unwrap();
    db.create_directory("/segtrap/a/bc", None).await.unwrap();
    db.create_file_in_directory("/segtrap/a/b", "x", "trapxyz123seg")
        .await
        .unwrap();
    db.create_file_in_directory("/segtrap/a/bc", "y", "trapxyz123seg")
        .await
        .unwrap();
    let scoped = db
        .search_hits("trapxyz123seg", Some("/segtrap/a/b"), 10)
        .await
        .unwrap();
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].path(), "/segtrap/a/b/x");
}
