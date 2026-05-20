use std::collections::HashMap;
use std::ffi::OsStr;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use clap::Parser;
use fuser::{
    FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyData, ReplyDirectory, ReplyEmpty,
    ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, ReplyXattr, Request, TimeOrNow,
};
use tabularium::rpc::Client;
use tokio::runtime::Runtime;
use tracing_subscriber::EnvFilter;

mod write_ops;

const ROOT_INODE: u64 = 1;
const RPC_TIMEOUT_SECS: u64 = 15;
const TTL: Duration = Duration::from_secs(1);
const DEFAULT_RUST_LOG: &str = "tabularium_fuse=info";

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_RUST_LOG)),
        )
        .try_init();
}

#[cfg(target_os = "macos")]
fn statfs_fstype(path: &Path) -> std::io::Result<String> {
    use std::ffi::{CStr, CString};
    use std::mem::MaybeUninit;

    let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "mountpoint path contains NUL",
        )
    })?;
    let mut buf = MaybeUninit::<libc::statfs>::uninit();
    let rc = unsafe { libc::statfs(c_path.as_ptr(), buf.as_mut_ptr()) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let buf = unsafe { buf.assume_init() };
    let s = unsafe { CStr::from_ptr(buf.f_fstypename.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    Ok(s)
}

#[cfg(target_os = "macos")]
fn fstype_suggests_fuse(fstype: &str) -> bool {
    let lower = fstype.to_lowercase();
    lower.contains("fuse") || lower.contains("macfuse") || lower.contains("osxfuse")
}

#[cfg(not(target_os = "macos"))]
fn statfs_fstype(_path: &Path) -> std::io::Result<String> {
    Ok(String::new())
}

#[cfg(not(target_os = "macos"))]
fn fstype_suggests_fuse(_fstype: &str) -> bool {
    false
}

#[derive(Debug, Clone, Parser)]
#[command(name = "tabularium-fuse")]
#[command(about = "Mount Tabularium as a local FUSE filesystem")]
struct Args {
    #[arg(long, env = "TABULARIUM_URL", default_value = "http://127.0.0.1:3050")]
    api_uri: String,
    #[arg(long)]
    mountpoint: PathBuf,
    #[arg(long, default_value_t = false)]
    read_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Directory,
    File,
}

#[derive(Debug, Clone)]
struct EntryMeta {
    kind: EntryKind,
    id: Option<tabularium::EntryId>,
    size_bytes: u64,
    modified: SystemTime,
}

fn split_parent_name(path: &str) -> Result<(&str, &str), i32> {
    if !path.starts_with('/') || path == "/" {
        return Err(libc::EINVAL);
    }
    let trimmed = path.trim_end_matches('/');
    let (parent, name) = trimmed.rsplit_once('/').ok_or(libc::EINVAL)?;
    let parent = if parent.is_empty() { "/" } else { parent };
    if name.is_empty() {
        return Err(libc::EINVAL);
    }
    Ok((parent, name))
}

fn join_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

fn inode_from_entry_id(id: tabularium::EntryId) -> u64 {
    let raw = i64::from(id);
    if raw <= 0 {
        return ROOT_INODE + 1;
    }
    (raw as u64).saturating_add(2)
}

fn ts_to_system_time(ts: tabularium::Timestamp) -> SystemTime {
    SystemTime::UNIX_EPOCH + ts.as_duration()
}

fn file_attr(ino: u64, kind: EntryKind, size_bytes: u64, modified: SystemTime) -> FileAttr {
    let now = modified;
    FileAttr {
        ino,
        size: size_bytes,
        blocks: size_bytes.div_ceil(512),
        atime: now,
        mtime: modified,
        ctime: modified,
        crtime: modified,
        kind: if kind == EntryKind::Directory {
            FileType::Directory
        } else {
            FileType::RegularFile
        },
        perm: if kind == EntryKind::Directory {
            0o755
        } else {
            0o644
        },
        nlink: if kind == EntryKind::Directory { 2 } else { 1 },
        uid: unsafe { libc::geteuid() },
        gid: unsafe { libc::getegid() },
        rdev: 0,
        blksize: 4096,
        flags: 0,
    }
}

struct TbFs {
    client: Client,
    rt: Runtime,
    inode_to_path: Arc<Mutex<HashMap<u64, String>>>,
    read_only: bool,
    next_fh: Arc<Mutex<u64>>,
    fh_state: Arc<Mutex<HashMap<u64, FileHandleState>>>,
}

#[derive(Debug, Clone)]
struct FileHandleState {
    path: String,
    data: Vec<u8>,
    dirty: bool,
}

impl TbFs {
    fn new(client: Client, read_only: bool) -> Result<Self, std::io::Error> {
        let mut inode_to_path = HashMap::new();
        inode_to_path.insert(ROOT_INODE, "/".to_string());
        Ok(Self {
            client,
            rt: Runtime::new()?,
            inode_to_path: Arc::new(Mutex::new(inode_to_path)),
            read_only,
            next_fh: Arc::new(Mutex::new(1)),
            fh_state: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn path_for_inode(&self, ino: u64) -> Option<String> {
        self.inode_to_path.lock().ok()?.get(&ino).cloned()
    }

    fn remember_path(&self, ino: u64, path: &str) {
        if let Ok(mut map) = self.inode_to_path.lock() {
            map.insert(ino, path.to_string());
        }
    }

    fn map_error(err: &tabularium::Error) -> i32 {
        match err {
            tabularium::Error::NotFound(_) => libc::ENOENT,
            tabularium::Error::Duplicate(_) => libc::EEXIST,
            tabularium::Error::NotEmpty(_) => libc::ENOTEMPTY,
            tabularium::Error::RevisionMismatch(_) => libc::ESTALE,
            tabularium::Error::Unauthorized(_) => libc::EACCES,
            tabularium::Error::Forbidden(_) => libc::EPERM,
            tabularium::Error::InvalidInput(_) => libc::EINVAL,
            tabularium::Error::Sql(_) | tabularium::Error::Search(_) | tabularium::Error::Io(_) => {
                libc::EIO
            }
        }
    }

    fn rpc_err_is_missing_path(err: &tabularium::Error) -> bool {
        match err {
            tabularium::Error::NotFound(_) => true,
            tabularium::Error::InvalidInput(msg) => msg.contains("not found"),
            _ => false,
        }
    }

    fn lookup_file(&self, path: &str) -> Result<EntryMeta, i32> {
        let stat = match self.rt.block_on(self.client.document_stat(path)) {
            Ok(s) => s,
            Err(e) if Self::rpc_err_is_missing_path(&e) => return Err(libc::ENOENT),
            Err(e) => return Err(Self::map_error(&e)),
        };
        Ok(EntryMeta {
            kind: EntryKind::File,
            id: Some(stat.id()),
            size_bytes: stat.size_bytes().max(0) as u64,
            modified: ts_to_system_time(stat.modified_at()),
        })
    }

    fn lookup_directory(&self, path: &str) -> Result<EntryMeta, i32> {
        if path == "/" {
            return Ok(EntryMeta {
                kind: EntryKind::Directory,
                id: None,
                size_bytes: 0,
                modified: SystemTime::UNIX_EPOCH,
            });
        }
        let (parent, name) = split_parent_name(path)?;
        let rows = self
            .rt
            .block_on(self.client.list_directory(parent))
            .map_err(|e| Self::map_error(&e))?;
        let Some(row) = rows.into_iter().find(|r| r.is_directory() && r.name() == name) else {
            return Err(libc::ENOENT);
        };
        Ok(EntryMeta {
            kind: EntryKind::Directory,
            id: Some(row.id()),
            size_bytes: 0,
            modified: ts_to_system_time(row.modified_at),
        })
    }

    fn lookup_entry(&self, path: &str) -> Result<EntryMeta, i32> {
        match self.lookup_file(path) {
            Ok(meta) => Ok(meta),
            Err(code) if code == libc::ENOENT => self.lookup_directory(path),
            Err(code) => Err(code),
        }
    }

    fn parent_inode_for_dir_path(&self, dir_path: &str) -> Result<u64, i32> {
        if dir_path == "/" {
            return Ok(ROOT_INODE);
        }
        let (parent, _) = split_parent_name(dir_path)?;
        let meta = self.lookup_entry(parent)?;
        Ok(meta.id.map_or(ROOT_INODE, inode_from_entry_id))
    }

}

impl Filesystem for TbFs {
    fn statfs(&mut self, _req: &Request<'_>, _ino: u64, reply: ReplyStatfs) {
        reply.statfs(
            1_000_000,
            500_000,
            500_000,
            1_000_000,
            500_000,
            4096,
            255,
            4096,
        );
    }

    fn listxattr(&mut self, _req: &Request<'_>, ino: u64, size: u32, reply: ReplyXattr) {
        let Some(_) = self.path_for_inode(ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        if size == 0 {
            reply.size(0);
        } else {
            reply.data(&[]);
        }
    }

    fn getxattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _name: &OsStr,
        _size: u32,
        reply: ReplyXattr,
    ) {
        let Some(_) = self.path_for_inode(ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        reply.error(libc::ENOATTR);
    }

    fn access(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        _mask: i32,
        reply: ReplyEmpty,
    ) {
        reply.ok();
    }

    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let Some(parent_path) = self.path_for_inode(parent) else {
            reply.error(libc::ENOENT);
            return;
        };
        let Some(name) = name.to_str() else {
            reply.error(libc::EINVAL);
            return;
        };
        let full = join_path(&parent_path, name);
        match self.lookup_entry(&full) {
            Ok(meta) => {
                let ino = meta.id.map_or(ROOT_INODE, inode_from_entry_id);
                self.remember_path(ino, &full);
                let attr = file_attr(ino, meta.kind, meta.size_bytes, meta.modified);
                reply.entry(&TTL, &attr, 0);
            }
            Err(code) => reply.error(code),
        }
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        let Some(path) = self.path_for_inode(ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        match self.lookup_entry(&path) {
            Ok(meta) => {
                let attr = file_attr(ino, meta.kind, meta.size_bytes, meta.modified);
                reply.attr(&TTL, &attr);
            }
            Err(code) => reply.error(code),
        }
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        if offset < 0 {
            reply.error(libc::EINVAL);
            return;
        }
        let Some(path) = self.path_for_inode(ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        let rows = match self.rt.block_on(self.client.list_directory(&path)) {
            Ok(rows) => rows,
            Err(e) => {
                reply.error(Self::map_error(&e));
                return;
            }
        };
        let parent_ino = match self.parent_inode_for_dir_path(&path) {
            Ok(v) => v,
            Err(code) => {
                reply.error(code);
                return;
            }
        };
        let mut entries: Vec<(u64, FileType, String)> = vec![
            (ino, FileType::Directory, ".".to_string()),
            (parent_ino, FileType::Directory, "..".to_string()),
        ];
        for row in rows {
            let kind = if row.is_directory() {
                FileType::Directory
            } else {
                FileType::RegularFile
            };
            let child_path = join_path(&path, row.name());
            let child_ino = inode_from_entry_id(row.id());
            self.remember_path(child_ino, &child_path);
            entries.push((child_ino, kind, row.name().to_string()));
        }
        for (idx, (e_ino, kind, name)) in entries.into_iter().enumerate().skip(offset as usize) {
            if reply.add(e_ino, (idx + 1) as i64, kind, name) {
                break;
            }
        }
        reply.ok();
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        if offset < 0 {
            reply.error(libc::EINVAL);
            return;
        }
        let Some(path) = self.path_for_inode(ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        let body = match self.rt.block_on(self.client.get_document(&path)) {
            Ok(v) => v,
            Err(e) if Self::rpc_err_is_missing_path(&e) => {
                reply.error(libc::ENOENT);
                return;
            }
            Err(e) => {
                reply.error(Self::map_error(&e));
                return;
            }
        };
        let bytes = body.content().as_bytes();
        let start = (offset as usize).min(bytes.len());
        let end = start.saturating_add(size as usize).min(bytes.len());
        reply.data(&bytes[start..end]);
    }

    fn mkdir(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        if let Err(code) = self.require_writable() {
            reply.error(code);
            return;
        }
        let full = match self.child_path(parent, name) {
            Ok(v) => v,
            Err(code) => {
                reply.error(code);
                return;
            }
        };
        let id = match self.map_rpc(
            self.rt
                .block_on(self.client.create_directory(&full, None, false)),
        ) {
            Ok(id) => id,
            Err(code) => {
                reply.error(code);
                return;
            }
        };
        let ino = inode_from_entry_id(id);
        self.remember_path(ino, &full);
        let attr = file_attr(ino, EntryKind::Directory, 0, SystemTime::now());
        reply.entry(&TTL, &attr, 0);
    }

    fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        if let Err(code) = self.require_writable() {
            reply.error(code);
            return;
        }
        let full = match self.child_path(parent, name) {
            Ok(v) => v,
            Err(code) => {
                reply.error(code);
                return;
            }
        };
        match self.map_rpc(self.rt.block_on(self.client.delete_directory(&full, false))) {
            Ok(()) => reply.ok(),
            Err(code) => reply.error(code),
        }
    }

    fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        if let Err(code) = self.require_writable() {
            reply.error(code);
            return;
        }
        let full = match self.child_path(parent, name) {
            Ok(v) => v,
            Err(code) => {
                reply.error(code);
                return;
            }
        };
        match self.map_rpc(self.rt.block_on(self.client.delete_document(&full))) {
            Ok(()) => reply.ok(),
            Err(code) => reply.error(code),
        }
    }

    fn create(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        flags: i32,
        reply: fuser::ReplyCreate,
    ) {
        if let Err(code) = self.require_writable() {
            reply.error(code);
            return;
        }
        let full = match self.child_path(parent, name) {
            Ok(v) => v,
            Err(code) => {
                reply.error(code);
                return;
            }
        };
        if let Err(code) = self.map_rpc(self.rt.block_on(self.client.put_document_force(&full, ""))) {
            reply.error(code);
            return;
        }
        let stat = match self.map_rpc(self.rt.block_on(self.client.document_stat(&full))) {
            Ok(stat) => stat,
            Err(code) => {
                reply.error(code);
                return;
            }
        };
        let ino = inode_from_entry_id(stat.id());
        self.remember_path(ino, &full);
        let fh = match self.alloc_fh(FileHandleState {
            path: full,
            data: Vec::new(),
            dirty: false,
        }) {
            Ok(fh) => fh,
            Err(code) => {
                reply.error(code);
                return;
            }
        };
        let attr = file_attr(ino, EntryKind::File, 0, SystemTime::now());
        let _ = flags;
        reply.created(&TTL, &attr, 0, fh, 0);
    }

    fn mknod(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _rdev: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        if let Err(code) = self.require_writable() {
            reply.error(code);
            return;
        }
        let full = match self.child_path(parent, name) {
            Ok(v) => v,
            Err(code) => {
                reply.error(code);
                return;
            }
        };
        if let Err(code) = self.map_rpc(self.rt.block_on(self.client.put_document_force(&full, ""))) {
            reply.error(code);
            return;
        }
        let stat = match self.map_rpc(self.rt.block_on(self.client.document_stat(&full))) {
            Ok(stat) => stat,
            Err(code) => {
                reply.error(code);
                return;
            }
        };
        let ino = inode_from_entry_id(stat.id());
        self.remember_path(ino, &full);
        let attr = file_attr(ino, EntryKind::File, 0, SystemTime::now());
        reply.entry(&TTL, &attr, 0);
    }

    fn open(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        let Some(path) = self.path_for_inode(ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        let body = match self.map_rpc(self.rt.block_on(self.client.get_document(&path))) {
            Ok(v) => v,
            Err(code) => {
                reply.error(code);
                return;
            }
        };
        let mut state = FileHandleState {
            path,
            data: body.content().as_bytes().to_vec(),
            dirty: false,
        };
        if flags & libc::O_TRUNC != 0 {
            if let Err(code) = self.require_writable() {
                reply.error(code);
                return;
            }
            state.data.clear();
            state.dirty = true;
        }
        let fh = match self.alloc_fh(state) {
            Ok(v) => v,
            Err(code) => {
                reply.error(code);
                return;
            }
        };
        let _ = flags;
        reply.opened(fh, 0);
    }

    fn write(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        if let Err(code) = self.require_writable() {
            reply.error(code);
            return;
        }
        if offset < 0 {
            reply.error(libc::EINVAL);
            return;
        }
        let start = offset as usize;
        let Some(path) = self.path_for_inode(ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        let body = match self.map_rpc(self.rt.block_on(self.client.get_document(&path))) {
            Ok(v) => v,
            Err(code) => {
                reply.error(code);
                return;
            }
        };
        let mut bytes = body.content().as_bytes().to_vec();
        if start > bytes.len() {
            bytes.resize(start, 0);
        }
        let end = start.saturating_add(data.len());
        if end > bytes.len() {
            bytes.resize(end, 0);
        }
        bytes[start..end].copy_from_slice(data);
        let content = match String::from_utf8(bytes.clone()) {
            Ok(v) => v,
            Err(_) => {
                reply.error(libc::EINVAL);
                return;
            }
        };
        if let Err(code) = self.map_rpc(self.rt.block_on(self.client.put_document_force(&path, &content))) {
            reply.error(code);
            return;
        }
        if let Ok(mut map) = self.fh_state.lock()
            && let Some(state) = map.get_mut(&fh)
        {
            state.data = bytes;
            state.dirty = false;
        }
        reply.written(data.len() as u32);
    }

    fn flush(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        _lock_owner: u64,
        reply: ReplyEmpty,
    ) {
        match self.flush_fh(fh) {
            Ok(()) => reply.ok(),
            Err(code) => reply.error(code),
        }
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        if let Err(code) = self.flush_fh(fh) {
            reply.error(code);
            return;
        }
        if let Ok(mut map) = self.fh_state.lock() {
            map.remove(&fh);
        }
        reply.ok();
    }

    fn setattr(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        let Some(path) = self.path_for_inode(ino) else {
            reply.error(libc::ENOENT);
            return;
        };
        if let Some(size) = size {
            if let Err(code) = self.require_writable() {
                reply.error(code);
                return;
            }
            if let Err(code) = self.truncate_path(&path, size) {
                reply.error(code);
                return;
            }
        }
        match self.lookup_entry(&path) {
            Ok(meta) => {
                let attr = file_attr(ino, meta.kind, meta.size_bytes, meta.modified);
                reply.attr(&TTL, &attr);
            }
            Err(code) => reply.error(code),
        }
    }

    fn rename(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        _flags: u32,
        reply: ReplyEmpty,
    ) {
        if let Err(code) = self.require_writable() {
            reply.error(code);
            return;
        }
        let src = match self.child_path(parent, name) {
            Ok(v) => v,
            Err(code) => {
                reply.error(code);
                return;
            }
        };
        let dst = {
            let Some(parent_path) = self.path_for_inode(newparent) else {
                reply.error(libc::ENOENT);
                return;
            };
            let Some(new_name) = newname.to_str() else {
                reply.error(libc::EINVAL);
                return;
            };
            join_path(&parent_path, new_name)
        };

        let result = match self.lookup_entry(&src) {
            Ok(meta) if meta.kind == EntryKind::File => {
                self.map_rpc(self.rt.block_on(self.client.move_document(&src, &dst)))
            }
            Ok(meta) if meta.kind == EntryKind::Directory => {
                let (new_parent_path, new_leaf) = match split_parent_name(&dst) {
                    Ok(v) => v,
                    Err(code) => {
                        reply.error(code);
                        return;
                    }
                };
                self.map_rpc(
                    self.rt
                        .block_on(self.client.move_directory(&src, new_parent_path, new_leaf)),
                )
            }
            Ok(_) => Err(libc::EINVAL),
            Err(code) => {
                reply.error(code);
                return;
            }
        };
        match result {
            Ok(()) => reply.ok(),
            Err(code) => reply.error(code),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();
    let args = Args::parse();
    tracing::info!(
        api_uri = %args.api_uri,
        mountpoint = %args.mountpoint.display(),
        read_only = args.read_only,
        "tabularium-fuse starting"
    );

    if !args.mountpoint.exists() {
        tracing::error!(path = %args.mountpoint.display(), "mountpoint does not exist");
        return Err(format!("mountpoint does not exist: {}", args.mountpoint.display()).into());
    }
    if !args.mountpoint.is_dir() {
        tracing::error!(path = %args.mountpoint.display(), "mountpoint is not a directory");
        return Err(format!("mountpoint is not a directory: {}", args.mountpoint.display()).into());
    }

    let mountpoint_canon = args.mountpoint.canonicalize().unwrap_or_else(|_| args.mountpoint.clone());
    tracing::info!(resolved = %mountpoint_canon.display(), "mountpoint path");

    match statfs_fstype(&mountpoint_canon) {
        Ok(fs) => {
            tracing::info!(filesystem_type = %fs, path = %mountpoint_canon.display(), "statfs mountpoint");
            if fstype_suggests_fuse(&fs) {
                tracing::error!(
                    filesystem_type = %fs,
                    "macFUSE refuses to mount here: the mountpoint sits on a FUSE-backed volume (nested FUSE is not allowed). Use an empty directory on the main APFS volume, e.g. `mkdir -p \"$HOME/Library/Application Support/tabularium/mnt\"` and pass that path."
                );
                eprintln!(
                    "tabularium-fuse: mountpoint {} is on filesystem type '{}' (FUSE). Choose a directory on APFS/HFS+ (not inside another FUSE mount).",
                    mountpoint_canon.display(),
                    fs
                );
                std::process::exit(1);
            }
        }
        Err(e) => tracing::warn!(error = %e, "statfs failed; continuing without pre-check"),
    }

    let client = Client::init(args.api_uri, Duration::from_secs(RPC_TIMEOUT_SECS))?;
    let ping_rt = Runtime::new()?;
    match ping_rt.block_on(client.test()) {
        Ok(t) => tracing::info!(
            product = t.product_name(),
            version = t.product_version(),
            "rpc test ok"
        ),
        Err(e) => tracing::warn!(error = %e, "rpc test failed; mount may still fail"),
    }

    let fs = TbFs::new(client, args.read_only)?;
    let mut options = vec![MountOption::FSName("tabularium".to_string())];
    if args.read_only {
        options.push(MountOption::RO);
    }
    tracing::info!("calling fuser::mount2");
    if let Err(e) = fuser::mount2(fs, &args.mountpoint, &options) {
        tracing::error!(error = %e, "fuser::mount2 failed");
        eprintln!(
            "tabularium-fuse: mount failed: {e}. If you see 'mount point ... is itself on a macFUSE volume', move the mountpoint to a normal disk path (APFS). Set RUST_LOG=tabularium_fuse=debug for details."
        );
        return Err(e.into());
    }
    Ok(())
}
