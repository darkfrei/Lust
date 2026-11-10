// a game as one file: the program lust with the folder of the game (its lua files) glued to
// its end. a person double clicks that file and the game opens at once, on its own menu; no
// command line, no list of games. the file is made by the tool lust_pack (an AI) or by
// "lust pack <folder>".
//   the file = the program | the files | the trailer
//   the files: for each file the length of its path (u32), the path ('/' as separator),
//              the length of its data (u64), the data. all numbers little endian
//   the trailer: the length of the files (u64) and the magic LUSTGAME (8 bytes)
// at the start lust reads its own file; with a trailer it unpacks the files into a folder
// of the temp directory (named after the files, so the same game goes to the same folder)
// and runs that folder. the folder .lust (history) and dist (made files) are not packed;
// the file .id names the game, so its saves are found again (saves.rs)
use std::fs;
use std::path::{Component, Path, PathBuf};

const MAGIC: &[u8; 8] = b"LUSTGAME";
const TRAILER: usize = 16;
// a game is a few text files; this is far above that and far below the download limit
const LIMIT: u64 = 30 * 1024 * 1024;

// the files of a folder as paths with '/' and their data, sorted by path
pub fn collect(root: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::new();
    let mut total = 0u64;
    walk(root, "", &mut out, &mut total)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    if !out.iter().any(|(p, _)| p == "world.lua") {
        return Err(String::from("no world.lua in the folder"));
    }
    Ok(out)
}

fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, Vec<u8>)>, total: &mut u64) -> Result<(), String> {
    let rd = fs::read_dir(dir).map_err(|e| format!("{}: {}", dir.display(), e))?;
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || (prefix.is_empty() && name == "dist") {
            continue;
        }
        let path = entry.path();
        let rel = if prefix.is_empty() { name.clone() } else { format!("{}/{}", prefix, name) };
        if path.is_dir() {
            walk(&path, &rel, out, total)?;
        } else if path.is_file() {
            let data = fs::read(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
            *total += data.len() as u64;
            if *total > LIMIT {
                return Err(String::from("the folder is too big for one file (30 MB)"));
            }
            out.push((rel, data));
        }
    }
    Ok(())
}

// the files as one block of bytes
pub fn encode(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut b = Vec::new();
    for (p, d) in files {
        b.extend_from_slice(&(p.len() as u32).to_le_bytes());
        b.extend_from_slice(p.as_bytes());
        b.extend_from_slice(&(d.len() as u64).to_le_bytes());
        b.extend_from_slice(d);
    }
    b
}

fn take<'a>(b: &'a [u8], at: &mut usize, n: usize) -> Result<&'a [u8], String> {
    let end = at.checked_add(n).filter(|e| *e <= b.len()).ok_or("the packed files are cut off")?;
    let s = &b[*at..end];
    *at = end;
    Ok(s)
}

pub fn decode(b: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let (mut at, mut out) = (0usize, Vec::new());
    while at < b.len() {
        let n = u32::from_le_bytes(take(b, &mut at, 4)?.try_into().map_err(|_| "bad length")?) as usize;
        let path = String::from_utf8(take(b, &mut at, n)?.to_vec()).map_err(|_| "a path is not text")?;
        let m = u64::from_le_bytes(take(b, &mut at, 8)?.try_into().map_err(|_| "bad length")?) as usize;
        out.push((path, take(b, &mut at, m)?.to_vec()));
    }
    Ok(out)
}

// the program without any files glued to it, and the files if there are some
pub fn split(file: &[u8]) -> (&[u8], Option<&[u8]>) {
    if file.len() >= TRAILER && &file[file.len() - 8..] == MAGIC {
        let mut n = [0u8; 8];
        n.copy_from_slice(&file[file.len() - TRAILER..file.len() - 8]);
        let n = u64::from_le_bytes(n) as usize;
        if n <= file.len() - TRAILER {
            let start = file.len() - TRAILER - n;
            return (&file[..start], Some(&file[start..file.len() - TRAILER]));
        }
    }
    (file, None)
}

// the program (this file, or one with files already glued: they are replaced) and the folder
pub fn make(program: &[u8], root: &Path) -> Result<Vec<u8>, String> {
    let (bare, _) = split(program);
    let mut files = collect(root)?;
    // the name of the game for its saves (saves.rs): the folder it was packed from
    files.insert(0, (String::from(".id"), crate::saves::id_of(root).into_bytes()));
    let block = encode(&files);
    let mut out = bare.to_vec();
    out.extend_from_slice(&block);
    out.extend_from_slice(&(block.len() as u64).to_le_bytes());
    out.extend_from_slice(MAGIC);
    Ok(out)
}

// where the game file goes: dist/<name of the folder> with the extension of the program
pub fn target(root: &Path, program: &Path) -> PathBuf {
    let name = root.canonicalize().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_else(|| String::from("game"));
    let mut t = root.join("dist").join(name);
    if let Some(e) = program.extension() {
        t.set_extension(e);
    }
    t
}

// makes the game file of a folder; gives where it is and how big
pub fn pack(root: &Path, program: &Path) -> Result<(PathBuf, usize), String> {
    let bytes = fs::read(program).map_err(|e| format!("{}: {}", program.display(), e))?;
    let out = make(&bytes, root)?;
    let to = target(root, program);
    if let Some(d) = to.parent() {
        fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    fs::write(&to, &out).map_err(|e| format!("{}: {}", to.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&to, fs::Permissions::from_mode(0o755));
    }
    Ok((to, out.len()))
}

fn safe(path: &str) -> bool {
    !path.is_empty() && Path::new(path).components().all(|c| matches!(c, Component::Normal(_)))
}

// the files into a folder; a path that leaves the folder is refused
pub fn unpack(files: &[(String, Vec<u8>)], dest: &Path) -> Result<(), String> {
    for (p, d) in files {
        if !safe(p) {
            return Err(format!("a packed path is not allowed: {}", p));
        }
        let to = dest.join(p);
        if let Some(dir) = to.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        fs::write(&to, d).map_err(|e| format!("{}: {}", to.display(), e))?;
    }
    Ok(())
}

fn fnv(b: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for x in b {
        h ^= *x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

// the game glued to this program, unpacked into the temp directory; None for a plain lust
pub fn embedded() -> Result<Option<PathBuf>, String> {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(_) => return Ok(None),
    };
    let bytes = fs::read(&exe).map_err(|e| format!("{}: {}", exe.display(), e))?;
    let block = match split(&bytes).1 {
        Some(b) => b,
        None => return Ok(None),
    };
    let dest = std::env::temp_dir().join(format!("lust-game-{:016x}", fnv(block)));
    // the files again each time: a folder left by an older start may be damaged
    unpack(&decode(block)?, &dest)?;
    Ok(Some(dest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lust-pack-test-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_game_goes_into_a_file_and_comes_out_the_same() {
        let d = tmp("a");
        fs::create_dir_all(d.join("sys")).unwrap();
        fs::create_dir_all(d.join(".lust")).unwrap();
        fs::create_dir_all(d.join("dist")).unwrap();
        fs::write(d.join("world.lua"), "return {}").unwrap();
        fs::write(d.join("sys/a.lua"), "x = 1").unwrap();
        fs::write(d.join(".lust/history"), "no").unwrap();
        fs::write(d.join("dist/old.exe"), "no").unwrap();
        let program = b"MZ-the-program".to_vec();
        let one = make(&program, &d).unwrap();
        // packing a packed file replaces the files, the program stays
        let two = make(&one, &d).unwrap();
        assert_eq!(one, two);
        let (bare, block) = split(&two);
        assert_eq!(bare, &program[..]);
        let files = decode(block.unwrap()).unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(names, vec![".id", "sys/a.lua", "world.lua"]);
        assert!(files[0].1.starts_with(b"lust-pack-test-a"));
        let out = tmp("b");
        unpack(&files, &out).unwrap();
        assert_eq!(fs::read_to_string(out.join("sys/a.lua")).unwrap(), "x = 1");
        // a plain program has no files
        assert!(split(&program).1.is_none());
        let _ = fs::remove_dir_all(&d);
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn a_path_that_leaves_the_folder_is_refused() {
        let out = tmp("c");
        assert!(unpack(&[(String::from("../x.lua"), vec![1])], &out).is_err());
        assert!(unpack(&[(String::from("/x.lua"), vec![1])], &out).is_err());
        assert!(decode(&[5, 0, 0, 0, 1]).is_err());
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn a_folder_without_a_world_is_refused() {
        let d = tmp("d");
        fs::write(d.join("a.lua"), "").unwrap();
        assert!(collect(&d).is_err());
        let _ = fs::remove_dir_all(&d);
    }
}
