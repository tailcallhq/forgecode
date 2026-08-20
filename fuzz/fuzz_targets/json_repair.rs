#![no_main]

use forge_json_repair::json_repair;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let text = String::from_utf8_lossy(input);
    let _ = json_repair::<serde_json::Value>(&text);
});
