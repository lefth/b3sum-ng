# b3sum-ng

[![Build Status](https://travis-ci.com/lefth/b3sum-ng.svg?branch=master)](https://travis-ci.com/lefth/b3sum-ng)

[Documentation (master)](https://lefth.github.io/b3sum-ng)

A version of b3sum that is fast on hard drives and SSDs, for large and small files.

This implementation aims to be a similar speed as the official rust [b3sum](https://docs.rs/crate/b3sum/)
project for all workloads, but much faster on spinning hard drives. Small files are read in parallel
and checksummed with one thread each, while large files are checksummed alone with multiple threads.
`--mmap` is currently not a default option, since that option causes the Blake3 library to read in a way
that is slow for large files on spinning drives. Many thanks to the [BLAKE3 team](https://github.com/BLAKE3-team/BLAKE3)
for creating this hash and the blake3 library this program uses.

### USAGE:

    b3sum-ng [FLAGS] [OPTIONS] [paths]...

### FLAGS:
    -h, --help       Prints help information
        --mmap       Use mmap. This gives better performance on SSDs. It is possible that the program
                     will crash if a file is modified while being read.
    -V, --version    Prints version information

### OPTIONS:
    -j, --job-count <job-count>    The number of concurrent reads to allow. Regardless of this value,
                                   checksums of large files will still be computed one at a time
                                   with multithreading. [default: 16]

### ARGS:
    <paths>...    Files to get the checksum of. When '-' is given, calculate the checksum of standard input.
                  [default: -]

## Examples
```
$ b3sum-ng .gitignore ./README.md
b955c237862d9e3192a835650e1a6cfb42085727e98f92f5586ec0279cd17e11  .gitignore
5032a97cf7c563415f02749243bd4289ec3560067db01c94a0782414b0a40530  ./README.md

$ tar -cf - . | b3sum-ng -
2b72789e52fc96405fe121b3904498b9f07c601d372807edcc0ae3f2e50a88c3  -
```

## Installation

```
cargo install --git https://github.com/lefth/b3sum-ng
```

## Todo

The `--check` command is not yet implemented.
