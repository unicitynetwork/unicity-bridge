use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use crate::domain::{event::Event, ledger::Ledger};

const FILE_NAME: &str = "journal.jsonl";

#[derive(Debug)]
pub enum Journal {
    Memory,
    File { file: File, path: PathBuf },
}

impl Journal {
    pub fn open(dir: &Path) -> io::Result<(Self, Vec<Event>)> {
        fs::create_dir_all(dir)?;
        let path = dir.join(FILE_NAME);
        let events = match File::open(&path) {
            Ok(file) => read_events(file)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok((Self::File { file, path }, events))
    }

    pub fn append(&mut self, event: &Event) -> io::Result<()> {
        let Self::File { file, .. } = self else {
            return Ok(());
        };
        file.write_all(&line(event)?)?;
        file.sync_data()
    }

    pub fn compact(&mut self, ledger: &Ledger) -> io::Result<()> {
        let path = match self {
            Self::Memory => return Ok(()),
            Self::File { path, .. } => path.clone(),
        };
        let tmp = path.with_extension("jsonl.tmp");
        let mut snapshot = File::create(&tmp)?;
        snapshot.write_all(&line(&Event::Snapshot(Box::new(ledger.clone())))?)?;
        snapshot.sync_all()?;
        fs::rename(&tmp, &path)?;
        let file = OpenOptions::new().append(true).open(&path)?;
        *self = Self::File { file, path };
        Ok(())
    }
}

fn line(event: &Event) -> io::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(event)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn read_events(file: File) -> io::Result<Vec<Event>> {
    let lines: Vec<String> = BufReader::new(file).lines().collect::<io::Result<_>>()?;
    let last = lines.len().saturating_sub(1);
    let mut events = Vec::with_capacity(lines.len());
    for (index, text) in lines.iter().enumerate() {
        if text.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(text) {
            Ok(event) => events.push(event),
            Err(e) if index == last => {
                tracing::warn!(error = %e, "journal ends with a partial line; skipping it");
            }
            Err(e) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("journal line {}: {e}", index + 1),
                ))
            }
        }
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn requeued(id: &str, at_ms: u128) -> Event {
        Event::Requeued {
            id: id.to_string(),
            at_ms,
        }
    }

    #[test]
    fn append_then_open_replays_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let (mut journal, events) = Journal::open(dir.path()).unwrap();
        assert!(events.is_empty());
        journal.append(&requeued("a", 1)).unwrap();
        journal.append(&requeued("b", 2)).unwrap();
        drop(journal);

        let (_, events) = Journal::open(dir.path()).unwrap();
        assert_eq!(events, vec![requeued("a", 1), requeued("b", 2)]);
    }

    #[test]
    fn open_tolerates_trailing_partial_line() {
        let dir = tempfile::tempdir().unwrap();
        let (mut journal, _) = Journal::open(dir.path()).unwrap();
        journal.append(&requeued("a", 1)).unwrap();
        drop(journal);
        let path = dir.path().join(FILE_NAME);
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"{\"requeued\":{\"id\":\"b\",\"at").unwrap();
        drop(file);

        let (_, events) = Journal::open(dir.path()).unwrap();
        assert_eq!(events, vec![requeued("a", 1)]);
    }

    #[test]
    fn open_rejects_a_malformed_earlier_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        fs::write(
            &path,
            "{\"broken\"\n{\"requeued\":{\"id\":\"a\",\"at_ms\":1}}\n",
        )
        .unwrap();
        let err = Journal::open(dir.path()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn compact_leaves_one_snapshot_line_and_keeps_appending() {
        let dir = tempfile::tempdir().unwrap();
        let (mut journal, _) = Journal::open(dir.path()).unwrap();
        journal.append(&requeued("a", 1)).unwrap();
        journal.compact(&Ledger::default()).unwrap();
        journal.append(&requeued("b", 2)).unwrap();
        drop(journal);

        let path = dir.path().join(FILE_NAME);
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 2);
        assert!(!path.with_extension("jsonl.tmp").exists());
        let (_, events) = Journal::open(dir.path()).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], Event::Snapshot(_)));
        assert_eq!(events[1], requeued("b", 2));
    }

    #[test]
    fn memory_journal_keeps_nothing() {
        let mut journal = Journal::Memory;
        journal.append(&requeued("a", 1)).unwrap();
        journal.compact(&Ledger::default()).unwrap();
    }
}
