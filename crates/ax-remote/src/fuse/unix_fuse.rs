//! Unix FUSE implementation using the `fuser` crate.

use std::ffi::OsStr;
use std::time::SystemTime;

use fuser::{
    FileType, Filesystem, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty,
    ReplyEntry, ReplyOpen, ReplyWrite, Request, TimeOrNow,
};
use tracing::{debug, error};

use super::common::{inode_attr_to_file_attr, AxFsCore, FsOpError};
use super::inode::{InodeAttr, InodeKind, VIRTUAL_INO_BASE};

/// Unix FUSE filesystem wrapper around `AxFsCore`.
pub struct UnixFuse(pub AxFsCore);

impl UnixFuse {
    fn error_to_errno(e: &FsOpError) -> i32 {
        match e {
            FsOpError::NotFound => libc::ENOENT,
            FsOpError::ReadOnly => libc::EROFS,
            FsOpError::InvalidArg => libc::EINVAL,
            FsOpError::NotEmpty => libc::ENOTEMPTY,
            FsOpError::Io(_) => libc::EIO,
            FsOpError::NotSymlink => libc::EINVAL,
            FsOpError::IsDir => libc::EISDIR,
        }
    }
}

impl Filesystem for UnixFuse {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        let name_str = match name.to_str() {
            Some(n) => n,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        debug!("lookup: parent={}, name={}", parent, name_str);

        match self.0.do_lookup(parent, name_str) {
            Ok(attr) => {
                let file_attr = inode_attr_to_file_attr(&attr);
                reply.entry(&InodeAttr::ttl(), &file_attr, 0);
            }
            Err(e) => {
                debug!("lookup failed: {:?}", Self::error_to_errno(&e));
                reply.error(Self::error_to_errno(&e));
            }
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        debug!("getattr: ino={}", ino);

        match self.0.do_getattr(ino) {
            Ok(attr) => {
                let file_attr = inode_attr_to_file_attr(&attr);
                reply.attr(&InodeAttr::ttl(), &file_attr);
            }
            Err(e) => {
                reply.error(Self::error_to_errno(&e));
            }
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        debug!("read: ino={}, offset={}, size={}", ino, offset, size);

        match self.0.do_read(ino, offset, size) {
            Ok(data) => reply.data(&data),
            Err(e) => {
                error!("read failed: {:?}", Self::error_to_errno(&e));
                reply.error(Self::error_to_errno(&e));
            }
        }
    }

    fn write(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        debug!("write: ino={}, offset={}, size={}", ino, offset, data.len());

        match self.0.do_write(ino, offset, data) {
            Ok(written) => reply.written(written),
            Err(e) => {
                error!("write failed: {:?}", Self::error_to_errno(&e));
                reply.error(Self::error_to_errno(&e));
            }
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        debug!("readdir: ino={}, offset={}", ino, offset);

        match self.0.do_readdir(ino) {
            Ok(result) => {
                let mut i = offset as usize;

                // Add . and ..
                if i == 0 {
                    let _ = reply.add(result.ino, 1, FileType::Directory, ".");
                    i += 1;
                }
                if i == 1 {
                    let _ = reply.add(result.parent_ino, 2, FileType::Directory, "..");
                    i += 1;
                }

                // Add .search to root directory listing
                if result.is_root && i == 2 {
                    let search_ino = VIRTUAL_INO_BASE;
                    if !reply.add(search_ino, 3, FileType::Directory, ".search") {
                        i += 1;
                    } else {
                        reply.ok();
                        return;
                    }
                }

                let skip = i.saturating_sub(if result.is_root { 3 } else { 2 });
                for entry in result.entries.into_iter().skip(skip) {
                    let ft = match entry.kind {
                        InodeKind::File => FileType::RegularFile,
                        InodeKind::Directory => FileType::Directory,
                        InodeKind::Symlink => FileType::Symlink,
                    };
                    i += 1;
                    if reply.add(entry.ino, i as i64, ft, &entry.name) {
                        break;
                    }
                }
                reply.ok();
            }
            Err(e) => {
                error!("readdir failed: {:?}", Self::error_to_errno(&e));
                reply.error(Self::error_to_errno(&e));
            }
        }
    }

    fn create(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        let name_str = match name.to_str() {
            Some(n) => n,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        debug!("create: parent={}, name={}", parent, name_str);

        match self.0.do_create(parent, name_str) {
            Ok(attr) => {
                let file_attr = inode_attr_to_file_attr(&attr);
                reply.created(&InodeAttr::ttl(), &file_attr, 0, 0, 0);
            }
            Err(e) => {
                error!("create failed: {:?}", Self::error_to_errno(&e));
                reply.error(Self::error_to_errno(&e));
            }
        }
    }

    fn mkdir(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        let name_str = match name.to_str() {
            Some(n) => n,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        debug!("mkdir: parent={}, name={}", parent, name_str);

        match self.0.do_mkdir(parent, name_str) {
            Ok(attr) => {
                let file_attr = inode_attr_to_file_attr(&attr);
                reply.entry(&InodeAttr::ttl(), &file_attr, 0);
            }
            Err(e) => {
                error!("mkdir failed: {:?}", Self::error_to_errno(&e));
                reply.error(Self::error_to_errno(&e));
            }
        }
    }

    fn unlink(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let name_str = match name.to_str() {
            Some(n) => n,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        debug!("unlink: parent={}, name={}", parent, name_str);

        match self.0.do_unlink(parent, name_str) {
            Ok(()) => reply.ok(),
            Err(e) => {
                error!("unlink failed: {:?}", Self::error_to_errno(&e));
                reply.error(Self::error_to_errno(&e));
            }
        }
    }

    fn rmdir(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let name_str = match name.to_str() {
            Some(n) => n,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        debug!("rmdir: parent={}, name={}", parent, name_str);

        match self.0.do_rmdir(parent, name_str) {
            Ok(()) => reply.ok(),
            Err(e) => {
                error!("rmdir failed: {:?}", Self::error_to_errno(&e));
                reply.error(Self::error_to_errno(&e));
            }
        }
    }

    fn readlink(&mut self, _req: &Request, ino: u64, reply: ReplyData) {
        debug!("readlink: ino={}", ino);

        match self.0.do_readlink(ino) {
            Ok(target) => reply.data(target.as_bytes()),
            Err(e) => reply.error(Self::error_to_errno(&e)),
        }
    }

    fn open(&mut self, _req: &Request, ino: u64, _flags: i32, reply: ReplyOpen) {
        debug!("open: ino={}", ino);

        match self.0.do_access(ino) {
            Ok(()) => reply.opened(0, 0),
            Err(e) => reply.error(Self::error_to_errno(&e)),
        }
    }

    fn opendir(&mut self, _req: &Request, ino: u64, _flags: i32, reply: ReplyOpen) {
        debug!("opendir: ino={}", ino);

        if self.0.get_path(ino).is_some() || ino >= VIRTUAL_INO_BASE {
            reply.opened(0, 0);
        } else {
            reply.error(libc::ENOENT);
        }
    }

    fn setattr(
        &mut self,
        _req: &Request,
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
        debug!("setattr: ino={}, size={:?}", ino, size);

        match self.0.do_setattr(ino, size) {
            Ok(attr) => {
                let file_attr = inode_attr_to_file_attr(&attr);
                reply.attr(&InodeAttr::ttl(), &file_attr);
            }
            Err(e) => reply.error(Self::error_to_errno(&e)),
        }
    }

    fn rename(
        &mut self,
        _req: &Request,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
        _flags: u32,
        reply: ReplyEmpty,
    ) {
        let name_str = match name.to_str() {
            Some(n) => n,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };
        let newname_str = match newname.to_str() {
            Some(n) => n,
            None => {
                reply.error(libc::EINVAL);
                return;
            }
        };

        debug!(
            "rename: parent={}, name={}, newparent={}, newname={}",
            parent, name_str, newparent, newname_str
        );

        match self.0.do_rename(parent, name_str, newparent, newname_str) {
            Ok(()) => reply.ok(),
            Err(e) => {
                error!("rename failed: {:?}", Self::error_to_errno(&e));
                reply.error(Self::error_to_errno(&e));
            }
        }
    }

    fn statfs(&mut self, _req: &Request, _ino: u64, reply: fuser::ReplyStatfs) {
        reply.statfs(
            1_000_000, 500_000, 500_000, 1_000_000, 500_000, 4096, 255, 4096,
        );
    }

    fn access(&mut self, _req: &Request, ino: u64, _mask: i32, reply: ReplyEmpty) {
        match self.0.do_access(ino) {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(Self::error_to_errno(&e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_attr_conversion() {
        let attr = InodeAttr::file(42, 1024);
        let file_attr = inode_attr_to_file_attr(&attr);

        assert_eq!(file_attr.ino, 42);
        assert_eq!(file_attr.size, 1024);
        assert_eq!(file_attr.kind, FileType::RegularFile);
    }

    #[test]
    fn test_dir_attr_conversion() {
        let attr = InodeAttr::directory(1);
        let file_attr = inode_attr_to_file_attr(&attr);

        assert_eq!(file_attr.ino, 1);
        assert_eq!(file_attr.kind, FileType::Directory);
    }

    #[test]
    fn test_symlink_attr_conversion() {
        let attr = InodeAttr::symlink(10, 50);
        let file_attr = inode_attr_to_file_attr(&attr);

        assert_eq!(file_attr.ino, 10);
        assert_eq!(file_attr.size, 50);
        assert_eq!(file_attr.kind, FileType::Symlink);
    }

    #[test]
    fn test_file_attr_preserves_all_fields() {
        let attr = InodeAttr::file(42, 1024);
        let file_attr = inode_attr_to_file_attr(&attr);

        assert_eq!(file_attr.ino, attr.ino);
        assert_eq!(file_attr.size, attr.size);
        assert_eq!(file_attr.blocks, attr.blocks);
        assert_eq!(file_attr.atime, attr.atime);
        assert_eq!(file_attr.mtime, attr.mtime);
        assert_eq!(file_attr.ctime, attr.ctime);
        assert_eq!(file_attr.crtime, attr.crtime);
        assert_eq!(file_attr.perm, attr.perm);
        assert_eq!(file_attr.nlink, attr.nlink);
        assert_eq!(file_attr.uid, attr.uid);
        assert_eq!(file_attr.gid, attr.gid);
        assert_eq!(file_attr.rdev, 0);
        assert_eq!(file_attr.blksize, 4096);
        assert_eq!(file_attr.flags, 0);
    }

    #[test]
    fn test_file_attr_kind_mapping() {
        let attr = InodeAttr::file(1, 100);
        assert_eq!(inode_attr_to_file_attr(&attr).kind, FileType::RegularFile);

        let attr = InodeAttr::directory(2);
        assert_eq!(inode_attr_to_file_attr(&attr).kind, FileType::Directory);

        let attr = InodeAttr::symlink(3, 10);
        assert_eq!(inode_attr_to_file_attr(&attr).kind, FileType::Symlink);
    }

    #[test]
    fn test_error_to_errno_mapping() {
        assert_eq!(UnixFuse::error_to_errno(&FsOpError::NotFound), libc::ENOENT);
        assert_eq!(UnixFuse::error_to_errno(&FsOpError::ReadOnly), libc::EROFS);
        assert_eq!(UnixFuse::error_to_errno(&FsOpError::InvalidArg), libc::EINVAL);
        assert_eq!(UnixFuse::error_to_errno(&FsOpError::NotEmpty), libc::ENOTEMPTY);
        assert_eq!(UnixFuse::error_to_errno(&FsOpError::Io("test".to_string())), libc::EIO);
        assert_eq!(UnixFuse::error_to_errno(&FsOpError::NotSymlink), libc::EINVAL);
        assert_eq!(UnixFuse::error_to_errno(&FsOpError::IsDir), libc::EISDIR);
    }
}
