#[derive(Clone)]
pub struct HeaderId<Hash>(Hash);

// TODO: Move this to the right location.
/// An ECG header.
#[derive(Clone)]
pub struct Header<Hash> {
    /// The header id of our parent in the ECG graph.
    parent: HeaderId<Hash>,
    /// The hash of (batched) operations in this header.
    /// The maximum number of operations is 256.
    operation_hashes: Vec<Hash>,
}

// TODO: OperationID's are header ids and index (HeaderId, u8)
