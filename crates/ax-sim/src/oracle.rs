use std::collections::{HashMap, HashSet};

use crate::ops::{EntrySummary, MountId, Op};

/// What the real system should produce for a given operation.
#[derive(Debug, Clone)]
pub enum Expected {
    /// Operation should succeed with Ok(())
    Ok,
    /// Read should return this content
    ReadOk(Vec<u8>),
    /// Read from shared write mount — content may differ due to per-agent caching.
    /// We only verify the read succeeds (not NotFound) but don't compare content.
    SharedWriteOk,
    /// Operation should fail because mount is read-only
    ReadOnly,
    /// Read/delete should fail because file doesn't exist
    NotFound,
    /// Exists should return this value
    ExistsOk(bool),
    /// List should return these entries
    ListOk(Vec<EntrySummary>),
    /// Stat should return this entry
    StatOk(EntrySummary),
    /// Indexing result (just check success)
    IndexOk,
    /// Search result (just check no error)
    SearchOk,
    /// Flush is a no-op in our sim (no background sync)
    FlushOk,
}

/// Ground truth model that tracks what the system state should be.
pub struct Oracle {
    /// Per-agent private file state: [agent_id] -> { relative_path -> content }
    /// Index 0 = agent 0's work files, index 1 = agent 0's indexed files
    /// Index 2 = agent 1's work files, index 3 = agent 1's indexed files
    agent_files: Vec<HashMap<String, Vec<u8>>>,

    /// Shared read-only state (pre-populated, immutable after init).
    shared_read: HashMap<String, Vec<u8>>,

    /// Shared writable state (both agents can write here).
    shared_write: HashMap<String, Vec<u8>>,

    /// Last writer for shared write files.
    shared_write_last_writer: HashMap<String, usize>,

    /// Which files have been indexed: (agent_id, source_path).
    pub indexed: HashSet<(usize, String)>,
}

impl Default for Oracle {
    fn default() -> Self {
        Self::new()
    }
}

impl Oracle {
    pub fn new() -> Self {
        Oracle {
            agent_files: vec![HashMap::new(); 4],
            shared_read: HashMap::new(),
            shared_write: HashMap::new(),
            shared_write_last_writer: HashMap::new(),
            indexed: HashSet::new(),
        }
    }

    /// Pre-populate the shared read-only mount with seed data.
    pub fn seed_shared_read(&mut self, files: HashMap<String, Vec<u8>>) {
        self.shared_read = files;
    }

    fn private_index(agent_id: usize, mount: MountId) -> usize {
        match (agent_id, mount) {
            (0, MountId::Work) => 0,
            (0, MountId::Indexed) => 1,
            (_, MountId::Work) => 2,
            (_, MountId::Indexed) => 3,
            _ => panic!("private_index called with shared mount"),
        }
    }

    /// Apply an operation to the oracle model and return the expected outcome.
    /// This is a convenience wrapper that calls predict() then commit().
    pub fn apply(&mut self, agent_id: usize, op: &Op) -> Expected {
        let expected = self.predict(agent_id, op);
        self.commit(agent_id, op);
        expected
    }

    /// Read-only prediction: returns what the system should produce without mutating oracle state.
    pub fn predict(&self, agent_id: usize, op: &Op) -> Expected {
        match op {
            Op::Write {
                mount,
                path: _,
                content: _,
            } => {
                if *mount == MountId::SharedRead {
                    return Expected::ReadOnly;
                }
                Expected::Ok
            }

            Op::Read { mount, path } => {
                if *mount == MountId::SharedRead {
                    if let Some(data) = self.shared_read.get(path) {
                        return Expected::ReadOk(data.clone());
                    }
                    return Expected::NotFound;
                }
                if *mount == MountId::SharedWrite {
                    if self.shared_write.contains_key(path) {
                        return Expected::SharedWriteOk;
                    }
                    return Expected::NotFound;
                }
                let idx = Self::private_index(agent_id, *mount);
                if let Some(data) = self.agent_files[idx].get(path) {
                    Expected::ReadOk(data.clone())
                } else {
                    Expected::NotFound
                }
            }

            Op::Append {
                mount,
                path: _,
                content: _,
            } => {
                if *mount == MountId::SharedRead {
                    return Expected::ReadOnly;
                }
                Expected::Ok
            }

            Op::Delete { mount, path } => {
                if *mount == MountId::SharedRead {
                    return Expected::ReadOnly;
                }
                if mount.is_shared() {
                    if self.shared_write.contains_key(path) {
                        return Expected::Ok;
                    }
                    return Expected::NotFound;
                }
                let idx = Self::private_index(agent_id, *mount);
                if self.agent_files[idx].contains_key(path) {
                    Expected::Ok
                } else {
                    Expected::NotFound
                }
            }

            Op::List { mount, path } => {
                let entries = match mount {
                    MountId::SharedRead => list_entries(&self.shared_read, path),
                    MountId::SharedWrite => list_entries(&self.shared_write, path),
                    _ => {
                        let idx = Self::private_index(agent_id, *mount);
                        list_entries(&self.agent_files[idx], path)
                    }
                };
                Expected::ListOk(entries)
            }

            Op::Exists { mount, path } => {
                let exists = match mount {
                    MountId::SharedRead => exists_path(&self.shared_read, path),
                    MountId::SharedWrite => exists_path(&self.shared_write, path),
                    _ => {
                        let idx = Self::private_index(agent_id, *mount);
                        exists_path(&self.agent_files[idx], path)
                    }
                };
                Expected::ExistsOk(exists)
            }

            Op::Stat { mount, path } => {
                let entry = match mount {
                    MountId::SharedRead => stat_entry(&self.shared_read, path),
                    MountId::SharedWrite => stat_entry(&self.shared_write, path),
                    _ => {
                        let idx = Self::private_index(agent_id, *mount);
                        stat_entry(&self.agent_files[idx], path)
                    }
                };
                entry.map(Expected::StatOk).unwrap_or(Expected::NotFound)
            }

            Op::Rename { mount, from, .. } => {
                if *mount == MountId::SharedRead {
                    return Expected::ReadOnly;
                }
                if mount.is_shared() {
                    if self.shared_write.contains_key(from) {
                        return Expected::Ok;
                    }
                    return Expected::NotFound;
                }
                let idx = Self::private_index(agent_id, *mount);
                if self.agent_files[idx].contains_key(from) {
                    Expected::Ok
                } else {
                    Expected::NotFound
                }
            }

            Op::IndexFile { path } => {
                let idx = Self::private_index(agent_id, MountId::Indexed);
                if self.agent_files[idx].contains_key(path) {
                    Expected::IndexOk
                } else {
                    Expected::NotFound
                }
            }

            Op::SearchChroma { .. } => Expected::SearchOk,

            Op::FlushWriteBack => Expected::FlushOk,
        }
    }

    /// Commit the state mutation for an operation. Call after predict() when the
    /// operation actually succeeded (was not intercepted by fault injection, etc).
    pub fn commit(&mut self, agent_id: usize, op: &Op) {
        match op {
            Op::Write {
                mount,
                path,
                content,
            } => {
                if *mount == MountId::SharedRead {
                    return;
                }
                if mount.is_shared() {
                    self.shared_write.insert(path.clone(), content.clone());
                    self.shared_write_last_writer
                        .insert(path.clone(), agent_id);
                    return;
                }
                let idx = Self::private_index(agent_id, *mount);
                self.agent_files[idx].insert(path.clone(), content.clone());
            }

            Op::Append {
                mount,
                path,
                content,
            } => {
                if *mount == MountId::SharedRead {
                    return;
                }
                if mount.is_shared() {
                    let entry = self.shared_write.entry(path.clone()).or_default();
                    entry.extend_from_slice(content);
                    self.shared_write_last_writer
                        .insert(path.clone(), agent_id);
                    return;
                }
                let idx = Self::private_index(agent_id, *mount);
                let entry = self.agent_files[idx].entry(path.clone()).or_default();
                entry.extend_from_slice(content);
            }

            Op::Delete { mount, path } => {
                if *mount == MountId::SharedRead {
                    return;
                }
                if mount.is_shared() {
                    self.shared_write.remove(path);
                    self.shared_write_last_writer.remove(path);
                    return;
                }
                let idx = Self::private_index(agent_id, *mount);
                self.agent_files[idx].remove(path);
            }

            Op::Rename { mount, from, to } => {
                if *mount == MountId::SharedRead {
                    return;
                }
                if mount.is_shared() {
                    if let Some(content) = self.shared_write.remove(from) {
                        self.shared_write.insert(to.clone(), content);
                        if let Some(writer) = self.shared_write_last_writer.remove(from) {
                            self.shared_write_last_writer.insert(to.clone(), writer);
                        }
                    }
                    return;
                }
                let idx = Self::private_index(agent_id, *mount);
                if let Some(content) = self.agent_files[idx].remove(from) {
                    self.agent_files[idx].insert(to.clone(), content);
                }
            }

            Op::IndexFile { path } => {
                let idx = Self::private_index(agent_id, MountId::Indexed);
                if self.agent_files[idx].contains_key(path) {
                    self.indexed.insert((agent_id, path.clone()));
                }
            }

            // Read-only operations and no-ops: no state change.
            Op::Read { .. }
            | Op::List { .. }
            | Op::Exists { .. }
            | Op::Stat { .. }
            | Op::SearchChroma { .. }
            | Op::FlushWriteBack => {}
        }
    }

    /// After concurrent shared-write operations, update the oracle to match the
    /// observed winner. The winner's content is set; the loser's write is discarded.
    pub fn commit_shared_write_winner(
        &mut self,
        path: &str,
        winner_agent_id: usize,
        winner_content: Vec<u8>,
    ) {
        self.shared_write.insert(path.to_string(), winner_content);
        self.shared_write_last_writer
            .insert(path.to_string(), winner_agent_id);
    }

    /// Get oracle's view of all files for a given agent and mount.
    pub fn files_for(&self, agent_id: usize, mount: MountId) -> &HashMap<String, Vec<u8>> {
        match mount {
            MountId::SharedRead => &self.shared_read,
            MountId::SharedWrite => &self.shared_write,
            _ => {
                let idx = Self::private_index(agent_id, mount);
                &self.agent_files[idx]
            }
        }
    }

    /// Get the shared write map (for cross-agent convergence checks).
    pub fn shared_write_files(&self) -> &HashMap<String, Vec<u8>> {
        &self.shared_write
    }

    /// Last writer for each shared write path.
    pub fn shared_write_last_writers(&self) -> &HashMap<String, usize> {
        &self.shared_write_last_writer
    }
}

fn normalize_rel(path: &str) -> String {
    path.trim_matches('/').to_string()
}

fn list_entries(files: &HashMap<String, Vec<u8>>, path: &str) -> Vec<EntrySummary> {
    let normalized = normalize_rel(path);
    let prefix = if normalized.is_empty() {
        String::new()
    } else {
        format!("{}/", normalized)
    };

    let mut entries: HashMap<String, EntrySummary> = HashMap::new();

    for (file_path, content) in files.iter() {
        let relative = if prefix.is_empty() {
            file_path.as_str()
        } else if file_path.starts_with(&prefix) {
            &file_path[prefix.len()..]
        } else {
            continue;
        };

        if relative.is_empty() {
            continue;
        }

        let first_component = relative.split('/').next().unwrap();
        if relative.contains('/') {
            entries.entry(first_component.to_string()).or_insert_with(|| EntrySummary {
                name: first_component.to_string(),
                is_dir: true,
                size: None,
            });
        } else {
            entries.insert(
                first_component.to_string(),
                EntrySummary {
                    name: first_component.to_string(),
                    is_dir: false,
                    size: Some(content.len() as u64),
                },
            );
        }
    }

    let mut out: Vec<EntrySummary> = entries.into_values().collect();
    out.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });
    out
}

fn exists_path(files: &HashMap<String, Vec<u8>>, path: &str) -> bool {
    let normalized = normalize_rel(path);
    if normalized.is_empty() {
        return false;
    }
    if files.contains_key(&normalized) {
        return true;
    }
    let dir_prefix = format!("{}/", normalized);
    files.keys().any(|k| k.starts_with(&dir_prefix))
}

fn stat_entry(files: &HashMap<String, Vec<u8>>, path: &str) -> Option<EntrySummary> {
    let normalized = normalize_rel(path);
    if normalized.is_empty() {
        return None;
    }

    if let Some(content) = files.get(&normalized) {
        let name = normalized.rsplit('/').next().unwrap_or(&normalized);
        return Some(EntrySummary {
            name: name.to_string(),
            is_dir: false,
            size: Some(content.len() as u64),
        });
    }

    let dir_prefix = format!("{}/", normalized);
    if files.keys().any(|k| k.starts_with(&dir_prefix)) {
        let name = normalized.rsplit('/').next().unwrap_or(&normalized);
        return Some(EntrySummary {
            name: name.to_string(),
            is_dir: true,
            size: None,
        });
    }

    None
}
