// Copyright 2021 Daniel Zwell.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::{convert::TryInto, error::Error, fs::File, io::Read, path::PathBuf, sync::Arc};

use blake3::OUT_LEN;
use memmap::Mmap;
use multi_semaphore::Semaphore;
use rayon::Scope;
use structopt::*;

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;

/// Compute a checksum using different logic depending on input characteristics. This function
/// handles locking to get the right amount of I/O parallelism.
///
/// `max_job_count` is the number of jobs that will be used for reading small files.  Small files
/// are checksummed using one thread each, in parallel.  Large files will be read by just one
/// thread but will use multiple threads for computing the checksum, irrespective of the
/// `max_job_count` value.  It is commonly thought that using just one I/O thread gives fastest
/// reads on spinning hard drives. I have not found that to be true when reading small files.
///
/// The semaphore's capacity should equal `max_job_count`.
///
/// The max number of threads used for checksumming large files is by default the number of logical
/// CPU cores Rayon detects (via `num_cpus`). This can be overridden by setting the
/// `RAYON_NUM_THREADS` environment variable. At least two threads must be used since one thread is
/// needed to read files and another is needed to compute checksums.
///
/// Since this function may may spawn a background thread, `Ok(())` may be returned but an error
/// may still be produced later. Callers should use [`print_error`] to handle errors, since that
/// matches what will be invoked internally.
///
/// # Example
///
/// ```no_run
/// use std::{error::Error, path::PathBuf, sync::Arc};
/// use multi_semaphore::Semaphore;
/// use rayon::Scope;
/// # fn print_error(path: &PathBuf, err: Box<dyn Error>) { }
/// # fn do_checksum( path: PathBuf, max_job_count: usize, io_lock: Arc<Semaphore>, use_mmap: bool, s: &Scope,) -> Result<(), Box<dyn Error>> { todo!() }
///
/// let paths = vec![PathBuf::from("song.mp3"), PathBuf::from("todo.txt")];
/// let max_job_count = 32;
/// let io_lock = Arc::new(Semaphore::new(max_job_count as isize));
/// rayon::scope(|s| {
///     for path in paths {
///         if let Err(err) =
///             do_checksum(path.clone(), max_job_count, Arc::clone(&io_lock), false, s)
///         {
///             print_error(&path, err);
///         }
///     }
/// });
/// ```
pub fn do_checksum(
    path: PathBuf,
    max_job_count: usize,
    io_lock: Arc<Semaphore>,
    use_mmap: bool,
    s: &Scope,
) -> Result<()> {
    if let Some(str) = path.to_str() {
        if str == "-" {
            let checksum = b3sum_large(Input::Stream(Box::new(std::io::stdin())), false);
            print_checksum(&path, checksum);
            return Ok(());
        }
    }

    // Be careful with locking: we can't use guards because
    // the lifetime restrictions are not worth the effort.
    io_lock.acquire(); // this operation will need at least one I/O resource
    let mut file = File::open(&path)?;
    let filesize = file.metadata()?.len();
    if filesize > 131_072 {
        // Wait for all other I/O to be finished, and take all the I/O resources.
        // Because concurrent reads of large files hurts performance on SSDs/HDDs.
        io_lock.acquire_many(max_job_count as isize - 1);
        let checksum = b3sum_large(Input::File(file), use_mmap);
        io_lock.release_many(max_job_count as isize);
        print_checksum(&path, checksum);
    } else {
        s.spawn(move |_| {
            let checksum = b3sum_small(&mut file);
            io_lock.release();
            print_checksum(&path, checksum);
        });
    };

    Ok(())
}

/// Compute a checksum of a small file or stdin by reading it all into memory.
pub(crate) fn b3sum_small(file: &mut dyn Read) -> Result<[u8; OUT_LEN]> {
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    return Ok(blake3::hash(&buf).try_into().unwrap());
}

/// Compute a multi-threaded checksum of a large file by buffering it or memory mapping it.
pub(crate) fn b3sum_large(file: Input, use_mmap: bool) -> Result<[u8; OUT_LEN]> {
    let mut hasher = blake3::Hasher::new();
    match file {
        Input::File(file) if use_mmap => {
            // Iterating over chunks is faster than computing the whole buffer,
            // even on SSDs. On spinning discs, mmap is still slower than normal file reads.
            // TODO: the buffer size may need to be tuned based on the number of threads.
            let buf = unsafe { Mmap::map(&file) }?;
            for slice in buf.chunks(4_194_304) {
                hasher.update_with_join::<blake3::join::RayonJoin>(&slice);
            }
        }
        _ => {
            let mut file: Box<dyn Read> = match file {
                Input::File(file) => Box::new(file),
                Input::Stream(read) => read,
            };
            let mut buf = vec![0u8; 2_097_152];
            loop {
                let bytes_read = file.read(&mut buf)?;
                if bytes_read == 0 {
                    break;
                }
                hasher.update_with_join::<blake3::join::RayonJoin>(&buf[0..bytes_read]);
            }
        }
    }
    Ok(hasher.finalize().try_into().unwrap())
}

/// Print a checksum or an error that was encountered.
pub(crate) fn print_checksum(path: &PathBuf, result: Result<[u8; OUT_LEN]>) {
    match result {
        Ok(checksum) => {
            println!("{}  {}", Checksum(checksum), path.display());
        }
        Err(err) => print_error(path, err),
    }
}

/// Print an error and the filename that caused it.
pub fn print_error(path: &PathBuf, err: Box<dyn Error>) {
    let binary_name = match std::env::current_exe() {
        Ok(binary_name) => match binary_name.file_name() {
            Some(binary_name) => binary_name.to_string_lossy().to_string(),
            None => binary_name.display().to_string(),
        },
        Err(_) => "".to_owned(),
    };
    eprintln!("{}: {}: {}", binary_name, path.display(), err);
}

#[derive(StructOpt)]
#[structopt()]
pub(crate) struct Options {
    #[structopt(
        default_value = "-",
        help = "Files to get the checksum of. When '-' is given, \
            calculate the checksum of standard input."
    )]
    pub paths: Vec<PathBuf>,

    #[structopt(
        long,
        // The author of rigrep says mmap causes random SIGSEGV or SIGBUS
        // when files are changed during reading. Unlikely.
        help = "Use mmap. This gives better performance on SSDs. It is possible that the program will crash \
            if a file is modified while being read.",
    )]
    pub mmap: bool,

    // Note: this number that was found to have good performance in testing
    // on hard drives and SSDs.
    #[structopt(
        short,
        long,
        default_value = "16",
        help = "The number of concurrent reads to allow. Regardless of this value, \
            checksums of large files will still be computed one at a time with multithreading."
    )]
    pub job_count: usize,
}

pub(crate) enum Input {
    File(File),            // Files are preferred, as the API is more flexible.
    Stream(Box<dyn Read>), // If it's not a file, it should still be readable.
}

pub(crate) struct Checksum(pub [u8; OUT_LEN]);
impl std::fmt::Display for Checksum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0.iter() {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

#[test]
fn b3_test_bytes() -> Result<()> {
    assert_eq!(
        "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24",
        &format!(
            "{}",
            Checksum(b3sum_small(&mut std::io::Cursor::new(b"hello world"))?)
        )
    );
    Ok(())
}

#[test]
fn b3_test_bytes_empty() -> Result<()> {
    assert_eq!(
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        &format!("{}", Checksum(b3sum_small(&mut std::io::Cursor::new(b""))?))
    );
    Ok(())
}

#[cfg(test)]
struct TempFileGuard {
    pub filename: PathBuf,
}
#[cfg(test)]
impl Drop for TempFileGuard {
    fn drop(&mut self) {
        std::fs::remove_file(&self.filename).unwrap();
    }
}

#[cfg(test)]
fn make_temp_file(contents: &[u8]) -> (File, PathBuf, TempFileGuard) {
    use std::io::Write;
    use std::sync::atomic::AtomicU32;

    static FILE_ID: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "b3sum-{}-{}",
        std::process::id(),
        FILE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut file = File::create(&path).unwrap();
    file.write_all(contents).unwrap();
    file.flush().unwrap();
    drop(file);
    (
        File::open(&path).unwrap(),
        path.to_owned(),
        TempFileGuard { filename: path },
    )
}

#[test]
fn b3_test_file_small() -> Result<()> {
    let (mut file, _path, _guard) = make_temp_file(b"hello world");
    assert_eq!(
        "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24",
        &format!("{}", Checksum(b3sum_small(&mut file)?))
    );
    Ok(())
}

#[test]
fn b3_test_file_large() -> Result<()> {
    let (file, _path, _guard) = make_temp_file(b"hello world");
    assert_eq!(
        "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24",
        &format!("{}", Checksum(b3sum_large(Input::File(file), false)?))
    );
    Ok(())
}

#[test]
fn b3_test_file_large_2() -> Result<()> {
    let (file, _path, _guard) = make_temp_file(&vec![0u8; 20_971_520]);
    assert_eq!(
        "bea89379ccc6ac7c6e1a2924643665501a7a6427877f2c6764f9813f8c9330b4",
        &format!("{}", Checksum(b3sum_large(Input::File(file), false)?))
    );
    Ok(())
}

#[test]
fn b3_test_file_mmap() -> Result<()> {
    let (file, _path, _guard) = make_temp_file(b"hello world");
    assert_eq!(
        "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24",
        &format!("{}", Checksum(b3sum_large(Input::File(file), true)?))
    );
    Ok(())
}

#[test]
fn b3_test_file_mmap_2() -> Result<()> {
    let (file, _path, _guard) = make_temp_file(&vec![0u8; 20_971_520]);
    assert_eq!(
        "bea89379ccc6ac7c6e1a2924643665501a7a6427877f2c6764f9813f8c9330b4",
        &format!("{}", Checksum(b3sum_large(Input::File(file), true)?))
    );
    Ok(())
}

#[test]
/// Test that several files can be opened, some of which may be opened in background threads.
fn b3_test_file_no_error_1() -> Result<()> {
    let buffers: Vec<Vec<u8>> = vec![
        vec![0u8; 20_971_520],
        (b"hello, world").iter().map(|b| *b).collect(),
        (b"hello, world").iter().map(|b| *b).collect(),
        (b"hello, world").iter().map(|b| *b).collect(),
        vec![],
        vec![0u8; 900_00],
        vec![0u8; 900_00],
        vec![0u8; 900_00],
        vec![0u8; 20_971_520],
        vec![0u8; 900_00],
        vec![0u8; 900_00],
    ];
    let temp_files: Vec<_> = buffers
        .into_iter()
        .map(|buf| make_temp_file(&buf))
        // Close open handles
        .map(|(file, path, guard)| {
            drop(file);
            (path, guard)
        })
        .collect();

    let semaphore = Arc::new(Semaphore::new(16));
    rayon::scope(|s| {
        for (path, _) in &temp_files {
            assert!(do_checksum(path.to_owned(), 16, Arc::clone(&semaphore), true, s).is_ok());
        }
        for (path, _) in &temp_files {
            assert!(do_checksum(path.to_owned(), 16, Arc::clone(&semaphore), false, s).is_ok());
        }
    });
    Ok(())
}
