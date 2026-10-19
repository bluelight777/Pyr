//! What changed between two versions' files, as a unified diff (for the review).

use std::collections::BTreeMap;

/// Lines in common, longest (dynamic programming; fine for files the size plugins are).
fn lcs(a: &[&str], b: &[&str]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    let mut t = vec![0u32; (n + 1) * (m + 1)];
    let w = m + 1;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[i * w + j] = if a[i] == b[j] { t[(i + 1) * w + j + 1] + 1 } else { t[(i + 1) * w + j].max(t[i * w + j + 1]) };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if t[(i + 1) * w + j] >= t[i * w + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// One file's changes with `context` lines round them.
pub fn unified(path: &str, old: &str, new: &str, context: usize) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    if a.len() * b.len() > 25_000_000 {
        // Too big to line up: the whole of both.
        return format!("--- a/{path}\n+++ b/{path}\n(too many lines to compare: the whole new file follows)\n{}\n", b.iter().map(|l| format!("+{l}")).collect::<Vec<_>>().join("\n"));
    }
    let common = lcs(&a, &b);
    // Every line, marked.
    let mut ops: Vec<(char, &str)> = Vec::new();
    let (mut i, mut j) = (0, 0);
    for (ci, cj) in common.iter().copied().chain(std::iter::once((a.len(), b.len()))) {
        while i < ci {
            ops.push(('-', a[i]));
            i += 1;
        }
        while j < cj {
            ops.push(('+', b[j]));
            j += 1;
        }
        if ci < a.len() && cj < b.len() {
            ops.push((' ', a[ci]));
            i += 1;
            j += 1;
        }
    }
    if ops.iter().all(|(k, _)| *k == ' ') {
        return String::new();
    }
    let mut out = format!("--- a/{path}\n+++ b/{path}\n");
    let changed: Vec<usize> = ops.iter().enumerate().filter(|(_, (k, _))| *k != ' ').map(|(n, _)| n).collect();
    let mut shown = vec![false; ops.len()];
    for &c in &changed {
        for k in c.saturating_sub(context)..(c + context + 1).min(ops.len()) {
            shown[k] = true;
        }
    }
    let mut gap = false;
    for (k, (op, line)) in ops.iter().enumerate() {
        if shown[k] {
            if gap {
                out.push_str("@@\n");
                gap = false;
            }
            out.push(*op);
            out.push_str(line);
            out.push('\n');
        } else {
            gap = true;
        }
    }
    out
}

/// Every file's changes: added, changed and removed (binary files by size and hash only).
pub fn between(old: &[(String, Vec<u8>)], new: &[(String, Vec<u8>)]) -> String {
    let o: BTreeMap<&str, &[u8]> = old.iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect();
    let n: BTreeMap<&str, &[u8]> = new.iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect();
    let mut out = String::new();
    let text = |b: &[u8]| std::str::from_utf8(b).ok().filter(|s| !s.contains('\0')).map(str::to_string);
    for (path, nb) in &n {
        match o.get(path) {
            Some(ob) if ob == nb => {}
            Some(ob) => match (text(ob), text(nb)) {
                (Some(a), Some(b)) => out.push_str(&unified(path, &a, &b, 3)),
                _ => out.push_str(&format!("~~~ {path}: binary, changed ({} bytes, sha256 {})\n", nb.len(), pyr_proto::release::sha256_hex(nb))),
            },
            None => match text(nb) {
                Some(b) => out.push_str(&unified(path, "", &b, 3)),
                None => out.push_str(&format!("+++ {path}: binary, new ({} bytes, sha256 {})\n", nb.len(), pyr_proto::release::sha256_hex(nb))),
            },
        }
    }
    for path in o.keys() {
        if !n.contains_key(path) {
            out.push_str(&format!("--- {path}: removed\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_what_changed_and_only_that() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
        let new = "a\nb\nc\nd\nE\nf\ng\nh\ni\nj\nk\n";
        let d = unified("x.rs", old, new, 1);
        assert!(d.contains("-e\n+E\n"), "{d}");
        assert!(d.contains("+k\n"), "{d}");
        assert!(!d.contains(" a\n"), "far context left out: {d}");
        assert!(unified("x.rs", old, old, 3).is_empty());
        let files = between(&[("a.rs".into(), b"x\n".to_vec()), ("gone.rs".into(), b"y\n".to_vec())], &[("a.rs".into(), b"x\nz\n".to_vec()), ("p.png".into(), vec![0, 1, 2])]);
        assert!(files.contains("+z") && files.contains("gone.rs: removed") && files.contains("p.png: binary, new"), "{files}");
    }
}
