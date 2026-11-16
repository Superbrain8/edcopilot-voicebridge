//! A tiny profile writer for the tests. It writes synthetic records in the
//! tested format. It holds no data from a real profile: the phrases and the
//! tokens of the tests are invented.
//!
//! The unit tests of `core` use it directly. The other crates get it with
//! the feature `test-support`, together with [`super::fixture`].

use std::cell::Cell;
use std::io::Write;

use super::record::{tested_default, TESTED_FIELD_COUNTS};

#[cfg(test)]
pub use peak::peak_request;

/// The request-file path of a token write.
pub const REQUEST_PATH: &str = r"{TXT:PythonLibPath}\{TXT:RequestFile}";

/// The counting allocator. It is only in the test binary of `core`: the
/// feature `test-support` does not put it into the tests of other crates.
#[cfg(test)]
mod peak {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The system allocator. It also records the largest single request, so
    /// that a test can show that no read length from the file sets the size
    /// of an allocation.
    pub struct PeakAlloc;

    static PEAK_REQUEST: AtomicUsize = AtomicUsize::new(0);

    unsafe impl GlobalAlloc for PeakAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            PEAK_REQUEST.fetch_max(layout.size(), Ordering::Relaxed);
            // SAFETY: the caller keeps the contract of `GlobalAlloc::alloc`.
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: the caller keeps the contract of `GlobalAlloc::dealloc`.
            unsafe { System.dealloc(ptr, layout) }
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            PEAK_REQUEST.fetch_max(new_size, Ordering::Relaxed);
            // SAFETY: the caller keeps the contract of `GlobalAlloc::realloc`.
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }

    #[global_allocator]
    static ALLOCATOR: PeakAlloc = PeakAlloc;

    /// The largest single allocation request since the test binary started.
    pub fn peak_request() -> usize {
        PEAK_REQUEST.load(Ordering::Relaxed)
    }
}

thread_local! {
    /// Fields that the writer adds at the end of each record, to imitate a
    /// newer format.
    static EXTRA_FIELDS: Cell<usize> = const { Cell::new(0) };
}

/// Runs `f` with `extra` more fields at the end of each record that it
/// writes.
pub fn with_extra_fields<T>(extra: usize, f: impl FnOnce() -> T) -> T {
    EXTRA_FIELDS.with(|e| e.set(extra));
    let out = f();
    EXTRA_FIELDS.with(|e| e.set(0));
    out
}

fn extra_fields() -> usize {
    EXTRA_FIELDS.with(Cell::get)
}

/// One record from its fields. The header count is `fields.len() - 1`.
pub fn record(fields: Vec<Vec<u8>>) -> Vec<u8> {
    let n = fields.len() - 1;
    let table_end = 8 + 4 * (n + 1);
    let body: usize = fields.iter().map(Vec::len).sum();
    let size = table_end + body;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&(n as i32).to_le_bytes());
    let mut at = table_end;
    for f in &fields {
        out.extend_from_slice(&(at as i32).to_le_bytes());
        at += f.len();
    }
    for f in fields {
        out.extend_from_slice(&f);
    }
    out
}

pub fn str_field(text: &str) -> Vec<u8> {
    let mut out = (text.len() as i32).to_le_bytes().to_vec();
    out.extend_from_slice(text.as_bytes());
    out
}

pub fn null_str() -> Vec<u8> {
    (-1i32).to_le_bytes().to_vec()
}

pub fn int_field(v: i32) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

pub fn opt_str(text: Option<&str>) -> Vec<u8> {
    text.map_or_else(null_str, str_field)
}

pub fn list(records: Vec<Vec<u8>>) -> Vec<u8> {
    let mut out = (records.len() as i32).to_le_bytes().to_vec();
    for r in records {
        out.extend_from_slice(&r);
    }
    out
}

/// `n + 1` null string fields, and the extra fields of [`with_extra_fields`].
fn blank_fields(n: u32) -> Vec<Vec<u8>> {
    (0..=n as usize + extra_fields())
        .map(|_| null_str())
        .collect()
}

/// The fields of an action of the given type, with neutral values.
pub fn action_fields(kind: i32) -> Vec<Vec<u8>> {
    let mut f = blank_fields(TESTED_FIELD_COUNTS.action);
    f[0] = vec![0; 16];
    f[1] = int_field(kind);
    // The values of the tested profile in the fields that the builder does
    // not read, and the tested lengths.
    for (k, field) in f
        .iter_mut()
        .enumerate()
        .take(TESTED_FIELD_COUNTS.action as usize + 1)
        .skip(2)
    {
        *field = tested_default(k).to_vec();
    }
    f[28] = vec![0];
    f
}

fn action(kind: i32, f5: Option<&str>, f6: Option<&str>) -> Vec<u8> {
    let mut f = action_fields(kind);
    f[5] = opt_str(f5);
    f[6] = opt_str(f6);
    record(f)
}

/// An action of any type with neutral fields.
pub fn plain_action(kind: i32) -> Vec<u8> {
    record(action_fields(kind))
}

/// The action with field 28 = 1 (probably "disabled").
pub fn flagged(action: Vec<u8>) -> Vec<u8> {
    set_field(action, 28, vec![1])
}

/// The action record with field `k` replaced.
pub fn set_field(action: Vec<u8>, k: usize, value: Vec<u8>) -> Vec<u8> {
    let mut f = fields_of(&action);
    f[k] = value;
    record(f)
}

/// The fields of a record that [`record`] wrote.
pub fn fields_of(rec: &[u8]) -> Vec<Vec<u8>> {
    let i32_at = |at: usize| i32::from_le_bytes(rec[at..at + 4].try_into().unwrap()) as usize;
    let size = i32_at(0);
    let n = i32_at(4);
    let mut offs: Vec<usize> = (0..=n).map(|k| i32_at(8 + 4 * k)).collect();
    offs.push(size);
    offs.windows(2).map(|w| rec[w[0]..w[1]].to_vec()).collect()
}

pub fn write(token: &str) -> Vec<u8> {
    action(44, Some(token), Some(REQUEST_PATH))
}

pub fn write_other_file(text: &str) -> Vec<u8> {
    action(44, Some(text), Some(r"{TXT:PythonLibPath}\other.txt"))
}

pub fn set_text(name: &str, value: &str) -> Vec<u8> {
    action(21, Some(name), Some(value))
}

/// A set-text action that reads the variable from a file (source 4).
pub fn set_text_from_file(name: &str, path: &str) -> Vec<u8> {
    let mut f = action_fields(21);
    f[5] = str_field(name);
    f[8] = str_field(path);
    f[13] = int_field(4);
    record(f)
}

/// A set-text action that copies another variable (source 1).
pub fn set_text_copy(name: &str, from: &str) -> Vec<u8> {
    let mut f = action_fields(21);
    f[5] = str_field(name);
    f[7] = str_field(from);
    f[13] = int_field(1);
    record(f)
}

/// A set-text action with a literal value and a text replacement.
pub fn set_text_replace(name: &str, value: &str, find: &str) -> Vec<u8> {
    let mut f = action_fields(21);
    f[5] = str_field(name);
    f[6] = str_field(value);
    f[12] = int_field(1);
    f[14] = str_field(find);
    record(f)
}

pub fn sound() -> Vec<u8> {
    action(14, Some("beep.wav"), None)
}

pub fn say(text: &str) -> Vec<u8> {
    action(13, Some(text), None)
}

pub fn key_press() -> Vec<u8> {
    action(0, None, None)
}

pub fn condition(variable: &str, value: &str, operator: i32) -> Vec<u8> {
    let mut f = action_fields(19);
    f[6] = str_field(value);
    // Field 12 is 1 on each `{PREFIX}` block of the tested profile.
    f[12] = int_field(i32::from(variable == "{PREFIX}"));
    f[18] = str_field(variable);
    f[19] = int_field(operator);
    // A text compare.
    f[23] = int_field(1);
    record(f)
}

pub fn else_branch() -> Vec<u8> {
    action(29, None, None)
}

pub fn end_condition() -> Vec<u8> {
    action(20, None, None)
}

/// Execute another command by its id (field 5, as GUID text).
pub fn exec_by_id(id: [u8; 16]) -> Vec<u8> {
    action(16, Some(&guid_text(id)), None)
}

/// Execute another command by its name (field 6). Field 5 is the zero id.
pub fn exec_by_name(name: &str) -> Vec<u8> {
    set_field(
        action(16, Some(&guid_text([0; 16])), Some(name)),
        3,
        1f64.to_le_bytes().to_vec(),
    )
}

/// Execute another command with any id, name, and mode (field 3: 1.0 is
/// "by name", 0.0 is "by id").
pub fn exec_with(id: [u8; 16], name: Option<&str>, by_name: bool) -> Vec<u8> {
    let mode = if by_name { 1f64 } else { 0f64 };
    set_field(
        action(16, Some(&guid_text(id)), name),
        3,
        mode.to_le_bytes().to_vec(),
    )
}

/// An inline code action (type 71) with the code text in field 5.
pub fn inline_code(code: &str) -> Vec<u8> {
    action(71, Some(code), Some("System.dll"))
}

/// The SHA-256 of `bytes`, in lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The GUID text of a 16-byte id in the .NET byte order.
pub fn guid_text(id: [u8; 16]) -> String {
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let rev = |b: &[u8]| hex(&b.iter().rev().copied().collect::<Vec<_>>());
    format!(
        "{}-{}-{}-{}-{}",
        rev(&id[0..4]),
        rev(&id[4..6]),
        rev(&id[6..8]),
        hex(&id[8..10]),
        hex(&id[10..16])
    )
}

/// The ten actions of the system-slot parse: read the slot file `file`
/// and take its first entry into `SystemName`.
pub fn slot_parse(file: u32) -> Vec<Vec<u8>> {
    // `fields`: the fields 10, 11 and 19, as the tested profile has them.
    let set_int = |name: &str, source: i32, value: Option<&str>, fields: [i32; 3]| {
        let mut f = action_fields(37);
        f[10] = int_field(fields[0]);
        f[11] = int_field(fields[1]);
        f[19] = int_field(fields[2]);
        f[13] = int_field(source);
        f[14] = str_field(name);
        f[15] = opt_str(value);
        record(f)
    };
    vec![
        set_text_from_file(
            "SystemFromFile",
            &format!(r"{{TXT:DynamicVarLib}}\EDCoPilotSystem{file}.txt"),
        ),
        condition("{TXT:SystemFromFile}", ";", 6),
        set_int(
            "Startpos",
            6,
            Some(r#"{TXTPOS:"[":SystemFromFile:0}"#),
            [0, 0, 0],
        ),
        set_int("Startpos", 8, None, [1, 1, 0]),
        set_int(
            "SysLength",
            6,
            Some(r#"{TXTPOS:";":SystemFromFile:0}"#),
            [0, 0, 0],
        ),
        set_int("SysLength", 9, Some("{INT:Startpos}"), [0, 1, 1]),
        set_text(
            "SystemName",
            "{TXTSUBSTR:SystemFromFile:Startpos:SysLength}",
        ),
        else_branch(),
        set_text("SystemName", "{TXT:SystemFromFile}"),
        end_condition(),
    ]
}

/// The fields of a command.
pub fn command_fields(
    phrase: &str,
    actions: Vec<Vec<u8>>,
    command_type: i32,
    group: Option<&str>,
) -> Vec<Vec<u8>> {
    let mut f = blank_fields(TESTED_FIELD_COUNTS.command);
    f[0] = vec![7; 16];
    f[1] = str_field(phrase);
    f[2] = list(actions);
    // The tested lengths and the common values. Field 4 = 1: probably
    // "enabled". Field 15 = 1, 1: probably "when I say".
    f[3] = vec![1];
    f[4] = vec![1];
    f[8] = vec![0];
    for k in [9, 10, 11, 12, 13, 18] {
        f[k] = int_field(0);
    }
    f[14] = vec![1, 1];
    f[15] = vec![1, 1];
    f[16] = vec![0];
    f[17] = int_field(2);
    // The other tested command settings.
    for (k, value) in super::record::COMMAND_SETTINGS {
        f[k] = value.to_vec();
    }
    f[7] = str_field("cat");
    f[19] = int_field(command_type);
    f[20] = opt_str(group);
    f
}

pub fn command(
    phrase: &str,
    actions: Vec<Vec<u8>>,
    command_type: i32,
    group: Option<&str>,
) -> Vec<u8> {
    record(command_fields(phrase, actions, command_type, group))
}

/// A full command with the given id.
pub fn command_with_id(id: [u8; 16], phrase: &str, actions: Vec<Vec<u8>>) -> Vec<u8> {
    let mut f = command_fields(phrase, actions, 0, None);
    f[0] = id.to_vec();
    record(f)
}

/// `n` full commands with invented fixed tokens. The builder needs at least
/// 200 records.
pub fn filler(n: usize) -> Vec<Vec<u8>> {
    (0..n)
        .map(|k| {
            command(
                &format!("filler command {k}"),
                vec![write(&format!("Filler:{k}"))],
                0,
                None,
            )
        })
        .collect()
}

/// An inflated profile from command records.
pub fn profile_raw(commands: Vec<Vec<u8>>) -> Vec<u8> {
    let mut f = blank_fields(TESTED_FIELD_COUNTS.profile);
    f[0] = vec![0; 16];
    f[1] = str_field("test");
    f[2] = list(commands);
    // The tested lengths of the profile fields with one byte or two.
    for k in [3, 5, 11, 12, 14, 15, 21, 22, 25] {
        f[k] = vec![0];
    }
    f[23] = vec![1];
    f[6] = vec![0, 0];
    f[16] = vec![0, 0];
    f[32] = str_field("1.16");
    record(f)
}

pub fn profile(commands: Vec<Vec<u8>>) -> Vec<u8> {
    profile_raw(commands)
}

pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

/// The deflated profile of `commands` and 200 filler commands.
pub fn vap(commands: Vec<Vec<u8>>) -> Vec<u8> {
    let mut all = commands;
    all.extend(filler(200));
    deflate(&profile(all))
}
