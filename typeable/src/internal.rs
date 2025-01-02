/// Internal helper functions. Do not rely on this API. It is unstable.
pub use sha2::{Digest, Sha256};

pub fn helper_counter(h: &mut Sha256, n: usize) {
    let c: u8 = n.try_into().expect(&format!("Too many inputs: {}", n));
    h.update([c]);
}

pub fn helper_string(h: &mut Sha256, s: &'static str) {
    // Check if there are invalid characters
    let is_valid_string = s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    assert!(is_valid_string, "Invalid string: {}", s);

    helper_string_non_ascii(h, s);
}

pub fn helper_string_non_ascii(h: &mut Sha256, s: &'static str) {
    helper_counter(h, s.len());
    h.update(s);
}

pub fn helper_u8(h: &mut Sha256, n: u8) {
    h.update([n]);
}

pub fn helper_usize(h: &mut Sha256, n: usize) {
    h.update(n.to_be_bytes());
}

pub fn helper_type_constructor(h: &mut Sha256, constructor: &'static str) {
    helper_string(h, constructor);
}

pub fn helper_type_args(h: &mut Sha256, type_args_count: u8) {
    h.update([type_args_count]);
}
