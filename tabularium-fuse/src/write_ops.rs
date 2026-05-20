use std::ffi::OsStr;

use crate::FileHandleState;
use crate::TbFs;
use crate::join_path;

impl TbFs {
    pub(crate) fn child_path(&self, parent: u64, name: &OsStr) -> Result<String, i32> {
        let parent_path = self.path_for_inode(parent).ok_or(libc::ENOENT)?;
        let name = name.to_str().ok_or(libc::EINVAL)?;
        Ok(join_path(&parent_path, name))
    }

    pub(crate) fn require_writable(&self) -> Result<(), i32> {
        if self.read_only {
            Err(libc::EROFS)
        } else {
            Ok(())
        }
    }

    pub(crate) fn map_rpc<T>(&self, result: tabularium::Result<T>) -> Result<T, i32> {
        result.map_err(|e| {
            tracing::debug!(error = %e, "rpc operation failed");
            Self::map_error(&e)
        })
    }

    pub(crate) fn alloc_fh(&self, state: FileHandleState) -> Result<u64, i32> {
        let mut next = self.next_fh.lock().map_err(|_| libc::EIO)?;
        let fh = *next;
        *next = next.saturating_add(1);
        let mut map = self.fh_state.lock().map_err(|_| libc::EIO)?;
        map.insert(fh, state);
        Ok(fh)
    }

    pub(crate) fn flush_fh(&self, fh: u64) -> Result<(), i32> {
        let mut map = self.fh_state.lock().map_err(|_| libc::EIO)?;
        let Some(state) = map.get_mut(&fh) else {
            return Ok(());
        };
        if !state.dirty {
            return Ok(());
        }
        let content = String::from_utf8(state.data.clone()).map_err(|_| libc::EINVAL)?;
        self.map_rpc(
            self.rt
                .block_on(self.client.put_document_force(&state.path, &content)),
        )?;
        state.dirty = false;
        Ok(())
    }

    pub(crate) fn truncate_path(&self, path: &str, size: u64) -> Result<(), i32> {
        let body = self.map_rpc(self.rt.block_on(self.client.get_document(path)))?;
        let mut data = body.content().as_bytes().to_vec();
        data.resize(size as usize, 0);
        let content = String::from_utf8(data).map_err(|_| libc::EINVAL)?;
        self.map_rpc(
            self.rt
                .block_on(self.client.put_document_force(path, &content)),
        )?;
        Ok(())
    }
}
