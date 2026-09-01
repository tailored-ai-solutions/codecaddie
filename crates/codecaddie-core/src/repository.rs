use codecaddie_domain::{EvidenceKind, EvidenceRef};
use fs2::FileExt;
use std::{
    collections::{HashMap, VecDeque},
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, ExitStatus, Output, Stdio},
};
use tempfile::TempDir;

const MAX_BLOB_BYTES: usize = 8 * 1024 * 1024;
const MAX_SNAPSHOT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_SNAPSHOT_FILES: usize = 100_000;
pub const MAX_EVIDENCE_LINES: u32 = 80;

#[derive(Debug, Clone)]
pub struct LocalRepository {
    pub id: String,
    pub path: PathBuf,
}

impl LocalRepository {
    pub fn attach(id: impl Into<String>, path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().canonicalize()?;
        if !run_git(&path, ["rev-parse", "--is-inside-work-tree"])?
            .status
            .success()
        {
            anyhow::bail!("path is not a Git work tree");
        }
        Ok(Self {
            id: id.into(),
            path,
        })
    }

    pub fn head(&self) -> anyhow::Result<String> {
        git_text(&self.path, ["rev-parse", "HEAD"])
    }

    /// Reports whether the attached checkout differs from its frozen HEAD.
    /// Only Git status metadata crosses this boundary; filenames and source
    /// content are deliberately discarded.
    pub fn working_tree_dirty(&self) -> anyhow::Result<bool> {
        let (status, output) = run_git_stdout_bounded(
            &self.path,
            ["status", "--porcelain=v1", "--untracked-files=normal"],
            1024 * 1024,
        )?;
        if !status.success() {
            anyhow::bail!("could not inspect repository working tree");
        }
        Ok(!output.is_empty())
    }

    pub fn disposable_clone(&self, commit: &str) -> anyhow::Result<DisposableClone> {
        let commit = self.resolve_commit(commit)?;
        let directory = tempfile::Builder::new()
            .prefix("codecaddie-scan-")
            .tempdir()?;
        self.clone_at(&commit, directory.path())?;
        Ok(DisposableClone { directory, commit })
    }

    /// Creates an independent clone inside a scan workspace that may contain
    /// several repositories. The destination is disposable and must never be
    /// the user's attached checkout.
    fn disposable_clone_into(
        &self,
        commit: &str,
        destination: impl AsRef<Path>,
    ) -> anyhow::Result<String> {
        let commit = self.resolve_commit(commit)?;
        self.clone_at(&commit, destination.as_ref())?;
        Ok(commit)
    }

    fn clone_at(&self, commit: &str, destination: &Path) -> anyhow::Result<()> {
        let destination = if destination.is_absolute() {
            destination.to_path_buf()
        } else {
            std::env::current_dir()?.join(destination)
        };
        std::fs::create_dir_all(&destination)?;
        let (status, listing) = run_git_stdout_bounded(
            &self.path,
            ["ls-tree", "-r", "-z", "--full-tree", commit],
            64 * 1024 * 1024,
        )?;
        if !status.success() {
            anyhow::bail!("could not list frozen repository tree");
        }
        let mut entries = Vec::new();
        for entry in listing
            .split(|byte| *byte == 0)
            .filter(|item| !item.is_empty())
        {
            let tab = entry
                .iter()
                .position(|byte| *byte == b'\t')
                .ok_or_else(|| anyhow::anyhow!("Git returned a malformed tree entry"))?;
            let metadata = std::str::from_utf8(&entry[..tab])?;
            let path = std::str::from_utf8(&entry[tab + 1..])?.to_owned();
            validate_relative_path(&path)?;
            let mut fields = metadata.split_whitespace();
            let mode = fields.next().unwrap_or_default();
            let kind = fields.next().unwrap_or_default();
            let oid = fields.next().unwrap_or_default();
            if kind != "blob" || oid.is_empty() {
                continue;
            }
            if entries.len() >= MAX_SNAPSHOT_FILES {
                anyhow::bail!("repository snapshot exceeds the 100,000-file safety limit");
            }
            entries.push(SnapshotEntry {
                mode: mode.to_owned(),
                oid: oid.to_owned(),
                path,
            });
        }
        self.materialize_snapshot(entries, &destination)
    }

    /// Reads every blob through one long-lived `git cat-file --batch`
    /// process. This keeps large repositories to two Git subprocesses total
    /// (tree listing plus batch materialization) and lets size limits apply
    /// before any blob-sized allocation.
    fn materialize_snapshot(
        &self,
        entries: Vec<SnapshotEntry>,
        destination: &Path,
    ) -> anyhow::Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut child = Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("Git batch input was unavailable"))?;
        let requests = entries
            .iter()
            .map(|entry| entry.oid.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let writer = std::thread::spawn(move || stdin.write_all(requests.as_bytes()));
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Git batch output was unavailable"))?;
        let mut reader = BufReader::new(stdout);
        let materialized = materialize_batch_entries(&mut reader, &entries, destination);
        drop(reader);
        if let Err(error) = materialized {
            let _ = child.kill();
            let _ = child.wait();
            let _ = writer.join();
            return Err(error);
        }
        writer
            .join()
            .map_err(|_| anyhow::anyhow!("Git batch input writer failed"))??;
        let output = child.wait_with_output()?;
        ensure_git(&output, "materialize frozen repository blobs")
    }

    pub fn resolve_commit(&self, requested: &str) -> anyhow::Result<String> {
        let resolved = git_text(
            &self.path,
            ["rev-parse", "--verify", &format!("{requested}^{{commit}}")],
        )?;
        if resolved.len() != 40 && resolved.len() != 64 {
            anyhow::bail!("Git returned an invalid commit identifier");
        }
        Ok(resolved)
    }

    pub fn evidence(
        &self,
        commit: &str,
        path: &str,
        start_line: u32,
        end_line: u32,
        kind: EvidenceKind,
    ) -> anyhow::Result<EvidenceRef> {
        validate_relative_path(path)?;
        if start_line == 0 || end_line < start_line {
            anyhow::bail!("invalid evidence line range");
        }
        let commit = self.resolve_commit(commit)?;
        let path = self.resolve_tree_path(&commit, path)?;
        let blob_oid = git_text(&self.path, ["rev-parse", &format!("{commit}:{path}")])?;
        let (status, output) =
            run_git_stdout_bounded(&self.path, ["cat-file", "-p", &blob_oid], MAX_BLOB_BYTES)?;
        if !status.success() {
            anyhow::bail!("could not read evidence blob");
        }
        let text = std::str::from_utf8(&output)?;
        let lines: Vec<&str> = text.lines().collect();
        if start_line as usize > lines.len() {
            anyhow::bail!("evidence line range exceeds the blob");
        }
        // Providers occasionally cite one line past EOF after inspecting a
        // rendered or case-insensitive working tree. Keep the immutable start
        // coordinate and clamp only the trailing edge to the frozen blob.
        // Ranges wider than the 80-line viewer limit are likewise clamped
        // rather than discarded: the leading lines are real evidence, and
        // rejecting the citation would silently flip its criterion to
        // unverified.
        let end_line = end_line
            .min(lines.len() as u32)
            .min(start_line + MAX_EVIDENCE_LINES - 1);
        let excerpt = lines[(start_line - 1) as usize..end_line as usize].join("\n");
        Ok(EvidenceRef {
            repository_id: self.id.clone(),
            commit_sha: commit,
            blob_oid,
            path,
            start_line,
            end_line,
            content_hash: blake3::hash(excerpt.as_bytes()).to_hex().to_string(),
            kind,
        })
    }

    /// Resolves provider-supplied path casing against the frozen Git tree.
    /// macOS commonly presents a case-insensitive working tree even though
    /// the Git object database is case-sensitive. The exact tree spelling is
    /// the immutable coordinate retained in reports.
    fn resolve_tree_path(&self, commit: &str, requested: &str) -> anyhow::Result<String> {
        if run_git(
            &self.path,
            ["cat-file", "-e", &format!("{commit}:{requested}")],
        )?
        .status
        .success()
        {
            return Ok(requested.to_owned());
        }

        let output = run_git(&self.path, ["ls-tree", "-r", "--name-only", commit])?;
        ensure_git(&output, "list frozen repository paths")?;
        let paths = String::from_utf8(output.stdout)?;
        let mut matches = paths
            .lines()
            .filter(|candidate| candidate.eq_ignore_ascii_case(requested));
        let matched = matches
            .next()
            .ok_or_else(|| anyhow::anyhow!("evidence path does not exist at the frozen commit"))?;
        if matches.next().is_some() {
            anyhow::bail!("evidence path casing is ambiguous at the frozen commit");
        }
        Ok(matched.to_owned())
    }

    pub fn read_evidence(&self, evidence: &EvidenceRef) -> anyhow::Result<String> {
        self.verify_evidence(evidence)?;
        let (status, output) = run_git_stdout_bounded(
            &self.path,
            ["cat-file", "-p", &evidence.blob_oid],
            MAX_BLOB_BYTES,
        )?;
        if !status.success() {
            anyhow::bail!("could not read evidence blob");
        }
        let text = std::str::from_utf8(&output)?;
        Ok(text.lines().collect::<Vec<_>>()
            [(evidence.start_line - 1) as usize..evidence.end_line as usize]
            .join("\n"))
    }

    /// Re-resolves an evidence coordinate from Git and compares its immutable
    /// blob and excerpt hashes without returning repository source. Persistence
    /// and export boundaries use this proof before accepting report metadata.
    pub fn verify_evidence(&self, evidence: &EvidenceRef) -> anyhow::Result<()> {
        if evidence.repository_id != self.id {
            anyhow::bail!("evidence belongs to a different repository");
        }
        let validated = self.evidence(
            &evidence.commit_sha,
            &evidence.path,
            evidence.start_line,
            evidence.end_line,
            evidence.kind,
        )?;
        if validated.blob_oid != evidence.blob_oid
            || validated.content_hash != evidence.content_hash
        {
            anyhow::bail!("evidence no longer matches its immutable coordinates");
        }
        Ok(())
    }

    /// Compares provider prose with streamed fingerprints of every non-binary
    /// tracked line, reporting which narrative fields matched. Full lines and
    /// three-to-four-word fragments of at least 24 characters are represented
    /// by fixed-size hashes; single words are deliberately not fingerprinted —
    /// a lone identifier is coordinate-adjacent vocabulary, and reports
    /// already legally carry paths, which reveal more than one identifier.
    /// Repository text is never retained, and a repository up to the snapshot
    /// limit can be checked without first collecting all Git output in memory.
    pub(crate) fn narrative_fields_matching_source(
        &self,
        commit: &str,
        fields: &[String],
    ) -> anyhow::Result<std::collections::BTreeSet<usize>> {
        let commit = self.resolve_commit(commit)?;
        let narrative = field_fingerprints(fields)?;
        let mut matched = std::collections::BTreeSet::new();
        if narrative.is_empty() {
            return Ok(matched);
        }
        let mut child = Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args(["grep", "-I", "-h", "-e", ".", &commit, "--"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Git stdout was unavailable"))?;
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::new();
        let mut total = 0_u64;
        loop {
            line.clear();
            let count = reader
                .by_ref()
                .take(MAX_BLOB_BYTES as u64 + 1)
                .read_until(b'\n', &mut line)?;
            if count == 0 {
                break;
            }
            total = total
                .checked_add(count as u64)
                .ok_or_else(|| anyhow::anyhow!("repository text size overflowed"))?;
            if total > MAX_SNAPSHOT_BYTES {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!("repository text exceeds the 512 MiB safety limit");
            }
            if line.len() > MAX_BLOB_BYTES {
                let remainder = if line.ends_with(b"\n") {
                    0
                } else {
                    discard_through_newline(&mut reader)?
                };
                total = total
                    .checked_add(remainder as u64)
                    .ok_or_else(|| anyhow::anyhow!("repository text size overflowed"))?;
                if total > MAX_SNAPSHOT_BYTES {
                    let _ = child.kill();
                    let _ = child.wait();
                    anyhow::bail!("repository text exceeds the 512 MiB safety limit");
                }
                continue;
            }
            while line
                .last()
                .is_some_and(|byte| matches!(byte, b'\n' | b'\r'))
            {
                line.pop();
            }
            if let Ok(text) = std::str::from_utf8(&line) {
                collect_matching_fields(text, &narrative, &mut matched);
                if matched.len() == fields.len() {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(matched);
                }
            }
        }
        let status = child.wait()?;
        if !status.success() && status.code() != Some(1) {
            anyhow::bail!("could not scan frozen text for source retention");
        }
        Ok(matched)
    }
}

fn discard_through_newline(reader: &mut BufReader<impl Read>) -> anyhow::Result<usize> {
    let mut discarded = 0_usize;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(discarded);
        }
        let count = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        let ended = available.get(count.saturating_sub(1)) == Some(&b'\n');
        reader.consume(count);
        discarded = discarded
            .checked_add(count)
            .ok_or_else(|| anyhow::anyhow!("repository text size overflowed"))?;
        if ended {
            return Ok(discarded);
        }
    }
}

/// The minimum length for a fingerprinted line or phrase. Aligned with the
/// 24-character cited-line threshold the analysis contract already publishes
/// in `evidence-rules.md`; shorter fragments are dictionary phrases
/// ("authentication middleware") whose collision with README prose says
/// nothing about source retention.
const MIN_FINGERPRINT_CHARS: usize = 24;

/// Walks every fingerprintable fragment of one line of text: the full
/// trimmed line and every three-to-four-word window, each at least
/// [`MIN_FINGERPRINT_CHARS`] characters. Single words and two-word phrases
/// are deliberately excluded (see `narrative_fields_matching_source`).
fn each_fingerprint(
    line: &str,
    mut apply: impl FnMut([u8; 32]) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let trimmed = line.trim();
    if trimmed.chars().count() >= MIN_FINGERPRINT_CHARS {
        apply(*blake3::hash(trimmed.as_bytes()).as_bytes())?;
    }
    let mut window = VecDeque::with_capacity(4);
    for word in line.split_whitespace() {
        let word = word.trim_matches(|character: char| !character.is_alphanumeric());
        if word.is_empty() {
            continue;
        }
        window.push_back(word);
        if window.len() > 4 {
            window.pop_front();
        }
        for count in 3..=window.len() {
            let mut hasher = blake3::Hasher::new();
            let mut characters = 0_usize;
            for (index, part) in window.iter().skip(window.len() - count).enumerate() {
                if index > 0 {
                    hasher.update(b" ");
                    characters += 1;
                }
                hasher.update(part.as_bytes());
                characters += part.chars().count();
            }
            if characters >= MIN_FINGERPRINT_CHARS {
                apply(*hasher.finalize().as_bytes())?;
            }
        }
    }
    Ok(())
}

/// Builds one fingerprint table for every narrative field, mapping each
/// fingerprint to the indices of the fields that produced it.
fn field_fingerprints(fields: &[String]) -> anyhow::Result<HashMap<[u8; 32], Vec<usize>>> {
    const MAX_FINGERPRINTS: usize = 250_000;
    let mut fingerprints: HashMap<[u8; 32], Vec<usize>> = HashMap::new();
    for (index, field) in fields.iter().enumerate() {
        for line in field.lines() {
            each_fingerprint(line, |fingerprint| {
                if !fingerprints.contains_key(&fingerprint)
                    && fingerprints.len() >= MAX_FINGERPRINTS
                {
                    anyhow::bail!("repository text exceeds the report fingerprint safety limit");
                }
                let owners = fingerprints.entry(fingerprint).or_default();
                if !owners.contains(&index) {
                    owners.push(index);
                }
                Ok(())
            })?;
        }
    }
    Ok(fingerprints)
}

/// Hashes one line of repository text through the same fragment walk as the
/// narrative side and records every narrative field whose fingerprint table
/// contains a produced hash.
fn collect_matching_fields(
    text: &str,
    expected: &HashMap<[u8; 32], Vec<usize>>,
    matched: &mut std::collections::BTreeSet<usize>,
) {
    for line in text.lines() {
        let _ = each_fingerprint(line, |fingerprint| {
            if let Some(owners) = expected.get(&fingerprint) {
                matched.extend(owners.iter().copied());
            }
            Ok(())
        });
    }
}

fn run_git_stdout_bounded<I, S>(
    path: &Path,
    arguments: I,
    limit: usize,
) -> anyhow::Result<(ExitStatus, Vec<u8>)>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut child = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("Git stdout was unavailable"))?;
    let mut retained = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let count = stdout.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        if retained.len().saturating_add(count) > limit {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("Git output exceeded the {limit}-byte safety limit");
        }
        retained.extend_from_slice(&chunk[..count]);
    }
    Ok((child.wait()?, retained))
}

#[derive(Debug)]
struct SnapshotEntry {
    mode: String,
    oid: String,
    path: String,
}

fn materialize_batch_entries(
    reader: &mut BufReader<impl Read>,
    entries: &[SnapshotEntry],
    destination: &Path,
) -> anyhow::Result<()> {
    let mut snapshot_bytes = 0_u64;
    for entry in entries {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        if header.len() > 512 {
            anyhow::bail!("Git returned an oversized batch header");
        }
        let mut fields = header.split_whitespace();
        let oid = fields.next().unwrap_or_default();
        let kind = fields.next().unwrap_or_default();
        let blob_size = fields
            .next()
            .ok_or_else(|| anyhow::anyhow!("Git returned a malformed batch header"))?
            .parse::<u64>()?;
        if oid != entry.oid || kind != "blob" {
            anyhow::bail!("Git returned an unexpected batch object");
        }
        snapshot_bytes = snapshot_bytes
            .checked_add(blob_size)
            .ok_or_else(|| anyhow::anyhow!("repository snapshot size overflowed"))?;
        if snapshot_bytes > MAX_SNAPSHOT_BYTES {
            anyhow::bail!("repository snapshot exceeds the 512 MiB safety limit");
        }
        if blob_size <= MAX_BLOB_BYTES as u64 {
            let mut content = vec![0_u8; blob_size as usize];
            reader.read_exact(&mut content)?;
            let target = destination.join(&entry.path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // Git symlinks are written as inert regular files containing the
            // link target. Provider tools cannot follow them into host data.
            std::fs::write(&target, content)?;
            make_snapshot_file_read_only(&target, entry.mode == "100755")?;
        } else {
            let copied = std::io::copy(&mut reader.by_ref().take(blob_size), &mut std::io::sink())?;
            if copied != blob_size {
                anyhow::bail!("Git ended an oversized blob early");
            }
        }
        let mut newline = [0_u8; 1];
        reader.read_exact(&mut newline)?;
        if newline != [b'\n'] {
            anyhow::bail!("Git returned a malformed batch separator");
        }
    }
    Ok(())
}

fn make_snapshot_file_read_only(path: &Path, executable: bool) -> anyhow::Result<()> {
    let mut permissions = std::fs::metadata(path)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(if executable { 0o555 } else { 0o444 });
    }
    #[cfg(not(unix))]
    {
        let _ = executable;
        permissions.set_readonly(true);
    }
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

pub struct DisposableClone {
    directory: TempDir,
    pub commit: String,
}

/// The only workspace root passed to repository-aware provider runs.
///
/// Callers cannot choose a checkout destination: every frozen repository is
/// materialized below this disposable root as `repository-N`. Dropping the
/// guard removes the whole source snapshot whether the owning operation
/// succeeds, errors, times out, or is cancelled.
pub(crate) struct ProviderSnapshotWorkspace {
    directory: Option<TempDir>,
    lease: Option<File>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SnapshotPurpose {
    Analysis,
    Map,
}

impl ProviderSnapshotWorkspace {
    pub(crate) fn new(purpose: SnapshotPurpose) -> anyhow::Result<Self> {
        Self::new_in(purpose, &std::env::temp_dir())
    }

    fn new_in(purpose: SnapshotPurpose, parent: &Path) -> anyhow::Result<Self> {
        with_snapshot_cleanup_lock(parent, || {
            cleanup_stale_provider_snapshots_unlocked(parent)?;
            let prefix = match purpose {
                SnapshotPurpose::Analysis => "codecaddie-multi-repository-scan-",
                SnapshotPurpose::Map => "codecaddie-map-",
            };
            let directory = tempfile::Builder::new().prefix(prefix).tempdir_in(parent)?;
            let lease_path = directory.path().join(SNAPSHOT_LEASE_FILE);
            let lease = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(lease_path)?;
            lease.lock_exclusive()?;
            Ok(Self {
                directory: Some(directory),
                lease: Some(lease),
            })
        })
    }

    pub(crate) fn path(&self) -> &Path {
        self.directory
            .as_ref()
            .expect("provider snapshot directory remains present until drop")
            .path()
    }

    pub(crate) fn snapshot_repository(
        &self,
        index: usize,
        repository: &LocalRepository,
        requested_commit: &str,
    ) -> anyhow::Result<(String, String)> {
        let directory_name = format!("repository-{index}");
        let destination = self.path().join(&directory_name);
        if destination.exists() {
            anyhow::bail!("repository snapshot destination already exists");
        }
        let commit = repository.disposable_clone_into(requested_commit, &destination)?;
        debug_assert!(destination.starts_with(self.path()));
        Ok((directory_name, commit))
    }
}

impl Drop for ProviderSnapshotWorkspace {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            let _ = FileExt::unlock(&lease);
            drop(lease);
        }
        if let Some(directory) = self.directory.take() {
            let _ = directory.close();
        }
    }
}

const SNAPSHOT_LEASE_FILE: &str = ".codecaddie-snapshot-lease";
const SNAPSHOT_CLEANUP_LOCK_FILE: &str = ".codecaddie-snapshot-cleanup.lock";
const SNAPSHOT_PREFIXES: [&str; 2] = ["codecaddie-multi-repository-scan-", "codecaddie-map-"];

/// Removes only snapshot directories whose OS lease is no longer held. The
/// advisory lock survives neither a normal drop nor a process crash, so the
/// next map or analysis run can clean source left by an interrupted process
/// without touching a concurrently active CodeCaddie snapshot.
#[cfg(test)]
fn cleanup_stale_provider_snapshots(parent: &Path) -> anyhow::Result<()> {
    with_snapshot_cleanup_lock(parent, || cleanup_stale_provider_snapshots_unlocked(parent))
}

/// Serializes cleanup with the small interval between creating a new snapshot
/// directory and acquiring its per-snapshot lease. Without this parent lock,
/// another process could briefly observe the new lease file as unlocked and
/// mistake live work for crash debris.
fn with_snapshot_cleanup_lock<T>(
    parent: &Path,
    operation: impl FnOnce() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(parent.join(SNAPSHOT_CLEANUP_LOCK_FILE))?;
    lock.lock_exclusive()?;
    let result = operation();
    FileExt::unlock(&lock)?;
    result
}

fn cleanup_stale_provider_snapshots_unlocked(parent: &Path) -> anyhow::Result<()> {
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !SNAPSHOT_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            continue;
        }
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !metadata.file_type().is_dir() {
            continue;
        }
        let lease_path = path.join(SNAPSHOT_LEASE_FILE);
        let lease = match OpenOptions::new().read(true).write(true).open(lease_path) {
            Ok(lease) => lease,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) if snapshot_lease_is_held(&error) => continue,
            Err(error) => return Err(error.into()),
        };
        match lease.try_lock_exclusive() {
            Ok(()) => {
                FileExt::unlock(&lease)?;
                drop(lease);
                if let Err(error) = fs::remove_dir_all(path)
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    return Err(error.into());
                }
            }
            Err(error) if snapshot_lease_is_held(&error) => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn snapshot_lease_is_held(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock
        || (cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33)))
}

impl DisposableClone {
    pub fn path(&self) -> &Path {
        self.directory.path()
    }
}

fn validate_relative_path(path: &str) -> anyhow::Result<()> {
    let candidate = Path::new(path);
    if candidate.is_absolute()
        || candidate.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        anyhow::bail!("evidence path escapes the repository");
    }
    Ok(())
}

/// Wall-clock ceiling for one short git child (`rev-parse`, `cat-file -e`,
/// `ls-tree --name-only`). Mirrors the provider-runner posture: no spawned
/// child may hold the core hostage. Long-lived streaming children
/// (`cat-file --batch`, `grep`) are bounded separately by their byte limits
/// and explicit kill paths.
const GIT_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
/// Collected git stdout for the short helpers stays within the same bound
/// the snapshot tree listing already uses.
const MAX_GIT_STDOUT_BYTES: usize = 64 * 1024 * 1024;

fn run_git<I, S>(path: &Path, arguments: I) -> anyhow::Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new("git");
    command.arg("-C").arg(path).args(arguments);
    run_child_bounded(command, GIT_COMMAND_TIMEOUT)
}

/// Runs a child to completion under a wall-clock timeout with bounded
/// stdout, the same containment pattern the provider runner applies to its
/// children. Stderr is discarded at spawn: like provider stderr, git stderr
/// may echo repository paths or refs and must never reach a caller whose
/// errors travel over the desktop IPC boundary.
fn run_child_bounded(mut command: Command, timeout: std::time::Duration) -> anyhow::Result<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("Git stdout was unavailable"))?;
    let reader = std::thread::spawn(move || -> std::io::Result<(Vec<u8>, bool)> {
        let mut retained = Vec::new();
        let mut truncated = false;
        let mut chunk = [0_u8; 8192];
        loop {
            let count = stdout.read(&mut chunk)?;
            if count == 0 {
                return Ok((retained, truncated));
            }
            // Keep draining past the cap so the child never blocks on a
            // full pipe; only retention stops.
            if retained.len().saturating_add(count) > MAX_GIT_STDOUT_BYTES {
                truncated = true;
            } else {
                retained.extend_from_slice(&chunk[..count]);
            }
        }
    });
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            // The reader thread is left to drain and exit on its own: a
            // surviving descendant of the killed child could hold the pipe
            // open, and error reporting must not wait on it.
            drop(reader);
            anyhow::bail!(
                "Git did not finish within the {}-second safety limit",
                timeout.as_secs()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let (stdout, truncated) = reader
        .join()
        .map_err(|_| anyhow::anyhow!("the Git output reader failed"))??;
    if truncated {
        anyhow::bail!("Git output exceeded the {MAX_GIT_STDOUT_BYTES}-byte safety limit");
    }
    Ok(Output {
        status,
        stdout,
        stderr: Vec::new(),
    })
}

fn git_text<I, S>(path: &Path, arguments: I) -> anyhow::Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = run_git(path, arguments)?;
    ensure_git(&output, "inspect repository")?;
    Ok(String::from_utf8(output.stdout)?.trim().into())
}

/// Maps a git failure to a fixed, sanitized message: the operation name and
/// exit status only. Git stderr is deliberately not forwarded — it may
/// contain repository paths, refs, or file names, and these errors surface
/// over the desktop IPC boundary. This mirrors the provider-stderr policy in
/// `provider.rs`.
fn ensure_git(output: &Output, operation: &str) -> anyhow::Result<()> {
    if output.status.success() {
        return Ok(());
    }
    match output.status.code() {
        Some(code) => anyhow::bail!("could not {operation}: git exited with status {code}"),
        None => anyhow::bail!("could not {operation}: git was terminated by a signal"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_fingerprints_cover_multiword_fragments_and_are_capped() {
        let source = field_fingerprints(&["customer pricing override approved".into()]).unwrap();
        let fragment = field_fingerprints(&["pricing override approved".into()]).unwrap();
        assert!(
            fragment
                .keys()
                .any(|fingerprint| source.contains_key(fingerprint))
        );

        let mut adversarial = String::new();
        for index in 0..300_000 {
            use std::fmt::Write as _;
            write!(&mut adversarial, "w{index:06} ").unwrap();
        }
        assert!(field_fingerprints(&[adversarial]).is_err());
    }

    #[test]
    fn dictionary_phrases_and_single_identifiers_no_longer_fingerprint() {
        // A two-word technical phrase or a lone long identifier inside
        // ordinary rationale prose is coordinate-adjacent vocabulary, not
        // source retention; sharing one with a README line must not redact
        // a valid report.
        let fields =
            vec!["The authentication middleware validates every session cookie.".to_string()];
        let table = field_fingerprints(&fields).unwrap();
        let mut matched = std::collections::BTreeSet::new();
        collect_matching_fields(
            "uses authentication middleware for sessions",
            &table,
            &mut matched,
        );
        assert!(matched.is_empty());

        let fields =
            vec!["the materialize_report_for_repositories function validates output".to_string()];
        let table = field_fingerprints(&fields).unwrap();
        let mut matched = std::collections::BTreeSet::new();
        collect_matching_fields(
            "calls materialize_report_for_repositories here",
            &table,
            &mut matched,
        );
        assert!(matched.is_empty());

        // Three-word verbatim runs of 24+ characters still match, and the
        // match is attributed to the specific field that carried it.
        let fields = vec![
            "a clean conclusion".to_string(),
            "quotes internal reconciliation token verbatim".to_string(),
        ];
        let table = field_fingerprints(&fields).unwrap();
        let mut matched = std::collections::BTreeSet::new();
        collect_matching_fields(
            "the internal reconciliation token stays local",
            &table,
            &mut matched,
        );
        assert_eq!(matched.into_iter().collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn source_retention_scan_streams_past_the_previous_64_mib_limit() {
        let (directory, repository) = repository();
        let line = b"ordinary repository filler text\n";
        let target_size = 8 * 1024 * 1024 - 1024;
        let mut body = Vec::with_capacity(target_size);
        while body.len() + line.len() <= target_size {
            body.extend_from_slice(line);
        }
        for index in 0..9 {
            fs::write(directory.path().join(format!("large-{index}.txt")), &body).unwrap();
        }
        Command::new("git")
            .arg("-C")
            .arg(directory.path())
            .args(["add", "."])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(directory.path())
            .args(["commit", "--quiet", "-m", "large text fixture"])
            .status()
            .unwrap();

        assert!(
            repository
                .narrative_fields_matching_source(
                    "HEAD",
                    &["This bounded provider conclusion is absent from the repository.".into()]
                )
                .unwrap()
                .is_empty()
        );
    }
    use std::{fs, process::Stdio};

    fn repository() -> (TempDir, LocalRepository) {
        let directory = tempfile::tempdir().unwrap();
        Command::new("git")
            .args(["init", "--quiet"])
            .arg(directory.path())
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(directory.path())
            .args(["config", "user.email", "test@codecaddie.local"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(directory.path())
            .args(["config", "user.name", "CodeCaddie Test"])
            .status()
            .unwrap();
        fs::write(
            directory.path().join("tenant.rs"),
            "fn scoped() {\n    tenant_id();\n}\n",
        )
        .unwrap();
        fs::create_dir_all(directory.path().join("ExampleCo.Api/Logging")).unwrap();
        fs::write(
            directory
                .path()
                .join("ExampleCo.Api/Logging/LoggingConfig.cs"),
            "line one\nline two\n",
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(directory.path())
            .args(["add", "."])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(directory.path())
            .args(["commit", "--quiet", "-m", "fixture"])
            .stdout(Stdio::null())
            .status()
            .unwrap();
        let repo = LocalRepository::attach("repo", directory.path()).unwrap();
        (directory, repo)
    }

    mod snapshot_lifecycle_assurance {
        include!("repository_snapshot_lifecycle_assurance.rs");
    }

    #[cfg(unix)]
    #[test]
    fn a_hanging_git_child_is_killed_at_the_wall_clock_limit() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let fake_git = directory.path().join("git");
        fs::write(&fake_git, "#!/bin/sh\nexec sleep 30\n").unwrap();
        fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o755)).unwrap();
        let mut command = Command::new(&fake_git);
        command.arg("rev-parse");
        let started = std::time::Instant::now();
        let error = run_child_bounded(command, std::time::Duration::from_millis(300))
            .expect_err("a hanging git child must time out");
        assert!(error.to_string().contains("safety limit"));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "the hanging child was not killed promptly"
        );
    }

    #[test]
    fn git_failures_surface_the_operation_and_exit_status_without_stderr() {
        let (_directory, repository) = repository();
        let error = repository
            .resolve_commit("cc-no-such-ref-cc")
            .expect_err("an unknown ref cannot resolve");
        let message = error.to_string();
        assert!(
            message.starts_with("could not inspect repository: git exited with status"),
            "unexpected git failure shape: {message}"
        );
        // Raw git stderr would echo the requested ref and 'fatal:' prose;
        // neither may cross into errors that travel over the desktop IPC.
        assert!(!message.contains("cc-no-such-ref-cc"));
        assert!(!message.to_lowercase().contains("fatal"));
    }

    #[test]
    fn provider_snapshot_has_no_git_history_and_writes_are_discarded() {
        let (_directory, repository) = repository();
        let before_head = repository.head().unwrap();
        let before_status = git_text(&repository.path, ["status", "--porcelain=v1"]).unwrap();
        {
            let clone = repository.disposable_clone(&before_head).unwrap();
            assert!(!clone.path().join(".git").exists());
            fs::write(clone.path().join("provider-created.txt"), "discarded").unwrap();
        }
        assert_eq!(repository.head().unwrap(), before_head);
        assert_eq!(
            git_text(&repository.path, ["status", "--porcelain=v1"]).unwrap(),
            before_status
        );
        assert!(!repository.path.join("provider-created.txt").exists());
    }

    #[test]
    fn provider_snapshot_is_exact_commit_read_only_checkout_confined_and_removed() {
        let (_directory, repository) = repository();
        let frozen_commit = repository.head().unwrap();
        fs::write(
            repository.path.join("tenant.rs"),
            "fn changed_after_frozen_commit() {}\n",
        )
        .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repository.path)
            .args(["add", "tenant.rs"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&repository.path)
            .args(["commit", "--quiet", "-m", "later fixture"])
            .status()
            .unwrap();
        let current_head = repository.head().unwrap();
        fs::write(
            repository.path.join("tenant.rs"),
            "fn uncommitted_checkout_change() {}\n",
        )
        .unwrap();

        let workspace = ProviderSnapshotWorkspace::new(SnapshotPurpose::Analysis).unwrap();
        let workspace_path = workspace.path().to_path_buf();
        let (directory_name, resolved) = workspace
            .snapshot_repository(0, &repository, &frozen_commit)
            .unwrap();
        let snapshot_root = workspace.path().join(directory_name);
        let snapshot_file = snapshot_root.join("tenant.rs");
        assert_eq!(resolved, frozen_commit);
        assert_eq!(
            fs::read_to_string(&snapshot_file).unwrap(),
            "fn scoped() {\n    tenant_id();\n}\n"
        );
        assert!(!snapshot_root.join(".git").exists());
        assert!(
            fs::metadata(&snapshot_file)
                .unwrap()
                .permissions()
                .readonly()
        );
        assert!(
            fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&snapshot_file)
                .is_err(),
            "frozen source files must not be writable by the provider process"
        );
        assert!(!workspace_path.starts_with(&repository.path));
        assert!(!repository.path.starts_with(&workspace_path));
        assert!(
            workspace
                .snapshot_repository(0, &repository, &current_head)
                .is_err(),
            "one repository index cannot be redirected or overwritten"
        );

        drop(workspace);
        assert!(!workspace_path.exists());
        assert_eq!(repository.head().unwrap(), current_head);
        assert_eq!(
            fs::read_to_string(repository.path.join("tenant.rs")).unwrap(),
            "fn uncommitted_checkout_change() {}\n"
        );
        assert!(repository.working_tree_dirty().unwrap());
    }

    #[tokio::test]
    async fn privacy_adversarial_snapshot_cleanup_after_success_failure_and_map_generation() {
        let success = ProviderSnapshotWorkspace::new(SnapshotPurpose::Analysis).unwrap();
        let success_path = success.path().to_path_buf();
        drop(success);
        assert!(!success_path.exists());

        let mut failure_path = None;
        let provider_failure = (|| -> anyhow::Result<()> {
            let workspace = ProviderSnapshotWorkspace::new(SnapshotPurpose::Analysis)?;
            failure_path = Some(workspace.path().to_path_buf());
            anyhow::bail!("simulated provider failure")
        })();
        assert!(provider_failure.is_err());
        assert!(!failure_path.unwrap().exists());

        let map = ProviderSnapshotWorkspace::new(SnapshotPurpose::Map).unwrap();
        let map_path = map.path().to_path_buf();
        assert!(
            map_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("codecaddie-map-")
        );
        fs::write(map.path().join("codecaddie-map.json"), b"{}").unwrap();
        drop(map);
        assert!(!map_path.exists());
    }

    #[tokio::test]
    async fn privacy_adversarial_snapshot_cleanup_after_timeout_and_task_cancellation() {
        use std::sync::{Arc, Mutex};

        let timeout_path = Arc::new(Mutex::new(None));
        let recorded_timeout_path = Arc::clone(&timeout_path);
        let timed = tokio::time::timeout(std::time::Duration::from_millis(20), async move {
            let workspace = ProviderSnapshotWorkspace::new(SnapshotPurpose::Analysis).unwrap();
            *recorded_timeout_path.lock().unwrap() = Some(workspace.path().to_path_buf());
            std::future::pending::<()>().await;
            drop(workspace);
        })
        .await;
        assert!(timed.is_err());
        assert!(!timeout_path.lock().unwrap().clone().unwrap().exists());

        let cancel_path = Arc::new(Mutex::new(None));
        let recorded_cancel_path = Arc::clone(&cancel_path);
        let (ready, prepared) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let workspace = ProviderSnapshotWorkspace::new(SnapshotPurpose::Analysis).unwrap();
            *recorded_cancel_path.lock().unwrap() = Some(workspace.path().to_path_buf());
            let _ = ready.send(());
            std::future::pending::<()>().await;
            drop(workspace);
        });
        prepared.await.unwrap();
        let path = cancel_path.lock().unwrap().clone().unwrap();
        assert!(path.exists());
        task.abort();
        let _ = task.await;
        assert!(!path.exists());
    }

    #[test]
    fn privacy_adversarial_snapshot_cleanup_recovers_after_restart_without_touching_live_work() {
        let parent = tempfile::tempdir().unwrap();
        let stale_analysis = parent
            .path()
            .join("codecaddie-multi-repository-scan-crashed-process");
        let stale_map = parent.path().join("codecaddie-map-crashed-process");
        let live_analysis = parent
            .path()
            .join("codecaddie-multi-repository-scan-live-process");
        for path in [&stale_analysis, &stale_map, &live_analysis] {
            fs::create_dir(path).unwrap();
            File::create(path.join(SNAPSHOT_LEASE_FILE)).unwrap();
            fs::write(path.join("source-canary.txt"), "PRIVATE SOURCE SENTINEL").unwrap();
        }
        let live_lease = OpenOptions::new()
            .read(true)
            .write(true)
            .open(live_analysis.join(SNAPSHOT_LEASE_FILE))
            .unwrap();
        live_lease.lock_exclusive().unwrap();

        let restarted =
            ProviderSnapshotWorkspace::new_in(SnapshotPurpose::Analysis, parent.path()).unwrap();
        let restarted_path = restarted.path().to_path_buf();
        assert!(restarted_path.exists());
        assert!(!stale_analysis.exists());
        assert!(!stale_map.exists());
        assert!(
            live_analysis.exists(),
            "a lease held by another live operation must never be removed"
        );
        drop(restarted);
        assert!(!restarted_path.exists());

        FileExt::unlock(&live_lease).unwrap();
        drop(live_lease);
        cleanup_stale_provider_snapshots(parent.path()).unwrap();
        assert!(
            !live_analysis.exists(),
            "a lease released by a crashed or exited process becomes recoverable"
        );
    }

    #[test]
    fn snapshot_cleanup_recognizes_platform_lock_contention() {
        assert!(snapshot_lease_is_held(&std::io::Error::from(
            std::io::ErrorKind::WouldBlock
        )));
        #[cfg(windows)]
        for code in [32, 33] {
            assert!(snapshot_lease_is_held(&std::io::Error::from_raw_os_error(
                code
            )));
        }
    }

    #[cfg(unix)]
    #[test]
    fn provider_snapshot_never_materializes_live_symlinks() {
        use std::os::unix::fs::symlink;
        let (directory, repository) = repository();
        let outside = directory.path().join("outside-secret");
        fs::write(&outside, "host canary").unwrap();
        symlink(&outside, directory.path().join("leak")).unwrap();
        Command::new("git")
            .arg("-C")
            .arg(directory.path())
            .args(["add", "leak"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(directory.path())
            .args(["commit", "--quiet", "-m", "symlink fixture"])
            .status()
            .unwrap();
        let snapshot = repository.disposable_clone("HEAD").unwrap();
        let leak = snapshot.path().join("leak");
        assert!(
            !fs::symlink_metadata(&leak)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_ne!(fs::read_to_string(leak).unwrap(), "host canary");
    }

    #[test]
    fn evidence_is_bound_to_commit_blob_and_line_range() {
        let (_directory, repository) = repository();
        let evidence = repository
            .evidence("HEAD", "tenant.rs", 2, 2, EvidenceKind::Implementation)
            .unwrap();
        assert_eq!(
            repository.read_evidence(&evidence).unwrap(),
            "    tenant_id();"
        );
        let mut changed = evidence.clone();
        changed.content_hash = "bad".into();
        assert!(repository.read_evidence(&changed).is_err());
        assert!(
            repository
                .evidence("HEAD", "../secret", 1, 1, EvidenceKind::Implementation)
                .is_err()
        );
    }

    #[test]
    fn evidence_normalizes_case_only_paths_and_clamps_the_trailing_line() {
        let (_directory, repository) = repository();
        let evidence = repository
            .evidence(
                "HEAD",
                "ExampleCo.API/Logging/LoggingConfig.cs",
                2,
                99,
                EvidenceKind::Configuration,
            )
            .unwrap();
        assert_eq!(evidence.path, "ExampleCo.Api/Logging/LoggingConfig.cs");
        assert_eq!(evidence.start_line, 2);
        assert_eq!(evidence.end_line, 2);
        assert_eq!(repository.read_evidence(&evidence).unwrap(), "line two");
    }
}
