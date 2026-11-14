#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| rustern_msgs::fuzz_support::shard_resp(data));
