// Copyright 2021 Daniel Zwell.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::sync::Arc;

use multi_semaphore::Semaphore;
use structopt::*;

mod lib;
use lib::*;

fn main() {
    let opts: Options = Options::from_args();
    let paths = opts.paths;
    let max_job_count = opts.job_count;
    let io_lock = Arc::new(Semaphore::new(max_job_count as isize));
    let use_mmap = opts.mmap;
    rayon::scope(|s| {
        for path in paths {
            if let Err(err) = do_checksum(
                path.clone(),
                max_job_count,
                Arc::clone(&io_lock),
                use_mmap,
                s,
            ) {
                print_error(&path, err);
            }
        }
    });
}
