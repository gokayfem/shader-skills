//! A pack: several things in one file, each of which can be read without the rest.
//!
//! A truck built of pieces is one (`bake --kit`, `apps/bake/src/kit.rs`): each of its pieces
//! is a small scene of its own, and with them are the texts that say what the truck is. The
//! game reads the list of what is in the pack, and then only the pieces that are fitted to
//! the truck: what is not on it is never read.
//!
//! The file is eight bytes that say what it is, the length of its list in four more, the
//! list, and then each thing's bytes, one after another in the list's order. The list is
//! text, a line a thing: `KIND NAME LENGTH`.

use std::io::{self, Read, Seek, SeekFrom};

/// What a pack's file begins with.
pub const MAGIC: &[u8; 8] = b"SOLPACK1";

/// One thing in a pack: what kind of thing it is, what it is called, and where its bytes
/// are in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: String,
    pub name: String,
    pub start: u64,
    pub length: u64,
}

/// A pack of these things, each its kind, its name and its bytes, as the bytes of a file.
/// A kind and a name are single words.
pub fn write(things: &[(&str, &str, &[u8])]) -> Vec<u8> {
    let list: String = things
        .iter()
        .map(|(kind, name, bytes)| format!("{kind} {name} {}\n", bytes.len()))
        .collect();
    let mut file = Vec::from(*MAGIC);
    file.extend((list.len() as u32).to_le_bytes());
    file.extend(list.as_bytes());
    for (_, _, bytes) in things {
        file.extend(*bytes);
    }
    file
}

fn wrong(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

/// Reads the list of what is in a pack, from its beginning.
/// Every length in it is held to what the file has, so a damaged pack is refused rather than
/// asking for more memory than the file could fill.
pub fn read_list(file: &mut (impl Read + Seek)) -> io::Result<Vec<Entry>> {
    let size = file.seek(SeekFrom::End(0))?;
    file.seek(SeekFrom::Start(0))?;
    let mut head = [0; 12];
    file.read_exact(&mut head)?;
    if head[..8] != MAGIC[..] {
        return Err(wrong("it is not a pack"));
    }
    let length = u32::from_le_bytes([head[8], head[9], head[10], head[11]]);
    if 12 + u64::from(length) > size {
        return Err(wrong("its list runs past its end"));
    }
    let mut list = vec![0; length as usize];
    file.read_exact(&mut list)?;
    let list = String::from_utf8(list).map_err(|_| wrong("its list is not text"))?;
    let mut start = 12 + u64::from(length);
    let mut entries = Vec::new();
    for line in list.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let [kind, name, length] = words[..] else {
            return Err(wrong("a line of its list is not `KIND NAME LENGTH`"));
        };
        let length: u64 = length
            .parse()
            .map_err(|_| wrong("a length in its list is not a number"))?;
        let entry = Entry {
            kind: kind.to_owned(),
            name: name.to_owned(),
            start,
            length,
        };
        start = start
            .checked_add(length)
            .filter(|&end| end <= size)
            .ok_or_else(|| wrong("a thing in its list runs past its end"))?;
        entries.push(entry);
    }
    Ok(entries)
}

/// Reads one thing out of a pack.
/// An entry that runs past the file's end is refused before anything is read.
pub fn read(file: &mut (impl Read + Seek), entry: &Entry) -> io::Result<Vec<u8>> {
    let size = file.seek(SeekFrom::End(0))?;
    if entry
        .start
        .checked_add(entry.length)
        .is_none_or(|end| end > size)
    {
        return Err(wrong("that thing runs past the pack's end"));
    }
    file.seek(SeekFrom::Start(entry.start))?;
    let mut bytes =
        vec![0; usize::try_from(entry.length).map_err(|_| wrong("that thing is too big"))?];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn each_thing_in_a_pack_is_read_alone() {
        let file = write(&[
            ("text", "truck.kit", b"base cab\n"),
            ("piece", "cab", &[1, 2, 3, 4, 5]),
            ("piece", "bonnet", &[]),
            ("piece", "door_FL", &[9, 8]),
        ]);
        let mut file = Cursor::new(file);
        let list = read_list(&mut file).unwrap();
        let names: Vec<&str> = list.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["truck.kit", "cab", "bonnet", "door_FL"]);
        assert_eq!(list[0].kind, "text");
        // The last, without the others; and the first after it.
        assert_eq!(read(&mut file, &list[3]).unwrap(), [9, 8]);
        assert_eq!(read(&mut file, &list[0]).unwrap(), b"base cab\n");
        assert_eq!(read(&mut file, &list[2]).unwrap(), [0u8; 0]);
        assert_eq!(read(&mut file, &list[1]).unwrap(), [1, 2, 3, 4, 5]);
        // What is not a pack is refused.
        assert!(read_list(&mut Cursor::new(b"not a pack at all".to_vec())).is_err());
    }

    #[test]
    fn a_damaged_pack_is_refused_not_believed() {
        let good = write(&[("piece", "cab", &[1, 2, 3])]);
        // A list longer than the file.
        let mut long_list = good.clone();
        long_list[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(read_list(&mut Cursor::new(long_list)).is_err());
        // A thing longer than the file, and one so long its end overflows.
        for length in ["4", "18446744073709551615"] {
            let mut file = Vec::from(*MAGIC);
            let list = format!(
                "piece cab {length}
"
            );
            file.extend((list.len() as u32).to_le_bytes());
            file.extend(list.as_bytes());
            file.extend([1, 2, 3]);
            assert!(read_list(&mut Cursor::new(file)).is_err(), "{length}");
        }
        // An entry made up by hand is held to the file too.
        let entry = Entry {
            kind: "piece".to_owned(),
            name: "cab".to_owned(),
            start: 0,
            length: u64::MAX,
        };
        assert!(read(&mut Cursor::new(good), &entry).is_err());
    }
}
