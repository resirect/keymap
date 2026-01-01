//! Safe commits of change plans.
//!
//! 1. Re-check every touched file against the hash the plan was built from.
//! 2. Stage the whole include graph into a private temporary directory with
//!    include paths remapped to the staged copies, and run `niri validate`
//!    on the staged root. Live files are never used for validation.
//! 3. Save before/after copies and a journal under the state directory.
//! 4. Replace each file atomically (temporary sibling + rename), writing to
//!    the symlink target so symlinks stay symlinks, preserving permissions.
//! 5. Mark the journal committed. A journal left "prepared" means the commit
//!    was interrupted; `pending_recoveries` reports it.
//!
//! Undo is a new transaction whose precondition is that every file still has
//! exactly the bytes this transaction wrote.

use crate::config::hash_bytes;
use crate::kdl;
use crate::model::Model;
use crate::plan::{FileChange, Plan, TextEdit, apply_edits};
use crate::store;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const KEEP_TRANSACTIONS: usize = 30;

pub trait Validator {
    /// Validate the configuration rooted at `root`. Err carries the message.
    fn validate(&self, root: &Path) -> Result<(), String>;
}

/// Runs the installed `niri validate -c ROOT` (flags checked against niri 26.04).
pub struct NiriValidator {
    pub bin: PathBuf,
}

impl Default for NiriValidator {
    fn default() -> Self {
        NiriValidator { bin: PathBuf::from("niri") }
    }
}

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            if it.peek() == Some(&'[') {
                it.next();
                for d in it.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

impl Validator for NiriValidator {
    fn validate(&self, root: &Path) -> Result<(), String> {
        let out = Command::new(&self.bin)
            .arg("validate")
            .arg("-c")
            .arg(root)
            .env_remove("NIRI_CONFIG")
            .env("NO_COLOR", "1")
            .output()
            .map_err(|e| format!("could not run `{} validate`: {e}", self.bin.display()))?;
        if out.status.success() {
            Ok(())
        } else {
            let msg = strip_ansi(&String::from_utf8_lossy(&out.stderr));
            let msg = msg
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.split_once(" niri: ").map_or(l, |(_, r)| r).trim_end())
                .collect::<Vec<_>>()
                .join("\n");
            Err(if msg.is_empty() { format!("niri validate failed ({})", out.status) } else { msg })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TxState {
    Prepared,
    Committed,
    Undone,
    RolledBack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalFile {
    pub path: PathBuf,
    pub real_path: PathBuf,
    pub before_hash: String,
    pub after_hash: String,
    pub mode: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Journal {
    pub txid: String,
    pub created_unix: u64,
    pub description: String,
    pub state: TxState,
    /// Set when this transaction undoes another.
    #[serde(default)]
    pub undoes: Option<String>,
    pub files: Vec<JournalFile>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum CommitError {
    /// Files changed on disk since the plan was made.
    Conflict(Vec<PathBuf>),
    Validation(String),
    Io(String),
}

impl std::fmt::Display for CommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommitError::Conflict(p) => write!(
                f,
                "changed on disk since it was loaded: {}",
                p.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
            ),
            CommitError::Validation(m) => write!(f, "niri rejected the new configuration:\n{m}"),
            CommitError::Io(m) => f.write_str(m),
        }
    }
}

fn io(e: impl std::fmt::Display) -> CommitError {
    CommitError::Io(e.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct Receipt {
    pub txid: String,
    pub description: String,
    pub files: Vec<PathBuf>,
}

pub struct Transactions {
    pub dir: PathBuf,
}

impl Default for Transactions {
    fn default() -> Self {
        Transactions { dir: store::state_dir().join("backups") }
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn new_txid() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    format!("{}-{:06}-{}", t.as_secs(), t.subsec_micros(), std::process::id())
}

fn private_dir(p: &Path) -> std::io::Result<()> {
    fs::create_dir_all(p)?;
    fs::set_permissions(p, fs::Permissions::from_mode(0o700))
}

fn write_new(path: &Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    let mut f = fs::OpenOptions::new().write(true).create_new(true).mode(mode).open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// Stage the include graph with the plan applied; returns the staged root.
pub fn stage(model: &Model, plan: &Plan, dir: &Path) -> std::io::Result<PathBuf> {
    private_dir(dir)?;
    let files = &model.config.files;
    let staged: Vec<PathBuf> = files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let base = f.path.file_name().and_then(|n| n.to_str()).unwrap_or("config.kdl");
            dir.join(format!("{i}-{base}"))
        })
        .collect();
    for (i, f) in files.iter().enumerate() {
        let text = plan.changes.iter().find(|c| c.path == f.path).map_or(f.text.as_str(), |c| c.after.as_str());
        let mut edits = Vec::new();
        if let Ok(doc) = kdl::parse(text) {
            let refs: Vec<_> = model.config.includes.iter().filter(|r| r.from_file == i).collect();
            let nodes = doc.enabled().filter(|n| n.name == "include");
            for (k, (node, r)) in nodes.zip(refs).enumerate() {
                let Some(arg) = node.args().next() else { continue };
                let target = match r.file {
                    Some(fi) => staged[fi].clone(),
                    None => dir.join(format!("missing-{i}-{k}.kdl")),
                };
                edits.push(TextEdit { start: arg.span.start, end: arg.span.end, text: kdl::quote(&target.to_string_lossy()) });
            }
        }
        write_new(&staged[i], apply_edits(text, &edits).as_bytes(), 0o600)?;
    }
    Ok(staged[0].clone())
}

impl Transactions {
    fn tx_dir(&self, txid: &str) -> PathBuf {
        self.dir.join(txid)
    }

    fn write_journal(&self, j: &Journal) -> Result<(), CommitError> {
        let bytes = serde_json::to_vec_pretty(j).map_err(io)?;
        store::write_private(&self.tx_dir(&j.txid).join("journal.json"), &bytes).map_err(io)
    }

    pub fn journal(&self, txid: &str) -> Option<Journal> {
        let b = fs::read(self.tx_dir(txid).join("journal.json")).ok()?;
        serde_json::from_slice(&b).ok()
    }

    /// Transactions, newest first.
    pub fn history(&self) -> Vec<Journal> {
        let mut v: Vec<Journal> =
            fs::read_dir(&self.dir).into_iter().flatten().flatten().filter_map(|e| self.journal(e.file_name().to_str()?)).collect();
        v.sort_by(|a, b| b.txid.cmp(&a.txid));
        v
    }

    pub fn pending_recoveries(&self) -> Vec<Journal> {
        self.history().into_iter().filter(|j| j.state == TxState::Prepared).collect()
    }

    fn prune(&self) {
        for j in self.history().into_iter().skip(KEEP_TRANSACTIONS) {
            if j.state != TxState::Prepared {
                let _ = fs::remove_dir_all(self.tx_dir(&j.txid));
            }
        }
    }

    /// Validate and commit a plan. Nothing live is written unless
    /// validation passes and every file still matches the plan's snapshot.
    pub fn commit(&self, model: &Model, plan: &Plan, validator: &dyn Validator) -> Result<Receipt, CommitError> {
        self.commit_inner(model, plan, validator, None)
    }

    fn commit_inner(&self, model: &Model, plan: &Plan, validator: &dyn Validator, undoes: Option<String>) -> Result<Receipt, CommitError> {
        if plan.changes.is_empty() {
            return Err(CommitError::Io("nothing to save".into()));
        }
        check_current(&plan.changes)?;
        let txid = new_txid();
        let tdir = self.tx_dir(&txid);
        private_dir(&self.dir).map_err(io)?;
        private_dir(&tdir).map_err(io)?;
        let stage_dir = tdir.join("staged");
        let root = stage(model, plan, &stage_dir).map_err(io)?;
        let result = validator.validate(&root);
        let _ = fs::remove_dir_all(&stage_dir);
        if let Err(msg) = result {
            let _ = fs::remove_dir_all(&tdir);
            let mut msg = msg;
            for (i, f) in model.config.files.iter().enumerate() {
                let base = f.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let staged = stage_dir.join(format!("{i}-{base}"));
                msg = msg.replace(&*staged.to_string_lossy(), &f.path.to_string_lossy());
                msg = msg.replace(&format!("{i}-{base}"), &f.path.to_string_lossy());
            }
            return Err(CommitError::Validation(msg));
        }
        let mut journal = Journal {
            txid: txid.clone(),
            created_unix: now_unix(),
            description: plan.description.clone(),
            state: TxState::Prepared,
            undoes,
            files: Vec::new(),
        };
        for (i, c) in plan.changes.iter().enumerate() {
            let mode = fs::metadata(&c.real_path).map(|m| m.permissions().mode() & 0o7777).unwrap_or(0o644);
            write_new(&tdir.join(format!("{i}.before")), c.before.as_bytes(), 0o600).map_err(io)?;
            write_new(&tdir.join(format!("{i}.after")), c.after.as_bytes(), 0o600).map_err(io)?;
            journal.files.push(JournalFile {
                path: c.path.clone(),
                real_path: c.real_path.clone(),
                before_hash: c.expected_hash.clone(),
                after_hash: hash_bytes(c.after.as_bytes()),
                mode,
            });
        }
        self.write_journal(&journal)?;
        let mut done: Vec<usize> = Vec::new();
        for (i, c) in plan.changes.iter().enumerate() {
            if let Err(e) = replace_file(c, journal.files[i].mode, &txid) {
                // Put back what we already replaced, if untouched since.
                for d in done.iter().rev() {
                    let pc = &plan.changes[*d];
                    let _ = restore_if(&pc.real_path, &journal.files[*d].after_hash, pc.before.as_bytes(), journal.files[*d].mode, &txid);
                }
                journal.state = TxState::RolledBack;
                let _ = self.write_journal(&journal);
                return Err(e);
            }
            done.push(i);
        }
        journal.state = TxState::Committed;
        self.write_journal(&journal)?;
        self.prune();
        Ok(Receipt { txid, description: plan.description.clone(), files: plan.changes.iter().map(|c| c.path.clone()).collect() })
    }

    /// Build the inverse plan of a committed transaction. Fails, naming the
    /// files, when any of them changed after this transaction wrote it.
    pub fn undo_plan(&self, txid: &str) -> Result<Plan, CommitError> {
        let j = self.journal(txid).ok_or_else(|| io("transaction not found"))?;
        if j.state != TxState::Committed {
            return Err(io(format!("transaction is {:?}, not committed", j.state)));
        }
        let mut changes = Vec::new();
        let mut conflicts = Vec::new();
        for (i, f) in j.files.iter().enumerate() {
            let tdir = self.tx_dir(txid);
            let before = fs::read_to_string(tdir.join(format!("{i}.before"))).map_err(io)?;
            let after = fs::read_to_string(tdir.join(format!("{i}.after"))).map_err(io)?;
            let current = fs::read(&f.real_path).map_err(io)?;
            if hash_bytes(&current) != f.after_hash {
                conflicts.push(f.path.clone());
                continue;
            }
            changes.push(FileChange {
                path: f.path.clone(),
                real_path: f.real_path.clone(),
                expected_hash: f.after_hash.clone(),
                before: after,
                after: before,
            });
        }
        if !conflicts.is_empty() {
            return Err(CommitError::Conflict(conflicts));
        }
        Ok(Plan { changes, description: format!("Undo: {}", j.description), notes: vec![] })
    }

    /// Undo a committed transaction (validated like any other commit).
    pub fn undo(&self, model: &Model, txid: &str, validator: &dyn Validator) -> Result<Receipt, CommitError> {
        let plan = self.undo_plan(txid)?;
        let r = self.commit_inner(model, &plan, validator, Some(txid.to_string()))?;
        if let Some(mut j) = self.journal(txid) {
            j.state = TxState::Undone;
            self.write_journal(&j)?;
        }
        Ok(r)
    }

    /// Recover an interrupted transaction: restore files that hold this
    /// transaction's "after" bytes; leave files with other content alone.
    pub fn recover(&self, txid: &str) -> Result<Vec<String>, CommitError> {
        let mut j = self.journal(txid).ok_or_else(|| io("transaction not found"))?;
        let mut report = Vec::new();
        for (i, f) in j.files.iter().enumerate() {
            let before = fs::read(self.tx_dir(txid).join(format!("{i}.before"))).map_err(io)?;
            let current = fs::read(&f.real_path).unwrap_or_default();
            let h = hash_bytes(&current);
            if h == f.before_hash {
                report.push(format!("{}: unchanged", f.path.display()));
            } else if h == f.after_hash {
                restore_if(&f.real_path, &f.after_hash, &before, f.mode, txid)?;
                report.push(format!("{}: restored", f.path.display()));
            } else {
                report.push(format!("{}: modified by something else; left as is (backup kept)", f.path.display()));
            }
        }
        j.state = TxState::RolledBack;
        self.write_journal(&j)?;
        Ok(report)
    }

    /// The saved "before" text of a file in a transaction (for manual recovery).
    pub fn backup_path(&self, txid: &str, index: usize) -> PathBuf {
        self.tx_dir(txid).join(format!("{index}.before"))
    }
}

fn check_current(changes: &[FileChange]) -> Result<(), CommitError> {
    let conflicts: Vec<PathBuf> = changes
        .iter()
        .filter(|c| fs::read(&c.real_path).map(|b| hash_bytes(&b) != c.expected_hash).unwrap_or(true))
        .map(|c| c.path.clone())
        .collect();
    if conflicts.is_empty() { Ok(()) } else { Err(CommitError::Conflict(conflicts)) }
}

fn atomic_write(real: &Path, bytes: &[u8], mode: u32, txid: &str, expect_hash: &str) -> Result<(), CommitError> {
    let dir = real.parent().ok_or_else(|| io("file has no parent directory"))?;
    let name = real.file_name().and_then(|n| n.to_str()).unwrap_or("config");
    let tmp = dir.join(format!(".{name}.keymap-{txid}.tmp"));
    let _ = fs::remove_file(&tmp);
    write_new(&tmp, bytes, mode).map_err(|e| io(format!("cannot write {}: {e}", tmp.display())))?;
    let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(mode));
    // Last-moment check to narrow the window for concurrent edits.
    let cur = fs::read(real).map(|b| hash_bytes(&b)).unwrap_or_default();
    if cur != expect_hash {
        let _ = fs::remove_file(&tmp);
        return Err(CommitError::Conflict(vec![real.to_path_buf()]));
    }
    fs::rename(&tmp, real).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        io(format!("cannot replace {}: {e}", real.display()))
    })?;
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

fn replace_file(c: &FileChange, mode: u32, txid: &str) -> Result<(), CommitError> {
    atomic_write(&c.real_path, c.after.as_bytes(), mode, txid, &c.expected_hash)
}

fn restore_if(real: &Path, expect_hash: &str, bytes: &[u8], mode: u32, txid: &str) -> Result<(), CommitError> {
    atomic_write(real, bytes, mode, &format!("{txid}-restore"), expect_hash)
}
