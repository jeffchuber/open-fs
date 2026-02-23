use std::collections::HashSet;

use openfs_core::Entry;
use rand::seq::SliceRandom;
use rand::Rng;

/// Identifies which mount an operation targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountId {
    Work,
    Indexed,
    SharedRead,
    SharedWrite,
}

impl MountId {
    /// Return the VFS path prefix for this mount, given an agent id.
    pub fn prefix(&self, agent_id: usize) -> &'static str {
        match (self, agent_id) {
            (MountId::Work, 0) => "/a0/work",
            (MountId::Work, 1) => "/a1/work",
            (MountId::Work, 2) => "/a2/work",
            (MountId::Work, _) => "/a1/work",
            (MountId::Indexed, 0) => "/a0/indexed",
            (MountId::Indexed, 1) => "/a1/indexed",
            (MountId::Indexed, 2) => "/a2/indexed",
            (MountId::Indexed, _) => "/a1/indexed",
            (MountId::SharedRead, _) => "/shared/read",
            (MountId::SharedWrite, _) => "/shared/write",
        }
    }

    pub fn is_shared(&self) -> bool {
        matches!(self, MountId::SharedRead | MountId::SharedWrite)
    }
}

/// Minimal entry info for deterministic comparisons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntrySummary {
    pub name: String,
    pub is_dir: bool,
    pub size: Option<u64>,
}

impl EntrySummary {
    pub fn from_entry(entry: &Entry) -> Self {
        EntrySummary {
            name: entry.name.clone(),
            is_dir: entry.is_dir,
            size: entry.size,
        }
    }
}

/// An operation the simulation can perform.
#[derive(Debug, Clone)]
pub enum Op {
    Write {
        mount: MountId,
        path: String,
        content: Vec<u8>,
    },
    Read {
        mount: MountId,
        path: String,
    },
    Append {
        mount: MountId,
        path: String,
        content: Vec<u8>,
    },
    Delete {
        mount: MountId,
        path: String,
    },
    List {
        mount: MountId,
        path: String,
    },
    Stat {
        mount: MountId,
        path: String,
    },
    Exists {
        mount: MountId,
        path: String,
    },
    Rename {
        mount: MountId,
        from: String,
        to: String,
    },
    IndexFile {
        path: String,
    },
    SearchChroma {
        query: String,
    },
    FlushWriteBack,
}

/// State tracked per-agent for smart operation generation.
pub struct AgentOpState {
    pub agent_id: usize,
    /// Known files per mount: mount -> set of relative paths.
    pub known_files: [Vec<String>; 4], // Work, Indexed, SharedRead, SharedWrite
    /// Counter for generating unique file names.
    pub file_counter: usize,
    /// Files that have been indexed (for SearchChroma to make sense).
    pub indexed_files: Vec<String>,
}

impl AgentOpState {
    pub fn new(agent_id: usize) -> Self {
        AgentOpState {
            agent_id,
            known_files: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            file_counter: 0,
            indexed_files: Vec::new(),
        }
    }

    fn mount_index(mount: MountId) -> usize {
        match mount {
            MountId::Work => 0,
            MountId::Indexed => 1,
            MountId::SharedRead => 2,
            MountId::SharedWrite => 3,
        }
    }

    pub fn known_for(&self, mount: MountId) -> &[String] {
        &self.known_files[Self::mount_index(mount)]
    }

    pub fn add_file(&mut self, mount: MountId, path: String) {
        let idx = Self::mount_index(mount);
        if !self.known_files[idx].contains(&path) {
            self.known_files[idx].push(path);
        }
    }

    pub fn remove_file(&mut self, mount: MountId, path: &str) {
        let idx = Self::mount_index(mount);
        self.known_files[idx].retain(|p| p != path);
    }
}

/// Generate a random operation weighted by the plan's distribution.
pub fn generate<R: Rng>(rng: &mut R, state: &AgentOpState, step: usize) -> Op {
    // Weights: Write 20%, Read 18%, Append 8%, Delete 6%, List 7%, Exists 5%, Stat 5%, Rename 4%,
    //          IndexFile 10%, SearchChroma 5%, SharedWrite 5%, SharedRead 3%, ReadOnlyOps 2%, FlushWriteBack 2%
    let roll: u32 = rng.gen_range(0..100);

    match roll {
        0..=19 => {
            // Write — pick a writable mount, generate a new file
            let mount = pick_writable_mount(rng);
            let name = format!("a{}_file_{}.txt", state.agent_id, state.file_counter);
            let name = maybe_nested_path(rng, state, name);
            let content =
                format!("agent{}_step{}_{}", state.agent_id, step, rng.gen::<u32>()).into_bytes();
            Op::Write {
                mount,
                path: name,
                content,
            }
        }
        20..=37 => {
            // Read — pick an existing file from any mount
            if let Some((mount, path)) = pick_existing_file(rng, state) {
                Op::Read { mount, path }
            } else {
                // Fallback: read a nonexistent file from work mount
                Op::Read {
                    mount: MountId::Work,
                    path: "nonexistent.txt".to_string(),
                }
            }
        }
        38..=45 => {
            // Append — pick an existing writable file, or write a new one
            if let Some((mount, path)) = pick_existing_writable_file(rng, state) {
                let content = format!("_append_{}", rng.gen::<u16>()).into_bytes();
                Op::Append {
                    mount,
                    path,
                    content,
                }
            } else {
                let mount = pick_writable_mount(rng);
                let name = format!("a{}_file_{}.txt", state.agent_id, state.file_counter);
                let name = maybe_nested_path(rng, state, name);
                let content = format!("agent{}_step{}", state.agent_id, step).into_bytes();
                Op::Write {
                    mount,
                    path: name,
                    content,
                }
            }
        }
        46..=51 => {
            // Delete — pick an existing writable file
            if let Some((mount, path)) = pick_existing_writable_file(rng, state) {
                Op::Delete { mount, path }
            } else {
                // Nothing to delete, do a write instead
                let mount = pick_writable_mount(rng);
                let name = format!("a{}_file_{}.txt", state.agent_id, state.file_counter);
                let name = maybe_nested_path(rng, state, name);
                let content = format!("agent{}_step{}", state.agent_id, step).into_bytes();
                Op::Write {
                    mount,
                    path: name,
                    content,
                }
            }
        }
        52..=58 => {
            // List
            let mount = pick_any_mount(rng);
            let path = if rng.gen_bool(0.5) {
                String::new()
            } else {
                pick_existing_dir_for_mount(rng, state, mount).unwrap_or_default()
            };
            Op::List { mount, path }
        }
        59..=63 => {
            // Exists
            if rng.gen_bool(0.4) {
                if let Some((mount, path)) = pick_existing_file(rng, state) {
                    Op::Exists { mount, path }
                } else {
                    Op::Exists {
                        mount: MountId::Work,
                        path: "nonexistent.txt".to_string(),
                    }
                }
            } else if let Some((mount, path)) = pick_existing_dir(rng, state) {
                Op::Exists { mount, path }
            } else {
                Op::Exists {
                    mount: MountId::Work,
                    path: "nonexistent.txt".to_string(),
                }
            }
        }
        64..=68 => {
            // Stat
            if rng.gen_bool(0.5) {
                if let Some((mount, path)) = pick_existing_file(rng, state) {
                    Op::Stat { mount, path }
                } else {
                    Op::Stat {
                        mount: MountId::Work,
                        path: "nonexistent.txt".to_string(),
                    }
                }
            } else if let Some((mount, path)) = pick_existing_dir(rng, state) {
                Op::Stat { mount, path }
            } else {
                Op::Stat {
                    mount: MountId::Work,
                    path: "nonexistent.txt".to_string(),
                }
            }
        }
        69..=72 => {
            // Rename
            if let Some((mount, from)) = pick_existing_writable_file(rng, state) {
                let name = format!("a{}_file_{}.txt", state.agent_id, state.file_counter);
                let to = maybe_nested_path(rng, state, name);
                Op::Rename { mount, from, to }
            } else {
                Op::Rename {
                    mount: MountId::Work,
                    from: "nonexistent.txt".to_string(),
                    to: "still_nonexistent.txt".to_string(),
                }
            }
        }
        73..=82 => {
            // IndexFile — index a file from the indexed mount
            if let Some(path) = state.known_for(MountId::Indexed).choose(rng).cloned() {
                Op::IndexFile { path }
            } else {
                // Nothing to index, fallback to write on indexed mount
                let name = format!("a{}_file_{}.txt", state.agent_id, state.file_counter);
                let name = maybe_nested_path(rng, state, name);
                let content = format!("agent{}_step{}_{}", state.agent_id, step, rng.gen::<u32>())
                    .into_bytes();
                Op::Write {
                    mount: MountId::Indexed,
                    path: name,
                    content,
                }
            }
        }
        83..=87 => {
            // SearchChroma
            Op::SearchChroma {
                query: format!("agent{}_search_{}", state.agent_id, rng.gen::<u16>()),
            }
        }
        88..=92 => {
            // Write to shared/write
            let name = format!("a{}_shared_{}.txt", state.agent_id, state.file_counter);
            let name = maybe_nested_path(rng, state, name);
            let content =
                format!("agent{}_step{}_{}", state.agent_id, step, rng.gen::<u32>()).into_bytes();
            Op::Write {
                mount: MountId::SharedWrite,
                path: name,
                content,
            }
        }
        93..=95 => {
            // Read from shared/read
            if let Some(path) = state.known_for(MountId::SharedRead).choose(rng).cloned() {
                Op::Read {
                    mount: MountId::SharedRead,
                    path,
                }
            } else {
                Op::Read {
                    mount: MountId::SharedRead,
                    path: "nonexistent.txt".to_string(),
                }
            }
        }
        96..=97 => {
            // Read-only operations against shared/read
            let choice: u8 = rng.gen_range(0..4);
            match choice {
                0 => {
                    let name = format!("a{}_ro_{}.txt", state.agent_id, state.file_counter);
                    let name = maybe_nested_path(rng, state, name);
                    Op::Write {
                        mount: MountId::SharedRead,
                        path: name,
                        content: format!("ro_write_{}", rng.gen::<u32>()).into_bytes(),
                    }
                }
                1 => {
                    let target = state
                        .known_for(MountId::SharedRead)
                        .choose(rng)
                        .cloned()
                        .unwrap_or_else(|| "nonexistent.txt".to_string());
                    Op::Append {
                        mount: MountId::SharedRead,
                        path: target,
                        content: format!("ro_append_{}", rng.gen::<u16>()).into_bytes(),
                    }
                }
                2 => {
                    let target = state
                        .known_for(MountId::SharedRead)
                        .choose(rng)
                        .cloned()
                        .unwrap_or_else(|| "nonexistent.txt".to_string());
                    Op::Delete {
                        mount: MountId::SharedRead,
                        path: target,
                    }
                }
                _ => {
                    let from = state
                        .known_for(MountId::SharedRead)
                        .choose(rng)
                        .cloned()
                        .unwrap_or_else(|| "nonexistent.txt".to_string());
                    let to = format!("a{}_ro_{}.txt", state.agent_id, state.file_counter);
                    let to = maybe_nested_path(rng, state, to);
                    Op::Rename {
                        mount: MountId::SharedRead,
                        from,
                        to,
                    }
                }
            }
        }
        _ => {
            // FlushWriteBack
            Op::FlushWriteBack
        }
    }
}

fn pick_writable_mount<R: Rng>(rng: &mut R) -> MountId {
    let mounts = [MountId::Work, MountId::Indexed, MountId::SharedWrite];
    *mounts.choose(rng).unwrap()
}

fn pick_any_mount<R: Rng>(rng: &mut R) -> MountId {
    let mounts = [
        MountId::Work,
        MountId::Indexed,
        MountId::SharedRead,
        MountId::SharedWrite,
    ];
    *mounts.choose(rng).unwrap()
}

fn pick_existing_file<R: Rng>(rng: &mut R, state: &AgentOpState) -> Option<(MountId, String)> {
    let all_mounts = [
        MountId::Work,
        MountId::Indexed,
        MountId::SharedRead,
        MountId::SharedWrite,
    ];
    // Collect all (mount, path) pairs
    let mut candidates: Vec<(MountId, &String)> = Vec::new();
    for &m in &all_mounts {
        for p in state.known_for(m) {
            candidates.push((m, p));
        }
    }
    candidates.choose(rng).map(|(m, p)| (*m, (*p).clone()))
}

fn pick_existing_dir<R: Rng>(rng: &mut R, state: &AgentOpState) -> Option<(MountId, String)> {
    let all_mounts = [
        MountId::Work,
        MountId::Indexed,
        MountId::SharedRead,
        MountId::SharedWrite,
    ];

    let mut candidates: Vec<(MountId, String)> = Vec::new();
    for &m in &all_mounts {
        for dir in collect_dirs(state.known_for(m)) {
            candidates.push((m, dir));
        }
    }

    candidates.choose(rng).cloned()
}

fn pick_existing_dir_for_mount<R: Rng>(
    rng: &mut R,
    state: &AgentOpState,
    mount: MountId,
) -> Option<String> {
    let dirs = collect_dirs(state.known_for(mount));
    dirs.choose(rng).cloned()
}

fn pick_existing_writable_file<R: Rng>(
    rng: &mut R,
    state: &AgentOpState,
) -> Option<(MountId, String)> {
    let writable = [MountId::Work, MountId::Indexed, MountId::SharedWrite];
    let mut candidates: Vec<(MountId, &String)> = Vec::new();
    for &m in &writable {
        for p in state.known_for(m) {
            candidates.push((m, p));
        }
    }
    candidates.choose(rng).map(|(m, p)| (*m, (*p).clone()))
}

fn maybe_nested_path<R: Rng>(rng: &mut R, state: &AgentOpState, base: String) -> String {
    if rng.gen_bool(0.3) {
        let dir = format!("dir{}_{}", state.agent_id, state.file_counter % 3);
        format!("{}/{}", dir, base)
    } else {
        base
    }
}

fn collect_dirs(paths: &[String]) -> Vec<String> {
    let mut dirs = HashSet::new();
    for path in paths {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() <= 1 {
            continue;
        }
        let mut current = String::new();
        for (idx, part) in parts.iter().enumerate() {
            if idx == parts.len() - 1 {
                break;
            }
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(part);
            dirs.insert(current.clone());
        }
    }
    let mut out: Vec<String> = dirs.into_iter().collect();
    out.sort();
    out
}
