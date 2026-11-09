//! SP1's shared-memory files.
//!
//! SP1's native executor shares the guest's memory and its trace ring with the prover through
//! files in `/dev/shm` (`sp1_<id>_m`, `sp1_<id>_t`, and the ring's two semaphores), under a new
//! random id each run. SP1 removes them only when the prover exits normally. A prover that is
//! killed (a timeout, an OOM kill, the service abandoning it) or that exits on a fatal SP1 task
//! error leaves them behind, still holding their memory and charged to the container's, until
//! the container is recreated; enough of them leave the next proof too little room in
//! `/dev/shm`, and its executor dies of SIGBUS.

use std::{
    collections::HashSet,
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

/// Where SP1's files are, and where to see which processes hold them.
#[derive(Clone, Debug)]
pub struct Sp1Shm {
    dir: PathBuf,
    proc: PathBuf,
}

/// What [`Sp1Shm::clear_stale`] did.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Cleared {
    pub removed: usize,
    /// The memory the removed files held: their allocated size, not their length, since
    /// SP1's guest memory file is a sparse file terabytes long.
    pub bytes: u64,
    /// SP1 files left because a live process has them open or mapped.
    pub in_use: usize,
}

impl Default for Sp1Shm {
    fn default() -> Self {
        Self::new("/dev/shm", "/proc")
    }
}

impl Sp1Shm {
    pub fn new(dir: impl Into<PathBuf>, proc: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            proc: proc.into(),
        }
    }

    /// Remove every SP1 file that no process has open or mapped. A live SP1 process holds each
    /// of its files from the moment it creates it until it removes it, so what nothing holds
    /// was left by a prover that is gone. Without `/dev/shm` there is nothing to do; without
    /// `/proc` nothing can tell which files are in use, so nothing is removed.
    pub fn clear_stale(&self) -> io::Result<Cleared> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Cleared::default()),
            Err(err) => return Err(err),
        };
        let mut candidates = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            if !name.to_str().is_some_and(is_sp1_name) {
                continue;
            }
            // Gone meanwhile: nothing to remove.
            let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if metadata.is_file() {
                candidates.push((name, FileId::of(&metadata), metadata.blocks() * 512));
            }
        }
        let mut cleared = Cleared::default();
        if candidates.is_empty() {
            return Ok(cleared);
        }
        let held = self.held()?;
        for (name, id, bytes) in candidates {
            if held.contains(&id) {
                cleared.in_use += 1;
                continue;
            }
            // Failing means it went meanwhile, or it is another user's to remove.
            if fs::remove_file(self.dir.join(&name)).is_ok() {
                cleared.removed += 1;
                cleared.bytes += bytes;
            }
        }
        Ok(cleared)
    }

    /// The files any process has open or mapped. A file is known by its device and inode, not
    /// its name: glibc creates a named semaphore under a temporary name, maps it, links it to
    /// its name and removes the temporary one, so its creator's mapping shows a name it no
    /// longer has.
    ///
    /// A live process this user cannot inspect fails the whole scan when it could hold a file
    /// this user can remove: one running as this user (a process that is not dumpable hides
    /// its mappings and descriptors from its own user, who can still remove its files), or any
    /// process when this user is root. Another user's process cannot open SP1's files, which
    /// only their owner may read.
    fn held(&self) -> io::Result<HashSet<FileId>> {
        let me = uids(&fs::read_to_string(self.proc.join("self/status"))?)
            .map(|[_, effective, _, _]| effective)
            .ok_or_else(|| io::Error::other("cannot read this process's user"))?;
        let mut held = HashSet::new();
        for process in fs::read_dir(&self.proc)? {
            let process = process?.path();
            let is_pid = process
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
            if !is_pid {
                continue;
            }
            let Err(err) = inspect(&process, &mut held) else {
                continue;
            };
            match fs::read_to_string(process.join("status")) {
                // Gone meanwhile.
                Err(gone) if gone.kind() == io::ErrorKind::NotFound => {}
                // A zombie holds nothing.
                Ok(status) if state(&status) == Some('Z') => {}
                Ok(status) if me != 0 && uids(&status).is_some_and(|ids| !ids.contains(&me)) => {}
                _ => {
                    return Err(io::Error::new(
                        err.kind(),
                        format!(
                            "cannot see which files process {} holds: {err}",
                            process.display()
                        ),
                    ))
                }
            }
        }
        Ok(held)
    }
}

/// Add the files `process` maps or has open to `held`.
fn inspect(process: &Path, held: &mut HashSet<FileId>) -> io::Result<()> {
    let maps = fs::read_to_string(process.join("maps"))?;
    held.extend(maps.lines().filter_map(FileId::of_mapping));
    for fd in fs::read_dir(process.join("fd"))? {
        match fs::metadata(fd?.path()) {
            Ok(metadata) => {
                held.insert(FileId::of(&metadata));
            }
            // Closed meanwhile.
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

/// The real, effective, saved and filesystem user ids in a `/proc/<pid>/status`.
fn uids(status: &str) -> Option<[u32; 4]> {
    let line = status.lines().find_map(|l| l.strip_prefix("Uid:"))?;
    let ids: Vec<u32> = line
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    ids.try_into().ok()
}

/// The state letter in a `/proc/<pid>/status`.
fn state(status: &str) -> Option<char> {
    status
        .lines()
        .find_map(|l| l.strip_prefix("State:"))?
        .trim()
        .chars()
        .next()
}

/// A file's device and inode.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct FileId {
    dev: u64,
    ino: u64,
}

impl FileId {
    fn of(metadata: &fs::Metadata) -> Self {
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
        }
    }

    /// The file a line of `/proc/<pid>/maps` maps (`address perms offset major:minor inode
    /// path`), if it maps one.
    fn of_mapping(line: &str) -> Option<Self> {
        let mut fields = line.split_whitespace();
        let (major, minor) = fields.nth(3)?.split_once(':')?;
        let ino = fields.next()?.parse().ok().filter(|&ino| ino != 0)?;
        let major = u64::from_str_radix(major, 16).ok()?;
        let minor = u64::from_str_radix(minor, 16).ok()?;
        Some(Self {
            dev: makedev(major, minor),
            ino,
        })
    }
}

/// The device number glibc's `makedev` makes of a major and a minor number, as `stat` reports
/// it.
fn makedev(major: u64, minor: u64) -> u64 {
    ((major & 0xffff_f000) << 32)
        | ((major & 0x0000_0fff) << 8)
        | ((minor & 0xffff_ff00) << 12)
        | (minor & 0x0000_00ff)
}

/// SP1's names: `sp1_<id>_m` (guest memory), `sp1_<id>_t` (trace ring), and the ring's
/// semaphores, which glibc keeps as `sem.sp1_<id>_t_filled` and `sem.sp1_<id>_t_empty`. `<id>`
/// is a v4 UUID in padded URL-safe base64.
fn is_sp1_name(name: &str) -> bool {
    let named = match name.strip_prefix("sem.") {
        Some(semaphore) => semaphore
            .strip_suffix("_t_filled")
            .or_else(|| semaphore.strip_suffix("_t_empty")),
        None => name.strip_suffix("_m").or_else(|| name.strip_suffix("_t")),
    };
    named
        .and_then(|named| named.strip_prefix("sp1_"))
        .is_some_and(|id| {
            id.len() == 24
                && id.ends_with("==")
                && id[..22]
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}

#[cfg(test)]
mod tests {
    use std::{fs::File, path::Path};

    use super::*;

    const ID: &str = "sp1_AbCdEfGhIjKl-_mNoPqRsT==";

    fn sp1_files(dir: &Path, id: &str) -> Vec<PathBuf> {
        ["{id}_m", "{id}_t", "sem.{id}_t_filled", "sem.{id}_t_empty"]
            .iter()
            .map(|pattern| {
                let path = dir.join(pattern.replace("{id}", id));
                fs::write(&path, vec![0u8; 4096]).unwrap();
                path
            })
            .collect()
    }

    /// A `/proc` that shows only `pids`, and this process as `self`, linked to the real ones: the
    /// host's other processes, some of which this user may not inspect, stay out of the test.
    fn proc_showing(dir: &Path, pids: &[u32]) -> PathBuf {
        let proc = dir.join("proc");
        fs::create_dir_all(&proc).unwrap();
        std::os::unix::fs::symlink("/proc/self", proc.join("self")).unwrap();
        for pid in pids {
            std::os::unix::fs::symlink(format!("/proc/{pid}"), proc.join(pid.to_string())).unwrap();
        }
        proc
    }

    /// A process's status with `uid` as all four of its user ids.
    fn status(uid: u32, state: &str) -> String {
        format!("Name:\tx\nState:\t{state}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n")
    }

    #[test]
    fn sp1_names_are_told_from_others() {
        for name in [
            // What a SIGKILLed SP1 6.3.1 prover left in a container's /dev/shm.
            "sp1_dxpcLYoSRAiTXh9D2BUD_w==_m",
            "sp1_dxpcLYoSRAiTXh9D2BUD_w==_t",
            "sem.sp1_dxpcLYoSRAiTXh9D2BUD_w==_t_empty",
            "sem.sp1_dxpcLYoSRAiTXh9D2BUD_w==_t_filled",
            "sp1_AbCdEfGhIjKl-_mNoPqRsT==_m",
            "sp1_AbCdEfGhIjKl-_mNoPqRsT==_t",
            "sem.sp1_AbCdEfGhIjKl-_mNoPqRsT==_t_filled",
            "sem.sp1_AbCdEfGhIjKl-_mNoPqRsT==_t_empty",
        ] {
            assert!(is_sp1_name(name), "{name}");
        }
        for name in [
            "sp1_AbCdEfGhIjKl-_mNoPqRsT==",
            "sp1_AbCdEfGhIjKl-_mNoPqRsT==_x",
            "sp1_AbCdEfGhIjKl-_mNoPqRsTuv_m",
            "sp1_AbCdEfGhIjKl.mNoPqRsTu==_m",
            "sem.sp1_AbCdEfGhIjKl-_mNoPqRsT==_filled",
            "sem.sp1_AbCdEfGhIjKl-_mNoPqRsT==_m",
            "pulse-shm-123",
            "sem.mysem",
            "sp1-native-runner-bin-abc",
        ] {
            assert!(!is_sp1_name(name), "{name}");
        }
    }

    #[test]
    fn files_nothing_holds_are_removed_and_the_rest_left() {
        let dir = tempfile::tempdir().unwrap();
        let stale = sp1_files(dir.path(), ID);
        let open = sp1_files(dir.path(), "sp1_0123456789abcdefghijkl==");
        let _held = File::open(&open[0]).unwrap();
        let other = dir.path().join("pulse-shm-123");
        fs::write(&other, b"x").unwrap();
        fs::create_dir(dir.path().join("sp1_zzzzzzzzzzzzzzzzzzzzzz==_m")).unwrap();

        let proc = proc_showing(&dir.path().join("view"), &[std::process::id()]);

        let cleared = Sp1Shm::new(dir.path(), proc).clear_stale().unwrap();

        assert_eq!((cleared.removed, cleared.in_use), (7, 1));
        assert!(stale.iter().all(|p| !p.exists()));
        assert!(open[0].exists(), "a file this process has open stays");
        assert!(open[1..].iter().all(|p| !p.exists()));
        assert!(other.exists());
        assert!(dir.path().join("sp1_zzzzzzzzzzzzzzzzzzzzzz==_m").is_dir());
    }

    /// `major:minor inode` of `path`, as `/proc/<pid>/maps` shows them.
    fn device_and_inode(path: &Path) -> String {
        let metadata = fs::metadata(path).unwrap();
        let dev = metadata.dev();
        let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff);
        let minor = (dev & 0xff) | ((dev >> 12) & !0xff);
        assert_eq!(makedev(major, minor), dev);
        format!("{major:02x}:{minor:02x} {}", metadata.ino())
    }

    #[test]
    fn what_a_removed_file_held_is_its_allocated_size() {
        let dir = tempfile::tempdir().unwrap();
        // Like SP1's guest memory: a terabyte long, a page written.
        let memory = File::create(dir.path().join(format!("{ID}_m"))).unwrap();
        memory.set_len(1 << 40).unwrap();
        std::os::unix::fs::FileExt::write_at(&memory, &[1u8; 4096], 0).unwrap();
        drop(memory);
        let proc = proc_showing(&dir.path().join("view"), &[]);

        let cleared = Sp1Shm::new(dir.path(), proc).clear_stale().unwrap();

        assert_eq!(cleared.removed, 1);
        assert!(cleared.bytes < 1 << 20, "{cleared:?}");
    }

    #[test]
    fn a_file_a_process_maps_or_has_open_stays_whatever_name_it_shows() {
        let dir = tempfile::tempdir().unwrap();
        let shm = dir.path().join("shm");
        fs::create_dir(&shm).unwrap();
        let files = sp1_files(&shm, ID);
        let proc = dir.path().join("proc");
        // A semaphore glibc created under a temporary name it then removed, so its mapping
        // shows that name; the guest memory mapped under its own; the ring open on a
        // descriptor; and the other semaphore held by nothing.
        fs::create_dir_all(proc.join("41")).unwrap();
        fs::write(
            proc.join("41/maps"),
            format!(
                "7f00-7f01 rw-s 00000000 {} {}/sem.Xa1b2C (deleted)\n\
                 7f02-7f03 r--s 00000000 {} {}\n\
                 7f04-7f05 rw-p 00000000 00:00 0 \n\
                 7f06-7f07 rw-p 00000000 00:00 0 [heap]\n",
                device_and_inode(&files[2]),
                shm.display(),
                device_and_inode(&files[0]),
                files[0].display(),
            ),
        )
        .unwrap();
        fs::create_dir_all(proc.join("41/fd")).unwrap();
        fs::write(proc.join("41/status"), status(1000, "S (sleeping)")).unwrap();
        fs::create_dir_all(proc.join("42/fd")).unwrap();
        fs::write(proc.join("42/maps"), "").unwrap();
        fs::write(proc.join("42/status"), status(1000, "S (sleeping)")).unwrap();
        std::os::unix::fs::symlink(&files[1], proc.join("42/fd/3")).unwrap();
        // Not a process.
        fs::create_dir_all(proc.join("self/fd")).unwrap();
        std::os::unix::fs::symlink(&files[3], proc.join("self/fd/3")).unwrap();
        fs::write(proc.join("self/status"), status(1000, "R (running)")).unwrap();

        let cleared = Sp1Shm::new(&shm, &proc).clear_stale().unwrap();

        assert_eq!(cleared.removed, 1);
        assert_eq!(cleared.in_use, 3);
        assert!(files[..3].iter().all(|p| p.exists()));
        assert!(!files[3].exists());
    }

    #[test]
    fn a_semaphore_its_creator_holds_under_a_removed_name_stays() {
        let dir = tempfile::tempdir().unwrap();
        let semaphore = dir.path().join(format!("sem.{ID}_t_filled"));
        let ready = dir.path().join("ready");
        // As glibc's sem_open creates one: a temporary file, held, linked to the name, and the
        // temporary name removed.
        let mut creator = std::process::Command::new("sh")
            .arg("-c")
            .arg(r#"exec 3<>"$1/sem.Xa1b2C" && ln "$1/sem.Xa1b2C" "$2" && rm "$1/sem.Xa1b2C" && : > "$3" && exec sleep 30 3>&3"#)
            .arg("sh")
            .arg(dir.path())
            .arg(&semaphore)
            .arg(&ready)
            .spawn()
            .unwrap();
        for _ in 0..100 {
            if ready.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(ready.exists(), "the creator never got ready");

        let proc = proc_showing(&dir.path().join("view"), &[creator.id()]);
        let cleared = Sp1Shm::new(dir.path(), proc).clear_stale();
        creator.kill().unwrap();
        creator.wait().unwrap();

        assert_eq!(cleared.unwrap().in_use, 1);
        assert!(semaphore.exists());
    }

    #[test]
    fn a_live_process_that_cannot_be_inspected_stops_the_cleanup_if_it_could_hold_the_files() {
        let dir = tempfile::tempdir().unwrap();
        let shm = dir.path().join("shm");
        fs::create_dir(&shm).unwrap();
        let proc = dir.path().join("proc");
        fs::create_dir_all(proc.join("self")).unwrap();
        // Its mappings cannot be read, as a process that is not dumpable hides them.
        fs::create_dir_all(proc.join("50/maps")).unwrap();
        fs::create_dir_all(proc.join("50/fd")).unwrap();
        // Gone, all but its directory.
        fs::create_dir_all(proc.join("51")).unwrap();
        let clear = |me: u32, held_by: u32, state: &str| {
            sp1_files(&shm, ID);
            fs::write(proc.join("self/status"), status(me, "R (running)")).unwrap();
            fs::write(proc.join("50/status"), status(held_by, state)).unwrap();
            Sp1Shm::new(&shm, &proc).clear_stale()
        };

        let err = clear(1000, 1000, "S (sleeping)").unwrap_err();
        assert!(err.to_string().contains("50"), "{err}");
        assert_eq!(fs::read_dir(&shm).unwrap().count(), 4, "nothing removed");
        assert!(
            clear(0, 2000, "S (sleeping)").is_err(),
            "root can remove anyone's files"
        );

        assert_eq!(
            clear(1000, 2000, "S (sleeping)").unwrap().removed,
            4,
            "another user's"
        );
        assert_eq!(
            clear(1000, 1000, "Z (zombie)").unwrap().removed,
            4,
            "a zombie's"
        );
    }

    #[test]
    fn nothing_is_removed_without_proc_and_nothing_happens_without_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let files = sp1_files(dir.path(), ID);
        let missing = dir.path().join("no-proc");
        assert!(Sp1Shm::new(dir.path(), &missing).clear_stale().is_err());
        assert!(files.iter().all(|p| p.exists()));
        assert_eq!(
            Sp1Shm::new(dir.path().join("no-shm"), "/proc")
                .clear_stale()
                .unwrap(),
            Cleared::default()
        );
    }
}
