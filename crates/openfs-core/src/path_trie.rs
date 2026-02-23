//! PathTrie — O(k) prefix lookup for filesystem paths (k = path depth).
//!
//! Replaces the O(n) `has_prefix()` scan on HashMap in `LruCache`.

use std::collections::HashMap;

/// A trie over `/`-separated path components for efficient prefix queries.
pub struct PathTrie {
    root: TrieNode,
    len: usize,
}

struct TrieNode {
    children: HashMap<String, TrieNode>,
    /// Whether this node is a terminal (a full path was inserted here).
    is_terminal: bool,
}

impl TrieNode {
    fn new() -> Self {
        TrieNode {
            children: HashMap::new(),
            is_terminal: false,
        }
    }
}

impl PathTrie {
    /// Create an empty trie.
    pub fn new() -> Self {
        PathTrie {
            root: TrieNode::new(),
            len: 0,
        }
    }

    /// Insert a path into the trie.
    pub fn insert(&mut self, path: &str) {
        let components = split_path(path);
        let mut node = &mut self.root;

        // Handle root path
        if components.is_empty() {
            if !node.is_terminal {
                node.is_terminal = true;
                self.len += 1;
            }
            return;
        }

        for component in components {
            node = node
                .children
                .entry(component.to_string())
                .or_insert_with(TrieNode::new);
        }
        if !node.is_terminal {
            node.is_terminal = true;
            self.len += 1;
        }
    }

    /// Remove a path from the trie. Returns true if the path was present.
    pub fn remove(&mut self, path: &str) -> bool {
        let components = split_path(path);
        if Self::remove_recursive(&mut self.root, &components, 0) {
            self.len -= 1;
            true
        } else {
            false
        }
    }

    fn remove_recursive(root: &mut TrieNode, components: &[&str], depth: usize) -> bool {
        if depth == components.len() {
            if root.is_terminal {
                root.is_terminal = false;
                return true;
            }
            return false;
        }

        let component = components[depth];
        if let Some(child) = root.children.get_mut(component) {
            let removed = Self::remove_recursive(child, components, depth + 1);
            if removed && !child.is_terminal && child.children.is_empty() {
                root.children.remove(component);
            }
            removed
        } else {
            false
        }
    }

    /// Check if a path exists in the trie.
    pub fn contains(&self, path: &str) -> bool {
        let components = split_path(path);
        let mut node = &self.root;

        if components.is_empty() {
            return node.is_terminal;
        }

        for component in components {
            match node.children.get(component) {
                Some(child) => node = child,
                None => return false,
            }
        }
        node.is_terminal
    }

    /// Check if any path in the trie starts with the given prefix.
    pub fn has_prefix(&self, prefix: &str) -> bool {
        let components = split_path(prefix);
        let mut node = &self.root;

        if components.is_empty() {
            // Root prefix — true if trie has anything
            return node.is_terminal || !node.children.is_empty();
        }

        for component in components {
            match node.children.get(component) {
                Some(child) => node = child,
                None => return false,
            }
        }
        // Found the prefix node — it has descendants (or is itself a terminal)
        true
    }

    /// List direct children of a path. Returns child names only.
    pub fn list_children(&self, path: &str) -> Vec<String> {
        let components = split_path(path);
        let mut node = &self.root;

        if !components.is_empty() {
            for component in components {
                match node.children.get(component) {
                    Some(child) => node = child,
                    None => return Vec::new(),
                }
            }
        }

        node.children.keys().cloned().collect()
    }

    /// Return all paths stored in the trie.
    pub fn all_paths(&self) -> Vec<String> {
        let mut result = Vec::new();
        self.collect_paths(&self.root, &mut Vec::new(), &mut result);
        result
    }

    fn collect_paths(&self, node: &TrieNode, components: &mut Vec<String>, result: &mut Vec<String>) {
        if node.is_terminal {
            if components.is_empty() {
                result.push("/".to_string());
            } else {
                result.push(format!("/{}", components.join("/")));
            }
        }
        for (name, child) in &node.children {
            components.push(name.clone());
            self.collect_paths(child, components, result);
            components.pop();
        }
    }

    /// Remove a prefix and all its descendants. Returns count of removed paths.
    pub fn remove_subtree(&mut self, prefix: &str) -> usize {
        let components = split_path(prefix);

        if components.is_empty() {
            let count = self.len;
            self.root = TrieNode::new();
            self.len = 0;
            return count;
        }

        let mut node = &mut self.root;
        let last = components.len() - 1;

        for component in &components[..last] {
            match node.children.get_mut(*component) {
                Some(child) => node = child,
                None => return 0,
            }
        }

        if let Some(removed_node) = node.children.remove(components[last]) {
            let count = count_terminals(&removed_node);
            self.len -= count;
            count
        } else {
            0
        }
    }

    /// Number of paths in the trie.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the trie is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for PathTrie {
    fn default() -> Self {
        Self::new()
    }
}

fn count_terminals(node: &TrieNode) -> usize {
    let mut count = if node.is_terminal { 1 } else { 0 };
    for child in node.children.values() {
        count += count_terminals(child);
    }
    count
}

/// Split a path into components, stripping leading/trailing slashes.
fn split_path(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_contains() {
        let mut trie = PathTrie::new();
        trie.insert("/a/b/c");
        trie.insert("/a/b/d");
        trie.insert("/x/y");

        assert!(trie.contains("/a/b/c"));
        assert!(trie.contains("/a/b/d"));
        assert!(trie.contains("/x/y"));
        assert!(!trie.contains("/a/b"));
        assert!(!trie.contains("/a"));
        assert!(!trie.contains("/z"));
        assert_eq!(trie.len(), 3);
    }

    #[test]
    fn test_has_prefix() {
        let mut trie = PathTrie::new();
        trie.insert("/a/b/c");
        trie.insert("/a/b/d");

        assert!(trie.has_prefix("/a"));
        assert!(trie.has_prefix("/a/b"));
        assert!(trie.has_prefix("/a/b/c"));
        assert!(!trie.has_prefix("/z"));
        assert!(!trie.has_prefix("/a/x"));
    }

    #[test]
    fn test_remove() {
        let mut trie = PathTrie::new();
        trie.insert("/a/b/c");
        trie.insert("/a/b/d");

        assert!(trie.remove("/a/b/c"));
        assert!(!trie.contains("/a/b/c"));
        assert!(trie.contains("/a/b/d"));
        assert_eq!(trie.len(), 1);

        // Removing non-existent returns false
        assert!(!trie.remove("/a/b/c"));
        assert!(!trie.remove("/z"));
    }

    #[test]
    fn test_remove_subtree() {
        let mut trie = PathTrie::new();
        trie.insert("/a/b/c");
        trie.insert("/a/b/d");
        trie.insert("/a/e");
        trie.insert("/x/y");

        let removed = trie.remove_subtree("/a/b");
        assert_eq!(removed, 2);
        assert!(!trie.contains("/a/b/c"));
        assert!(!trie.contains("/a/b/d"));
        assert!(trie.contains("/a/e"));
        assert!(trie.contains("/x/y"));
        assert_eq!(trie.len(), 2);
    }

    #[test]
    fn test_list_children() {
        let mut trie = PathTrie::new();
        trie.insert("/a/b/c");
        trie.insert("/a/b/d");
        trie.insert("/a/e");

        let mut children = trie.list_children("/a");
        children.sort();
        assert_eq!(children, vec!["b", "e"]);

        let mut children = trie.list_children("/a/b");
        children.sort();
        assert_eq!(children, vec!["c", "d"]);

        assert!(trie.list_children("/z").is_empty());
    }

    #[test]
    fn test_all_paths() {
        let mut trie = PathTrie::new();
        trie.insert("/a/b");
        trie.insert("/a/c");
        trie.insert("/d");

        let mut paths = trie.all_paths();
        paths.sort();
        assert_eq!(paths, vec!["/a/b", "/a/c", "/d"]);
    }

    #[test]
    fn test_empty_trie() {
        let trie = PathTrie::new();
        assert!(trie.is_empty());
        assert_eq!(trie.len(), 0);
        assert!(!trie.contains("/anything"));
        assert!(trie.all_paths().is_empty());
        assert!(trie.list_children("/").is_empty());
    }

    #[test]
    fn test_root_path() {
        let mut trie = PathTrie::new();
        trie.insert("/");
        assert!(trie.contains("/"));
        assert_eq!(trie.len(), 1);
        assert_eq!(trie.all_paths(), vec!["/"]);

        assert!(trie.remove("/"));
        assert!(!trie.contains("/"));
        assert_eq!(trie.len(), 0);
    }

    #[test]
    fn test_deep_nesting() {
        let mut trie = PathTrie::new();
        let deep = "/a/b/c/d/e/f/g/h/i/j/k/l";
        trie.insert(deep);
        assert!(trie.contains(deep));
        assert!(trie.has_prefix("/a/b/c/d/e"));
        assert!(!trie.has_prefix("/a/b/c/d/z"));
        assert_eq!(trie.len(), 1);

        let removed = trie.remove_subtree("/a/b/c/d/e");
        assert_eq!(removed, 1);
        assert!(trie.is_empty());
    }

    #[test]
    fn test_duplicate_insert() {
        let mut trie = PathTrie::new();
        trie.insert("/a/b");
        trie.insert("/a/b");
        assert_eq!(trie.len(), 1);
    }

    #[test]
    fn test_remove_subtree_root() {
        let mut trie = PathTrie::new();
        trie.insert("/a");
        trie.insert("/b");
        trie.insert("/c/d");
        let removed = trie.remove_subtree("/");
        assert_eq!(removed, 3);
        assert!(trie.is_empty());
    }
}
