use async_trait::async_trait;
use nfsserve::{
    nfs::{fattr3, fileid3, filename3, ftype3, nfspath3, nfsstat3, nfstime3, sattr3, specdata3},
    vfs::{DirEntry, NFSFileSystem, ReadDirResult, VFSCapabilities},
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::info;

use crate::db::{CatalogDb, SearchHit};
use crate::embedder::MultimodalEmbedder;
use crate::parser::ParsedQuery;

const ROOT_ID: fileid3 = 1;

struct VirtualState {
    next_id: fileid3,
    query_dirs: HashMap<fileid3, String>,
    query_to_id: HashMap<String, fileid3>,
    symlinks: HashMap<fileid3, (fileid3, SearchHit, PathBuf)>,
    dir_entries: HashMap<fileid3, Vec<(fileid3, String)>>,
}

pub struct PhotoSearchVfs {
    #[allow(dead_code)]
    drive_root: PathBuf,
    db_path: PathBuf,
    embedder: Arc<dyn MultimodalEmbedder>,
    state: Mutex<VirtualState>,
}

impl PhotoSearchVfs {
    pub fn new(drive_root: PathBuf, db_path: PathBuf, embedder: Arc<dyn MultimodalEmbedder>) -> Self {
        let query_dirs = HashMap::new();
        let query_to_id = HashMap::new();

        Self {
            drive_root,
            db_path,
            embedder,
            state: Mutex::new(VirtualState {
                next_id: 2,
                query_dirs,
                query_to_id,
                symlinks: HashMap::new(),
                dir_entries: HashMap::new(),
            }),
        }
    }

    fn make_attr(id: fileid3, ftype: ftype3, size: u64) -> fattr3 {
        let mode = match ftype {
            ftype3::NF3DIR => 0o755,
            ftype3::NF3LNK => 0o777,
            _ => 0o644,
        };
        fattr3 {
            ftype,
            mode,
            nlink: 1,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            size,
            used: size,
            rdev: specdata3::default(),
            fsid: 1,
            fileid: id,
            atime: nfstime3::default(),
            mtime: nfstime3::default(),
            ctime: nfstime3::default(),
        }
    }

    fn execute_and_populate(&self, dir_id: fileid3, query_str: &str) -> Vec<(fileid3, String)> {
        {
            let st = self.state.lock().unwrap();
            if let Some(children) = st.dir_entries.get(&dir_id) {
                return children.clone();
            }
        }

        let parsed = ParsedQuery::parse(query_str);
        let Ok(q_vec) = self.embedder.embed_text(&parsed.semantic_prompt) else {
            return vec![];
        };
        let Ok(db) = CatalogDb::open_readonly(&self.db_path) else {
            return vec![];
        };
        let Ok(hits) = db.search_hybrid(&q_vec, &parsed, 50) else {
            return vec![];
        };

        let mut st = self.state.lock().unwrap();
        if let Some(children) = st.dir_entries.get(&dir_id) {
            return children.clone();
        }
        let mut children = Vec::new();

        for (rank, hit) in hits.into_iter().enumerate() {
            let link_id = st.next_id;
            st.next_id += 1;

            let orig_name = Path::new(&hit.rel_path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("photo.jpg");

            let score = ((1.0 - hit.distance).clamp(0.0, 1.0) * 100.0) as u32;
            let entry_name = format!("{:02}_{}pct_{}", rank + 1, score, orig_name);
            // Search/<query>/<link> → ../../<rel_path>, so the link survives a remount.
            let target = PathBuf::from(format!("../../{}", hit.rel_path));

            st.symlinks.insert(link_id, (dir_id, hit, target));
            children.push((link_id, entry_name));
        }

        st.dir_entries.insert(dir_id, children.clone());
        info!(
            "Virtual Search Folder '{}' materialized {} results",
            query_str,
            children.len()
        );
        children
    }
}

#[async_trait]
impl NFSFileSystem for PhotoSearchVfs {
    fn root_dir(&self) -> fileid3 {
        ROOT_ID
    }

    fn capabilities(&self) -> VFSCapabilities {
        VFSCapabilities::ReadOnly
    }

    async fn lookup(&self, dirid: fileid3, filename: &filename3) -> Result<fileid3, nfsstat3> {
        let name = String::from_utf8_lossy(&filename.0).to_string();
        if name.starts_with('.') {
            return Err(nfsstat3::NFS3ERR_NOENT);
        }

        if dirid == ROOT_ID {
            let dir_id = {
                let mut st = self.state.lock().unwrap();
                if let Some(&existing_id) = st.query_to_id.get(&name) {
                    existing_id
                } else {
                    let id = st.next_id;
                    st.next_id += 1;
                    st.query_dirs.insert(id, name.clone());
                    st.query_to_id.insert(name.clone(), id);
                    id
                }
            };
            self.execute_and_populate(dir_id, &name);
            return Ok(dir_id);
        }

        let st = self.state.lock().unwrap();
        if let Some(children) = st.dir_entries.get(&dirid) {
            for (cid, cname) in children {
                if cname == &name {
                    return Ok(*cid);
                }
            }
        }
        Err(nfsstat3::NFS3ERR_NOENT)
    }

    async fn getattr(&self, id: fileid3) -> Result<fattr3, nfsstat3> {
        if id == ROOT_ID {
            return Ok(Self::make_attr(ROOT_ID, ftype3::NF3DIR, 4096));
        }
        let st = self.state.lock().unwrap();
        if st.query_dirs.contains_key(&id) {
            return Ok(Self::make_attr(id, ftype3::NF3DIR, 4096));
        }
        if let Some((_, _, target)) = st.symlinks.get(&id) {
            let len = target.to_string_lossy().len() as u64;
            return Ok(Self::make_attr(id, ftype3::NF3LNK, len));
        }
        Err(nfsstat3::NFS3ERR_NOENT)
    }

    async fn readdir(
        &self,
        dirid: fileid3,
        start_after: fileid3,
        max_entries: usize,
    ) -> Result<ReadDirResult, nfsstat3> {
        let mut result = ReadDirResult {
            entries: Vec::new(),
            end: true,
        };

        if dirid == ROOT_ID {
            let st = self.state.lock().unwrap();
            for (&qid, qname) in &st.query_dirs {
                if qid > start_after && result.entries.len() < max_entries {
                    result.entries.push(DirEntry {
                        fileid: qid,
                        name: qname.as_bytes().into(),
                        attr: Self::make_attr(qid, ftype3::NF3DIR, 4096),
                    });
                }
            }
            return Ok(result);
        }

        let query_str = {
            let st = self.state.lock().unwrap();
            st.query_dirs.get(&dirid).cloned()
        };

        if let Some(qstr) = query_str {
            let children = self.execute_and_populate(dirid, &qstr);
            let st = self.state.lock().unwrap();
            for (cid, cname) in children {
                if cid > start_after && result.entries.len() < max_entries {
                    if let Some((_, _, target)) = st.symlinks.get(&cid) {
                        let len = target.to_string_lossy().len() as u64;
                        result.entries.push(DirEntry {
                            fileid: cid,
                            name: cname.as_bytes().into(),
                            attr: Self::make_attr(cid, ftype3::NF3LNK, len),
                        });
                    }
                }
            }
            return Ok(result);
        }

        Err(nfsstat3::NFS3ERR_NOENT)
    }

    async fn readlink(&self, id: fileid3) -> Result<nfspath3, nfsstat3> {
        let st = self.state.lock().unwrap();
        if let Some((_, _, target_abs)) = st.symlinks.get(&id) {
            return Ok(target_abs.to_string_lossy().as_bytes().into());
        }
        Err(nfsstat3::NFS3ERR_NOENT)
    }

    async fn setattr(&self, _id: fileid3, _setattr: sattr3) -> Result<fattr3, nfsstat3> {
        Err(nfsstat3::NFS3ERR_ROFS)
    }
    async fn read(&self, _id: fileid3, _offset: u64, _count: u32) -> Result<(Vec<u8>, bool), nfsstat3> {
        Err(nfsstat3::NFS3ERR_ISDIR)
    }
    async fn write(&self, _id: fileid3, _offset: u64, _data: &[u8]) -> Result<fattr3, nfsstat3> {
        Err(nfsstat3::NFS3ERR_ROFS)
    }
    async fn create(&self, _dirid: fileid3, _filename: &filename3, _attr: sattr3) -> Result<(fileid3, fattr3), nfsstat3> {
        Err(nfsstat3::NFS3ERR_ROFS)
    }
    async fn create_exclusive(&self, _dirid: fileid3, _filename: &filename3) -> Result<fileid3, nfsstat3> {
        Err(nfsstat3::NFS3ERR_ROFS)
    }
    async fn mkdir(&self, _dirid: fileid3, _dirname: &filename3) -> Result<(fileid3, fattr3), nfsstat3> {
        Err(nfsstat3::NFS3ERR_ROFS)
    }
    async fn remove(&self, _dirid: fileid3, _filename: &filename3) -> Result<(), nfsstat3> {
        Err(nfsstat3::NFS3ERR_ROFS)
    }
    async fn rename(&self, _from_dirid: fileid3, _from_filename: &filename3, _to_dirid: fileid3, _to_filename: &filename3) -> Result<(), nfsstat3> {
        Err(nfsstat3::NFS3ERR_ROFS)
    }
    async fn symlink(&self, _dirid: fileid3, _linkname: &filename3, _symlink: &nfspath3, _attr: &sattr3) -> Result<(fileid3, fattr3), nfsstat3> {
        Err(nfsstat3::NFS3ERR_ROFS)
    }
}
