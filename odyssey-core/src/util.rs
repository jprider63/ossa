
use rand::{RngCore, rngs::OsRng};

use crate::store::Nonce;

/// Generate a random nonce.
pub(crate) fn generate_nonce() -> Nonce {
    let mut nonce = [0; 32];
    OsRng.fill_bytes(&mut nonce);
    nonce
}

#[test]
fn test() {
    let n1 = generate_nonce();
    let n2 = generate_nonce();
    // println!("Nonce: {:?}", n1);
    // println!("Nonce: {:?}", n2);
    assert!(n1 != n2);
}
