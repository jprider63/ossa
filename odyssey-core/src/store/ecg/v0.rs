
#[derive(Clone)]
pub struct HeaderId<Hash>(Hash);

// TODO: Move this to the right location.
/// An ECG header.
#[derive(Clone)]
pub struct Header<Hash> {
    /// The header id of our parent in the ECG graph.
    parent: HeaderId<Hash>,
    /// The hash of (batched) operations in this header.
    operation_hashes: Vec<Hash>,
}

